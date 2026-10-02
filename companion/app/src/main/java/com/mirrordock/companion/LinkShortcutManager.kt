package com.mirrordock.companion

import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.os.Build

/**
 * 桌面快捷方式（X10-73）：把「连接上次配对的电脑」钉到手机桌面。
 *
 * 为什么做这个：ToDesk/向日葵的核心交互是"从桌面一步连上常用电脑"，而不是
 * 先开 App 再点按钮。伴侣 App 本身不提供镜像画面，但常驻通道让它成为
 * 「文件互传 + 通知镜像 + 免扫码连电脑」的入口，桌面直达才有实际价值。
 *
 * 行为约定（ToDesk 同款惯例，需在系统弹窗里由用户主动确认后才生效）：
 * - 通过 [ShortcutManager.requestPinShortcut] 请求固定，由系统弹确认框；
 *   未经用户同意不会静默出现在桌面。
 * - 点按快捷方式 → [LinkShortcutActivity]：一个透明 Activity，立刻启动常驻
 *   连接然后 finish，用户看到的是"点了就连上了"，不落在多余界面上。
 * - 没有已配对电脑时不提供固定请求——钉一个点开没反应或报错的图标没有意义。
 *
 * API 注意（2026-10-03 编译实测踩坑，勿改回）：
 * 1. `ShortcutInfo.Builder` **只有 `(Context, String)` 构造器**，没有单参 String
 *    版本。写成 `Builder(SHORTCUT_ID)` 编译不过。
 * 2. `requestPinShortcut` 返回的 `ShortcutManager.RequestPinShortcutResult`
 *    是 **@hide 类型**，公开 SDK 里查不到，因此 `isSuccess` / `isLongLived`
 *    都**不可访问**。只能靠返回值是否为 null 判断：非 null = 用户同意并已固定，
 *    null = 用户取消或系统拒绝。想知道用户到底同不同意，标准做法是给
 *    `requestPinShortcut` 传一个 `IntentSender`，由系统在结果里回调。
 *    这里用最简形式（传 null），并把「已请求固定」与「已确认固定」在文案上
 *    区分开，不谎报结果。
 */
object LinkShortcutManager {

    /** 快捷方式的固定 id。改动会让系统认为是新图标（旧的会残留，需要用户删除）。 */
    const val SHORTCUT_ID = "mirrordock_connect_last"

    /** 无已配对电脑时展示的提示文案。 */
    const val RATIONALE_NO_PAIRED = "先扫码配对至少一台电脑，才能创建桌面快捷方式。"

    /**
     * 请求固定「连接上次配对的电脑」快捷方式。
     *
     * @return 结果文案，供界面如实展示（已提交系统确认 / 系统不支持 / 无已配对电脑）
     */
    fun requestPin(activity: Activity): String {
        val computer = ComputerStore.active(activity)
            ?: return RATIONALE_NO_PAIRED
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O_MR1) {
            return "系统版本较低，暂不支持桌面快捷方式。"
        }
        val manager = activity.getSystemService(ShortcutManager::class.java)
            ?: return "系统不支持桌面快捷方式。"
        if (!manager.isRequestPinShortcutSupported) {
            // 部分厂商 ROM（如部分国产桌面）禁用了固定请求，如实说明而不是静默失败。
            return "当前系统桌面不支持固定快捷方式，可从 MirrorDock 图标进入。"
        }

        val shortcut = try {
            ShortcutInfo.Builder(activity, SHORTCUT_ID)
                .setShortLabel(computer.displayName())
                .setLongLabel("连接 ${computer.displayName()}")
                // 用现有 launcher 图标，不新增资源。
                .setIcon(Icon.createWithResource(activity, R.mipmap.ic_launcher))
                .setIntent(
                    Intent(activity, LinkShortcutActivity::class.java)
                        .setAction(Intent.ACTION_VIEW)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK),
                )
                .build()
        } catch (e: Exception) {
            // 图标资源缺失或 Intent 不合法时构造会抛 IllegalArgumentException，
            // 那是我们的 bug，必须如实报出来而不是伪装成"已取消"。
            return "无法创建快捷方式：${e.message ?: e.javaClass.simpleName}"
        }

        // 返回非 null 表示系统已接受请求（用户会在系统弹窗里确认）。
        // 结果对象是 @hide 类型，拿不到 isSuccess —— 不谎报"已固定"。
        val accepted = runCatching { manager.requestPinShortcut(shortcut, null) }.getOrNull()
        return when {
            accepted != null ->
                "已向系统请求固定快捷方式：在弹出的确认框点「添加」后，桌面会出现「${computer.displayName()}」，点一下就连上。"
            else -> "已取消添加桌面快捷方式。"
        }
    }

    /** 供快捷方式目标 Activity 查询目标电脑（避免与 UI 层重复读 prefs）。 */
    fun activeComputer(context: Context): PairedComputer? = ComputerStore.active(context)
}
