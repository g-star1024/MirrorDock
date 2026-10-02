package com.mirrordock.companion

import android.app.Notification
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.provider.Settings
import java.util.concurrent.Executors

/**
 * 通知镜像一期（X10-66）：把手机收到的新通知（标题/内容/来源应用）经常驻
 * 连接转发给已配对的电脑，电脑端「工具」页实时展示。二期再做快捷回复。
 *
 * 隐私与边界（与 AGENTS 红线对齐）：
 * - 前置开关 `notify_mirror` **默认关**；主界面可开、可随时关，系统设置里
 *   也随时可撤回「读取通知」授权（可见且可撤销）。
 * - 只转发给台账里那台已配对电脑（TLS + 双向互信），不经任何云端。
 * - 通知内容**不写 logcat**（日志只记包名与发送结果），不落盘。
 * - 常驻通知（下载进度、音乐播放条）与锁屏受保护内容不转发；仅记录
 *   内容非空的「正常通知」。
 * - 只在常驻连接在线（LinkState.lineSender 就绪）时实时转发；离线期间的
 *   通知**不排队、不补发**——补发会在重连瞬间倾倒历史通知，噪音大于价值。
 */
class NotificationMirrorService : NotificationListenerService() {

    companion object {
        const val PREFS = "paired_computers"
        const val KEY_NOTIFY_MIRROR = "notify_mirror"

        /** 单字段截断上限：两个字段最坏情况 ~6 KiB UTF-8，低于协议 8 KiB 行上限
         * （超限会让桌面端整条会话判「消息超长」断开——宁裁剪不可断连）。 */
        private const val MAX_FIELD_CHARS = 1000

        /**
         * 本机是否已取得「读取通知」授权（系统设置手动授予，无运行时弹窗）。
         * 读 Settings.Secure 的授权清单，与应用内开关无关。
         */
        fun isListenerGranted(context: Context): Boolean {
            val flat = Settings.Secure.getString(
                context.contentResolver,
                "enabled_notification_listeners",
            ) ?: return false
            return flat.split(":").any { entry ->
                ComponentName.unflattenFromString(entry)?.packageName == context.packageName
            }
        }

        /** 打开系统的通知读取授权页（授权后 onResume 刷新状态即可）。 */
        fun openListenerSettings(context: Context) {
            runCatching {
                context.startActivity(
                    Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
        }
    }

    /** 串行发送：与心跳/UI 推送共用 PrintWriter（println 按行同步，行间不黏）。 */
    private val sendExecutor = Executors.newSingleThreadExecutor()

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        if (sbn == null) return
        val prefs = getSharedPreferences(PREFS, MODE_PRIVATE)
        if (!prefs.getBoolean(KEY_NOTIFY_MIRROR, false)) return
        // 自己的通知不转发（常驻连接状态条、录制提醒），避免自反馈。
        if (sbn.packageName == packageName) return
        // 常驻通知（进行中的下载/播放）是状态不是消息，不转发。
        if (sbn.isOngoing) return

        val extras = sbn.notification.extras
        val title = extras.getCharSequence(Notification.EXTRA_TITLE)?.toString().orEmpty()
        val text = extras.getCharSequence(Notification.EXTRA_TEXT)?.toString().orEmpty()
            .ifBlank { extras.getCharSequence(Notification.EXTRA_BIG_TEXT)?.toString().orEmpty() }
        // 无内容的通知（纯进度/静默通道）转发出去没有信息量，跳过。
        if (title.isBlank() && text.isBlank()) return

        val app = runCatching {
            packageManager.getApplicationLabel(
                packageManager.getApplicationInfo(sbn.packageName, 0),
            ).toString()
        }.getOrDefault(sbn.packageName)

        val payload = org.json.JSONObject().apply {
            put("type", "notification")
            put("pkg", sbn.packageName)
            put("app", app.take(MAX_FIELD_CHARS))
            put("title", title.take(MAX_FIELD_CHARS))
            put("text", text.take(MAX_FIELD_CHARS))
            put("posted", sbn.postTime)
        }
        val sender = LinkState.lineSender ?: return
        sendExecutor.execute {
            val ok = runCatching { sender(payload.toString()) }.getOrDefault(false)
            // 日志只含包名与结果，绝不含通知标题/内容。
            android.util.Log.i("NotifyMirror", "forward pkg=${sbn.packageName} sent=$ok")
        }
    }
}
