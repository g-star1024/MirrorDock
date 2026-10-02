package com.mirrordock.companion

import android.app.Activity
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.core.content.ContextCompat

/**
 * 桌面快捷方式的落点（X10-73）：无界面的透明 Activity，作用只有两个——
 * 启动常驻连接、然后立刻 finish。
 *
 * 为什么需要它：常驻服务是后台前台服务（dataSync 类型），从桌面图标直接启动
 * 在 Android 8+ 有限制；经一个可见的 Activity 启动是系统允许的路径。做成
 * 透明无主题 Activity，用户看到的就是"点了图标就连上了"，不闪一个界面。
 *
 * 如实反馈：没配对电脑时不静默失败，而是给一条 Toast 说明要先扫码。
 */
class LinkShortcutActivity : Activity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val computer = LinkShortcutManager.activeComputer(this)
        if (computer == null) {
            // 快捷方式存在但凭据已被解除（例如用户在设置里解除后没删图标）。
            // 如实说明并把用户送到主界面重新配对，而不是无声退出。
            Toast.makeText(this, LinkShortcutManager.RATIONALE_NO_PAIRED, Toast.LENGTH_LONG).show()
            startActivity(
                Intent(this, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK),
            )
            finish()
            return
        }
        val intent = Intent(this, PersistentConnectionService::class.java)
            .setAction(PersistentConnectionService.ACTION_START)
        ContextCompat.startForegroundService(this, intent)
        Toast.makeText(this, "正在连接 ${computer.displayName()}…", Toast.LENGTH_SHORT).show()
        finish()
    }
}
