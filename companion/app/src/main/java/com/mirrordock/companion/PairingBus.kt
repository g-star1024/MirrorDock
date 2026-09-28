package com.mirrordock.companion

/**
 * 进程内共享当前配对连接的极简总线：
 * CaptureService 的统计消息经它送到桌面端；未连接时静默丢弃（只打日志）。
 */
object PairingBus {
    @Volatile private var client: PairingClient? = null

    fun attach(c: PairingClient) {
        client = c
    }

    fun detach(c: PairingClient) {
        if (client === c) client = null
    }

    fun dispatch(line: String) {
        val c = client
        if (c == null) {
            android.util.Log.i("PairingBus", "no client, dropped: $line")
        } else {
            Thread { c.sendLine(line) }.start()
        }
    }
}
