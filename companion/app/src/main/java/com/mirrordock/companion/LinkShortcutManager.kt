package com.mirrordock.companion

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
 * 行为约定（ToDesk 同款惯例，需在系统设置里由用户主动确认后才生效）：
 * - 通过 [ShortcutManager.requestPinShortcut] 请求固定，由系统弹确认框；
 *   未经用户同意不会静默出现在桌面。
 * - 点按快捷方式 → [LinkShortcutActivity]：一个透明 Activity，立刻启动常驻
 *   连接然后 finish，用户看到的是"点了就连上了"，不落在多余界面上。
 * - 没有已配对电脑时不提供固定请求——钉一个点开没反应或报错的图标没有意义。
 */
object LinkShortcutManager {

    /** 快捷方式的固定 id。改动会让系统认为是新图标（旧的会残留，需要用户删除）。 */
    const val SHORTCUT_ID = "mirrordock_connect_last"

    /** 无已配对电脑时展示的提示文案。 */
    const val RATIONALE_NO_PAIRED = "先扫码配对至少一台电脑，才能创建桌面快捷方式。"

    /**
     * 请求固定「连接上次配对的电脑」快捷方式。
     *
     * @return 结果文案：已固定 / 系统不支持 / 用户取消 / 无已配对电脑（供界面如实展示）
     */
    fun requestPin(activity: android.app.Activity): String {
        val computer = ComputerStore.active(activity)
            ?: return RATIONALE_NO_PAIRED
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O_MR1) {
            return "系统版本较低，暂不支持桌面快捷方式。"
        }
        val manager = activity.getSystemService(ShortcutManager::class.java)
            ?: return "系统不支持桌面快捷方式。"

        val shortcut = ShortcutInfo.Builder(SHORTCUT_ID)
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

        val result = runCatching { manager.requestPinShortcut(shortcut, null) }.getOrNull()
            ?: return "系统拒绝了固定请求。"
        if (result.isSuccess) {
            return "已创建桌面快捷方式：点一下就连上 ${computer.displayName()}"
        }
        // isLongLived 是 API 30 才有的属性，minSdk 26 —— 直接读会在 Android 8/9
        // 上抛 NoSuchMethodError。按版本取，不让低版本用户崩在这行。
        val longLived = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            runCatching { result.isLongLived }.getOrDefault(false)
        } else {
            false
        }
        return if (longLived) "桌面快捷方式已固定。" else "已取消。"
    }

    /** 供快捷方式目标 Activity 查询目标电脑（避免与 UI 层重复读 prefs）。 */
    fun activeComputer(context: Context): PairedComputer? = ComputerStore.active(context)
}
