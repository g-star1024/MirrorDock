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
 * `MDP1|主机1,主机2|端口|一次性配对码|服务器SPKI的SHA-256(小写hex)`
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
            if (parts.size != 5 || parts[0] != "MDP1") return null
            val hosts = parts[1].split(",").map { it.trim() }.filter { it.isNotEmpty() }
            val port = parts[2].toIntOrNull() ?: return null
            if (hosts.isEmpty() || parts[3].length !in 8..64) return null
            if (parts[4].length != 64 || parts[4].any { it !in "0123456789abcdef" }) return null
            return PairingPayload(hosts, port, parts[3], parts[4])
        }
    }
}

/**
 * 同网加密会话客户端（C4-01 POC）。
 *
 * 安全设计：
 * - 不使用系统信任库：唯一的信任锚是二维码携带的 SPKI SHA-256 指纹（出带校验），
 *   同网中间人伪造的证书指纹无法匹配。
 * - token 只经加密通道发送，且一次性。
 */
class PairingClient(private val payload: PairingPayload, private val log: (String) -> Unit) {

    interface Listener {
        fun onWelcome()
        fun onRejected()
        fun onError(message: String)
        fun onDisconnected()
    }

    @Volatile private var socket: SSLSocket? = null
    @Volatile private var readerThread: Thread? = null

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
            listener.onError("无法连接电脑（请确认同一 Wi-Fi 且电脑配对未结束）")
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
            val writer = PrintWriter(tls.outputStream, true)
            writer.println("MDP1 ${payload.token}")
            val reader = BufferedReader(InputStreamReader(tls.inputStream, Charsets.UTF_8))
            val reply = reader.readLine() ?: ""
            if (reply.contains("\"welcome\"")) {
                val model = android.os.Build.MODEL.replace("\"", "").ifEmpty { "android" }
                writer.println("{\"type\":\"device_hello\",\"model\":\"$model\"}")
                listener.onWelcome()
            } else {
                listener.onRejected()
                tls.close()
                return
            }
            // 持续读取服务端消息（POC 里服务端基本不发；断开即回调）。
            // 读超时只服务于上面的握手回复：会话期必须解除，否则服务端
            // 不主动发消息时，15 秒后 readLine 必然超时误报「已断开」。
            tls.soTimeout = 0
            readerThread = Thread {
                try {
                    while (reader.readLine() != null) { /* POC 不处理服务端主动消息 */ }
                    listener.onDisconnected()
                } catch (_: Exception) {
                    listener.onDisconnected()
                }
            }.also { it.isDaemon = true; it.start() }
        } catch (e: Exception) {
            listener.onError("会话异常：${e.message}")
            runCatching { tls.close() }
        }
    }

    /** 发送一行 JSON（阻塞，须在后台线程调用）。 */
    fun sendLine(line: String): Boolean = runCatching {
        val s = socket ?: return false
        PrintWriter(s.outputStream, true).println(line)
        true
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
