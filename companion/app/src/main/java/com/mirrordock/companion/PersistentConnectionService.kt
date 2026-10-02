package com.mirrordock.companion

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import java.util.concurrent.CopyOnWriteArrayList

/**
 * 常驻连接状态（M4-2）：前台服务与 MainActivity 之间的共享状态。
 * 服务在后台线程更新，界面 onResume 注册监听、onPause 注销。
 */
enum class LinkPhase { IDLE, CONNECTING, CONNECTED, RETRYING }

object LinkState {
    @Volatile var phase: LinkPhase = LinkPhase.IDLE
    @Volatile var detail: String = ""

    /**
     * 当前活跃会话的下行发送桥（X10-60）：UI 层（如发送区有新文件、崩溃上报）
     * 经它把一行 JSON 推给电脑；无连接时为 null，调用方静默忽略即可——
     * 伴侣通道是附加信息通道，不构成错误。
     */
    @Volatile var lineSender: ((String) -> Boolean)? = null

    private val listeners = CopyOnWriteArrayList<() -> Unit>()

    fun update(newPhase: LinkPhase, newDetail: String) {
        phase = newPhase
        detail = newDetail
        listeners.forEach { runCatching { it() } }
    }

    fun addListener(listener: () -> Unit) = listeners.add(listener)
    fun removeListener(listener: () -> Unit) = listeners.remove(listener)
}

/**
 * M4-2 常驻通道（伴侣端）：凭本地保存的互信凭据免扫码直连已配对的电脑。
 *
 * - 前台服务（dataSync 类型，API 34 硬性要求）：应用退到后台/熄屏后仍保持连接。
 * - 心跳：每 15 秒发 `{"type":"ping"}`，写入失败即判断链（服务端 300 秒静默
 *   也会判死，两端口径一致）。
 * - 断连退避重试：5s → 10s → 20s → … 封顶 5 分钟；互信会话建立成功即归零。
 * - 重连目标：last_host 的主机 + 本地保存的 resident_port；**没有学到端口时
 *   回退默认端口 47017**（X10-64 定案 B：把「在电脑端打开常驻通道后可免扫码
 *   重连」的文案承诺变成真实能力——电脑固定端口未被占用时直接连上；被占用
 *   回退随机端口的场景仍需重新扫码）。
 * - 状态通知（M4-3）：前台常驻通知随「连接中/已连接/重试中」更新；
 *   连接建立/断开与电脑录制开始/结束另发可划走的事件通知（可关，默认开）。
 * - 互信凭据只来自本地 SharedPreferences（paired_computers），无凭据时服务
 *   直接结束并如实提示——不会要求重新扫码（那是主界面的职责）。
 */
class PersistentConnectionService : Service() {

    companion object {
        const val ACTION_START = "com.mirrordock.companion.action.START_LINK"
        const val ACTION_STOP = "com.mirrordock.companion.action.STOP_LINK"
        private const val CHANNEL_LINK = "link"
        private const val CHANNEL_EVENTS = "link_events"
        private const val NOTIF_LINK = 21
        private const val NOTIF_EVENT = 22
        private const val PREFS = "paired_computers"

        /** 通知开关（M4-3 可关）：存 SharedPreferences，主界面设置区读写。 */
        const val KEY_NOTIFY_EVENTS = "notify_events"

        private const val HEARTBEAT_MS = 15_000L
        private const val BACKOFF_BASE_MS = 5_000L
        private const val BACKOFF_MAX_MS = 300_000L

        /** 桌面常驻通道默认端口（X10-64 定案 B）：本地没学到 resident_port 时
         * 回退直连这个端口——与桌面端 RESIDENT_DEFAULT_PORT 一致。 */
        const val RESIDENT_DEFAULT_PORT = 47017
    }

    @Volatile private var running = false
    private var worker: Thread? = null
    /** startInForeground 的 LinkState 监听只注册一次（服务实例可能被多次 onStartCommand）。 */
    @Volatile private var listenerBound = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                // 经 startForegroundService 拉起的服务必须先 startForeground，
                // 否则系统 5 秒判定超时直接崩溃（ForegroundServiceDidNotStartInTime，
                // 2026-10-02 真机实锤：服务未运行时点「解除这台电脑」必崩）。
                startInForeground()
                running = false
                LinkState.lineSender = null
                LinkState.update(LinkPhase.IDLE, "已断开")
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return START_NOT_STICKY
            }
            else -> {
                // ACTION_START 或 START_STICKY 被系统重建时 intent 为 null：
                // 凭本地凭据继续。
                startInForeground()
                if (!running) {
                    running = true
                    worker = Thread { runLoop() }.also { it.start() }
                }
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        LinkState.update(LinkPhase.IDLE, "已断开")
        super.onDestroy()
    }

    // -- 主循环 ---------------------------------------------------------------

    private fun runLoop() {
        var attempt = 0
        while (running) {
            val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
            val pairingId = prefs.getString("pairing_id", null)?.takeIf { it.isNotBlank() }
            val fingerprint = prefs.getString("desktop_fingerprint", null)?.takeIf { it.isNotBlank() }
            val lastHost = prefs.getString("last_host", null)?.takeIf { it.isNotBlank() }
            val residentPort = prefs.getInt("resident_port", -1).takeIf { it > 0 }
                ?: RESIDENT_DEFAULT_PORT // X10-64B：没学到端口时回退默认端口直连。
            val host = lastHost?.substringBeforeLast(':')?.takeIf { it.isNotBlank() }
            if (pairingId == null || fingerprint == null || host == null) {
                LinkState.update(LinkPhase.IDLE, "还没有可直连的电脑，请先扫码配对并让电脑开启常驻通道")
                notifyEvent(getString(R.string.link_event_no_credential), false)
                stopForeground(STOP_FOREGROUND_REMOVE)
                stopSelf()
                return
            }

            val endpoint = "$host:$residentPort"
            val payload = PairingPayload(
                hosts = listOf(host),
                port = residentPort,
                token = "",
                fingerprint = fingerprint,
            )
            val client = PairingClient(payload, reconnectId = pairingId) { line ->
                android.util.Log.i("PersistentLink", line)
            }
            // 会话存活标记：worker 线程写、心跳线程读，用原子量保证可见性。
            val sessionAlive = java.util.concurrent.atomic.AtomicBoolean(false)
            var heartbeat: Thread? = null
            LinkState.update(
                if (attempt == 0) LinkPhase.CONNECTING else LinkPhase.RETRYING,
                "正在连接 $endpoint …",
            )

            client.connect(object : PairingClient.Listener {
                override fun onWelcome() {
                    LinkState.update(LinkPhase.CONNECTING, "正在与电脑完成身份互验…")
                }

                override fun onPaired(pid: String, newResidentPort: Int?) {
                    sessionAlive.set(true)
                    attempt = 0
                    // 下行发送桥：UI 层经 LinkState.lineSender 推 JSON 给电脑。
                    LinkState.lineSender = { line -> client.sendLine(line) }
                    android.util.Log.i("PersistentLink", "onPaired: lineSender attached (pid=$pid)")
                    // 桌面端常驻端口可能变化（回退随机端口后重开）：学到即更新。
                    if (newResidentPort != null && newResidentPort != residentPort) {
                        prefs.edit().putInt("resident_port", newResidentPort).apply()
                    }
                    LinkState.update(LinkPhase.CONNECTED, "已连接 $endpoint")
                    notifyEvent(getString(R.string.link_event_connected), true)
                    heartbeat = Thread {
                        while (running && sessionAlive.get()) {
                            try {
                                Thread.sleep(HEARTBEAT_MS)
                            } catch (_: InterruptedException) {
                                return@Thread
                            }
                            if (!running || !sessionAlive.get()) return@Thread
                            // 写失败即断链：主动断开触发外层重连。
                            if (!client.sendLine("{\"type\":\"ping\"}")) {
                                client.close()
                                return@Thread
                            }
                        }
                    }.also { it.isDaemon = true; it.start() }
                }

                override fun onRejected() {
                    // 台账里被移除/身份不符：如实告知，不再退避重试（重试也不会过）。
                    LinkState.lineSender = null
                    notifyEvent(getString(R.string.link_event_rejected), true)
                    LinkState.update(LinkPhase.IDLE, "电脑拒绝了这次连接（可能已解除互信）")
                }

                override fun onError(message: String) {
                    LinkState.update(LinkPhase.RETRYING, "连接失败：$message")
                }

                override fun onDisconnected() {
                    sessionAlive.set(false)
                    LinkState.lineSender = null
                    android.util.Log.w("PersistentLink", "onDisconnected: lineSender cleared")
                    if (running) {
                        notifyEvent(getString(R.string.link_event_disconnected), true)
                        LinkState.update(LinkPhase.RETRYING, "会话断开，稍后自动重连")
                    }
                }

                override fun onServerMessage(json: org.json.JSONObject) {
                    when (json.optString("type")) {
                        // M4-3：桌面录制状态经常驻通道下发。
                        "recording" -> notifyEvent(
                            getString(
                                if (json.optBoolean("active", false)) R.string.link_event_recording_start
                                else R.string.link_event_recording_stop,
                            ),
                            true,
                        )
                        // 常驻端口更新（电脑端重开常驻通道后经活跃会话推送）。
                        "resident_port" -> json.optInt("port", -1).takeIf { it > 0 }?.let { port ->
                            prefs.edit().putInt("resident_port", port).apply()
                        }
                        // 快捷回复（X10-69）：桌面端下发的通知回复文本，交给
                        // 监听服务填进原通知的 RemoteInput 动作。字段缺失直接忽略。
                        "notification_reply" -> {
                            val key = json.optString("key")
                            val text = json.optString("text")
                            if (key.isNotBlank() && text.isNotBlank()) {
                                NotificationMirrorService.handleReply(key, text)
                            }
                        }
                        // pong 及未知类型静默忽略。
                    }
                }
            })

            heartbeat?.interrupt()
            LinkState.lineSender = null
            client.close()
            if (!running) return
            if (LinkState.phase == LinkPhase.IDLE) {
                // 被拒绝（onRejected）：重试也不会过，保持 IDLE 静置直到用户处理；
                // 服务保持前台，避免「被拒了还在后台傻试」刷通知。
                sleepQuietly(BACKOFF_MAX_MS)
                continue
            }
            attempt++
            val delay = minOf(BACKOFF_BASE_MS shl (attempt - 1).coerceAtMost(8), BACKOFF_MAX_MS)
            LinkState.update(LinkPhase.RETRYING, "将在 ${delay / 1000} 秒后重试（第 $attempt 次）")
            sleepQuietly(delay)
        }
    }

    private fun sleepQuietly(ms: Long) {
        try {
            Thread.sleep(ms)
        } catch (_: InterruptedException) {
        }
    }

    // -- 通知 -----------------------------------------------------------------

    private fun startInForeground() {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_LINK, "与电脑的常驻连接", NotificationManager.IMPORTANCE_LOW)
            )
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_EVENTS, "连接与录制提醒", NotificationManager.IMPORTANCE_DEFAULT)
            )
        }
        val notification = buildLinkNotification("正在启动…")
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIF_LINK, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } else {
            startForeground(NOTIF_LINK, notification)
        }
        // LinkState 变化时同步刷新前台通知文案（只注册一次，服务生命周期内有效）。
        if (!listenerBound) {
            listenerBound = true
            LinkState.addListener {
                if (!running) return@addListener
                val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
                nm.notify(NOTIF_LINK, buildLinkNotification(LinkState.detail.ifEmpty { "运行中" }))
            }
        }
    }

    /** 前台常驻通知：内容随 LinkState 更新。 */
    private fun buildLinkNotification(text: String): Notification {
        val contentIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(this, CHANNEL_LINK)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        return builder
            .setContentTitle("MirrorDock 伴侣")
            .setContentText(text)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setOngoing(true)
            .setContentIntent(contentIntent)
            .build()
    }

    /** 一次性事件通知（连接/断开/录制）；用户可在主界面关掉（M4-3 可关）。 */
    private fun notifyEvent(text: String, respectToggle: Boolean) {
        val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        if (respectToggle && !prefs.getBoolean(KEY_NOTIFY_EVENTS, true)) return
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        val builder = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(this, CHANNEL_EVENTS)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }
        val contentIntent = PendingIntent.getActivity(
            this, 0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        manager.notify(
            NOTIF_EVENT,
            builder
                .setContentTitle("MirrorDock 伴侣")
                .setContentText(text)
                .setSmallIcon(R.mipmap.ic_launcher)
                .setAutoCancel(true)
                .setContentIntent(contentIntent)
                .build(),
        )
    }
}
