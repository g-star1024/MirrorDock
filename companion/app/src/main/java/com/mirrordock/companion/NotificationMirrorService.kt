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
 * 连接转发给已配对的电脑，电脑端「工具」页实时展示。二期（X10-69）：带
 * RemoteInput 回复动作的通知在桌面端可快捷回复，回复文本经 `notification_reply`
 * 下行填进原通知的回复动作。
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

        /** 当前存活的监听服务实例（系统绑定式生命周期：onCreate 登记销毁清除）。 */
        @Volatile
        private var activeInstance: NotificationMirrorService? = null

        /**
         * 快捷回复入口（X10-69）：常驻连接读到 `notification_reply` 下行后调用。
         * 回复文本只填进原通知动作的 RemoteInput，不接触通知 Intent 的其他部分。
         */
        @JvmStatic
        fun handleReply(key: String, text: String) {
            val service = activeInstance ?: run {
                android.util.Log.w("NotifyMirror", "reply dropped: listener not active")
                return
            }
            service.handleReplyInternal(key, text)
        }

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

    override fun onCreate() {
        super.onCreate()
        activeInstance = this
    }

    override fun onDestroy() {
        activeInstance = null
        super.onDestroy()
    }

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

        // X10-69 快捷回复：key 供桌面端回指这条通知；replyable 标记通知是否
        // 带 RemoteInput 回复动作（只有这类通知才在桌面端显示回复框）。
        val replyable = sbn.notification.actions?.any { action ->
            !action.remoteInputs.isNullOrEmpty()
        } == true

        val payload = org.json.JSONObject().apply {
            put("type", "notification")
            put("pkg", sbn.packageName)
            put("app", app.take(MAX_FIELD_CHARS))
            put("title", title.take(MAX_FIELD_CHARS))
            put("text", text.take(MAX_FIELD_CHARS))
            put("posted", sbn.postTime)
            put("key", sbn.key)
            put("replyable", replyable)
        }
        val sender = LinkState.lineSender ?: return
        sendExecutor.execute {
            val ok = runCatching { sender(payload.toString()) }.getOrDefault(false)
            // 日志只含包名与结果，绝不含通知标题/内容。
            android.util.Log.i("NotifyMirror", "forward pkg=${sbn.packageName} sent=$ok")
        }
    }

    /**
     * 把回复文本填进原通知的 RemoteInput 动作并发送（X10-69）。
     *
     * 安全边界（与 android-intent-security 约束对齐）：
     * - 只处理「当前仍活跃」的通知：key 必须与 activeNotifications 精确匹配，
     *   已撤回/过期通知无法被回指，防止对失效目标的误操作。
     * - 只向 PendingIntent 填充 RemoteInput 结果（addResultsToIntent），
     *   不注入任何额外 extras——fillIn Intent 里除了回复文本什么都没有。
     * - 通知的动作 PendingIntent 由发布通知的应用创建，执行权限归属对方应用，
     *   这里只触发它原本就允许的「直接回复」路径（与 Wear/Auto 同一机制）。
     * - 文本上限与转发一致（MAX_FIELD_CHARS），日志只记包名与结果。
     */
    private fun handleReplyInternal(key: String, text: String) {
        val sbn = activeNotifications?.firstOrNull { it.key == key } ?: run {
            android.util.Log.i("NotifyMirror", "reply dropped: notification gone")
            return
        }
        val action = sbn.notification.actions?.firstOrNull { action ->
            !action.remoteInputs.isNullOrEmpty()
        } ?: run {
            android.util.Log.i("NotifyMirror", "reply dropped: no remote input pkg=${sbn.packageName}")
            return
        }
        val fillIn = Intent()
        val results = android.os.Bundle()
        val capped = text.take(MAX_FIELD_CHARS)
        for (remote in action.remoteInputs!!) {
            results.putCharSequence(remote.resultKey, capped)
        }
        android.app.RemoteInput.addResultsToIntent(action.remoteInputs, fillIn, results)
        val ok = runCatching {
            action.actionIntent.send(this, android.app.Activity.RESULT_OK, fillIn)
        }.isSuccess
        android.util.Log.i("NotifyMirror", "reply pkg=${sbn.packageName} sent=$ok")
    }
}
