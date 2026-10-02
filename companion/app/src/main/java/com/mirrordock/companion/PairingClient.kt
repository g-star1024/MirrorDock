package com.mirrordock.companion

import java.io.BufferedReader
import java.io.InputStreamReader
import java.io.PrintWriter
import java.net.InetSocketAddress
import java.net.Socket
import java.security.MessageDigest
import javax.net.ssl.SSLContext
import javax.net.ssl.SSLSocket
import javax.net.ssl.TrustManager
import javax.net.ssl.X509TrustManager
import java.security.cert.CertificateException
import java.security.cert.X509Certificate

/**
 * 配对载荷（与桌面端 companion_pairing.rs 的约定一致）：
 * `MDP2|主机1,主机2|端口|一次性配对码|桌面长期身份SPKI的SHA-256(小写hex)`
 *
 * MDP2（M4-1）：指纹对应桌面**长期身份**（不是一次性证书）——首配后保存在
 * 本地，后续重连可凭它继续锁定服务器身份，无需重新扫码。
 */
data class PairingPayload(
    val hosts: List<String>,
    val port: Int,
    val token: String,
    val fingerprint: String,
) {
    companion object {
        fun parse(raw: String): PairingPayload? {
            val parts = raw.trim().split("|")
            if (parts.size != 5 || parts[0] != "MDP2") return null
            val hosts = parts[1].split(",").map { it.trim() }.filter { it.isNotEmpty() }
            val port = parts[2].toIntOrNull() ?: return null
            if (hosts.isEmpty() || parts[3].length !in 8..64) return null
            if (parts[4].length != 64 || parts[4].any { it !in "0123456789abcdef" }) return null
            return PairingPayload(hosts, port, parts[3], parts[4])
        }
    }
}

/**
 * 同网加密会话客户端（C4-01 POC → M4-1 持久互信 → M4-2 免扫码重连）。
 *
 * 安全设计：
 * - 不使用系统信任库：唯一的信任锚是二维码携带的 SPKI SHA-256 指纹（出带校验），
 *   同网中间人伪造的证书指纹无法匹配。
 * - token 只经加密通道发送，且一次性。
 *
 * M4-2：`reconnectId` 非空时走免扫码重连——首行发 `MDP2 RECONNECT <pairing_id>`，
 * 跳过 device_hello（桌面端从台账里取本机公钥），直接应答挑战。token 字段忽略。
 * 服务端消息经 [Listener.onServerMessage] 回调（心跳 pong、桌面事件如录制状态）。
 */
class PairingClient(
    private val payload: PairingPayload,
    private val reconnectId: String? = null,
    private val log: (String) -> Unit,
) {

    interface Listener {
        /** TLS 建立且服务端欢迎（互信握手开始）。 */
        fun onWelcome()
        /**
         * 互信握手完成：桌面端已把本机登记进配对台账。
         * `residentPort`：桌面端常驻通道端口（未开启为 null）——免扫码重连
         * 的端口依据，持久化后断线可直连（M4-2）。
         */
        fun onPaired(pairingId: String, residentPort: Int?)
        fun onRejected()
        fun onError(message: String)
        fun onDisconnected()
        /** 会话期服务端消息（已解析 JSON）。默认忽略。 */
        fun onServerMessage(json: org.json.JSONObject) {}
    }

    @Volatile private var socket: SSLSocket? = null
    @Volatile private var readerThread: Thread? = null
    // M4-2：握手与会话期共用同一个 writer；sendLine 凭 checkError 感知断链
    // （PrintWriter 吞 IOException，靠返回值判断会永远「成功」）。
    @Volatile private var writer: PrintWriter? = null

    /** 校验服务器证书 SPKI 指纹的 TrustManager。 */
    private inner class PinnedTrustManager : X509TrustManager {
        override fun checkClientTrusted(chain: Array<X509Certificate>, authType: String) {
            throw CertificateException("伴侣端不做服务端角色")
        }

        override fun checkServerTrusted(chain: Array<X509Certificate>, authType: String) {
            val cert = chain.firstOrNull() ?: throw CertificateException("服务器未提供证书")
            // Java 的 publicKey.encoded 即 X.509 SubjectPublicKeyInfo DER，
            // 与桌面端指纹计算（rcgen public_key_der）同源。
            val digest = MessageDigest.getInstance("SHA-256").digest(cert.publicKey.encoded)
            val hex = digest.joinToString("") { "%02x".format(it) }
            if (hex != payload.fingerprint) {
                throw CertificateException("服务器指纹不匹配（疑似中间人），已中止")
            }
        }

        override fun getAcceptedIssuers(): Array<X509Certificate> = arrayOf()
    }

    /** 逐主机尝试连接并完成握手；成功后回调 onWelcome。阻塞式，须在后台线程调用。 */
    fun connect(listener: Listener) {
        val plain = tryConnectAnyHost() ?: run {
            listener.onError("无法连接电脑（请确认同一 Wi-Fi 且电脑配对/常驻通道已开启）")
            return
        }
        val tls = try {
            val context = SSLContext.getInstance("TLS")
            context.init(null, arrayOf<TrustManager>(PinnedTrustManager()), null)
            (context.socketFactory.createSocket(plain, plain.inetAddress.hostAddress, payload.port, true) as SSLSocket).apply {
                startHandshake()
            }
        } catch (e: Exception) {
            runCatching { plain.close() }
            listener.onError("TLS 握手失败：${e.message}")
            return
        }
        socket = tls
        log("TLS 已建立：${tls.session.protocol} / ${tls.session.cipherSuite}")
        try {
            val out = PrintWriter(tls.outputStream, true).also { writer = it }
            val reader = BufferedReader(InputStreamReader(tls.inputStream, Charsets.UTF_8))

            // 首配发一次性配对码；重连发本地保存的 pairing_id（免扫码，M4-2）。
            val reconnect = reconnectId?.takeIf { it.isNotBlank() }
            if (reconnect != null) {
                out.println("MDP2 RECONNECT $reconnect")
            } else {
                out.println("MDP2 ${payload.token}")
            }

            fun readLineOrFail(what: String): String? {
                val line = reader.readLine()
                if (line == null) listener.onError("电脑没有回复$what")
                return line
            }

            val reply = readLineOrFail("欢迎消息") ?: run { runCatching { tls.close() }; return }
            if (!reply.contains("\"welcome\"")) {
                listener.onRejected()
                tls.close()
                return
            }
            listener.onWelcome()

            if (reconnect == null) {
                // 首配才申报身份：机型 + 公钥（SPKI hex）。重连时桌面端从台账取。
                val model = android.os.Build.MODEL.replace("\"", "").ifEmpty { "android" }
                val pubkeyHex = PairingIdentity.publicKeySpkiHex()
                out.println("{\"type\":\"device_hello\",\"model\":\"$model\",\"pubkey\":\"$pubkeyHex\"}")
            }

            val challengeLine = readLineOrFail("挑战消息") ?: run { runCatching { tls.close() }; return }
            val challenge = runCatching {
                org.json.JSONObject(challengeLine)
            }.getOrNull()
            if (challenge == null || challenge.optString("type") != "challenge") {
                listener.onError("电脑发来的挑战格式不正确")
                runCatching { tls.close() }
                return
            }
            val nonce = challenge.optString("nonce").hexToBytes()
            if (nonce == null || nonce.size !in 16..64) {
                listener.onError("挑战数据格式不正确")
                runCatching { tls.close() }
                return
            }
            val sigHex = PairingIdentity.sign(nonce).toHex()
            out.println("{\"type\":\"challenge_response\",\"sig\":\"$sigHex\"}")

            val verdict = readLineOrFail("配对结果") ?: run { runCatching { tls.close() }; return }
            val verdictJson = runCatching { org.json.JSONObject(verdict) }.getOrNull()
            if (verdictJson == null || verdictJson.optString("type") != "paired_ok") {
                listener.onError(if (reconnect != null) "电脑拒绝了这次免扫码重连（可能已在本机解除互信）" else "电脑没有接受本机的身份证明")
                runCatching { tls.close() }
                return
            }
            val pairingId = verdictJson.optString("pairing_id")
            // resident_port：JSON null → 保存为 null（清掉过期端口）。
            val residentPort = if (verdictJson.isNull("resident_port")) null
            else verdictJson.optInt("resident_port", -1).takeIf { it > 0 }
            listener.onPaired(pairingId, residentPort)
            // 持续读取服务端消息（心跳 pong、桌面事件如录制状态）。断开即回调。
            // 读超时只服务于上面的握手回复：会话期必须解除，否则服务端
            // 不主动发消息时，15 秒后 readLine 必然超时误报「已断开」。
            tls.soTimeout = 0
            readerThread = Thread {
                try {
                    while (true) {
                        val line = reader.readLine() ?: break
                        val json = runCatching { org.json.JSONObject(line) }.getOrNull() ?: continue
                        listener.onServerMessage(json)
                    }
                    listener.onDisconnected()
                } catch (_: Exception) {
                    listener.onDisconnected()
                }
            }.also { it.isDaemon = true; it.start() }
            // X10-62：阻塞到读线程退出（断开/关闭即返回）。connect 的语义是
            // 「返回 = 会话已结束」——之前握手完成即返回，服务端 runLoop 紧接着
            // client.close()，会话建立不到一秒就被自己掐断，只能靠 5 秒重连
            // 循环掩盖（真机验证桩实锤：paired_ok 后对端立刻 EOF）。心跳、
            // 桌面事件下行、files_changed 推送都没有存活窗口。
            readerThread?.join()
        } catch (e: Exception) {
            listener.onError("会话异常：${e.message}")
            runCatching { tls.close() }
        }
    }

    /**
     * 发送一行 JSON（阻塞，须在后台线程调用）。
     * 返回 false = 写入出错或连接已断（PrintWriter 吞异常，必须用 checkError 探测）。
     */
    fun sendLine(line: String): Boolean = runCatching {
        val out = writer ?: return false
        out.println(line)
        !out.checkError()
    }.getOrDefault(false)

    fun close() {
        readerThread?.interrupt()
        runCatching { socket?.close() }
    }

    private fun tryConnectAnyHost(): Socket? {
        for (host in payload.hosts) {
            try {
                log("尝试连接 $host:${payload.port} …")
                val s = Socket()
                s.connect(InetSocketAddress(host, payload.port), 5000)
                s.soTimeout = 15000
                return s
            } catch (e: Exception) {
                log("连接 $host 失败：${e.message}")
            }
        }
        return null
    }
}

/** hex 工具（与桌面端 companion_pairing.rs 的编码约定一致，小写无分隔）。 */
fun String.hexToBytes(): ByteArray? {
    if (length % 2 != 0) return null
    val out = ByteArray(length / 2)
    for (i in out.indices) {
        val byte = substring(i * 2, i * 2 + 2).toIntOrNull(16) ?: return null
        out[i] = byte.toByte()
    }
    return out
}

fun ByteArray.toHex(): String = joinToString("") { "%02x".format(it) }
