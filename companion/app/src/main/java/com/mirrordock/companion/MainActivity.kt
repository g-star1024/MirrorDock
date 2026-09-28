package com.mirrordock.companion

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat

/**
 * 主界面：配对入口 + 捕获入口 + 日志展示（C4-01/C4-02 POC）。
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val REQUEST_SCAN = 41
        private const val REQUEST_CAMERA = 42
        private const val REQUEST_CAPTURE = 43
        private const val REQUEST_NOTIFICATION = 44
    }

    private lateinit var statusText: TextView
    private lateinit var logText: TextView
    private lateinit var manualInput: EditText

    private val logLines = StringBuilder()
    private var client: PairingClient? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        statusText = findViewById(R.id.status_text)
        logText = findViewById(R.id.log_text)
        manualInput = findViewById(R.id.manual_input)

        findViewById<Button>(R.id.button_scan).setOnClickListener {
            if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
                == PackageManager.PERMISSION_GRANTED
            ) {
                openScanner()
            } else {
                requestPermissions(arrayOf(Manifest.permission.CAMERA), REQUEST_CAMERA)
            }
        }
        findViewById<Button>(R.id.button_manual).setOnClickListener {
            manualInput.visibility =
                if (manualInput.visibility == View.GONE) View.VISIBLE else View.GONE
        }
        manualInput.setOnEditorActionListener { _, _, _ ->
            startWithPayload(manualInput.text.toString())
            true
        }
        findViewById<Button>(R.id.button_capture).setOnClickListener { startCaptureFlow() }

        log("伴侣 App 已启动。")
    }

    override fun onRequestPermissionsResult(
        requestCode: Int, permissions: Array<out String>, grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        when (requestCode) {
            REQUEST_CAMERA -> if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) openScanner()
            REQUEST_NOTIFICATION -> if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) requestCaptureConsent()
        }
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        when (requestCode) {
            REQUEST_SCAN -> {
                val raw = data?.getStringExtra(ScanActivity.EXTRA_PAYLOAD) ?: return
                if (resultCode == RESULT_OK) startWithPayload(raw)
            }
            REQUEST_CAPTURE -> {
                // 把系统授权结果交给前台服务；拒绝也如实记录进同意状态机。
                val intent = Intent(this, CaptureService::class.java).apply {
                    putExtra(CaptureService.EXTRA_RESULT_CODE, resultCode)
                    putExtra(CaptureService.EXTRA_RESULT_DATA, data)
                }
                ContextCompat.startForegroundService(this, intent)
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        client?.let { PairingBus.detach(it) }
        client?.close()
        client = null
    }

    private fun openScanner() {
        @Suppress("DEPRECATION")
        startActivityForResult(Intent(this, ScanActivity::class.java), REQUEST_SCAN)
    }

    private fun startCaptureFlow() {
        if (android.os.Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_NOTIFICATION)
            return
        }
        requestCaptureConsent()
    }

    private fun requestCaptureConsent() {
        val manager = getSystemService(android.media.projection.MediaProjectionManager::class.java)
        @Suppress("DEPRECATION")
        startActivityForResult(manager.createScreenCaptureIntent(), REQUEST_CAPTURE)
    }

    private fun startWithPayload(raw: String) {
        val payload = PairingPayload.parse(raw)
        if (payload == null) {
            Toast.makeText(this, "配对信息格式不正确", Toast.LENGTH_SHORT).show()
            return
        }
        connect(payload)
    }

    private fun connect(payload: PairingPayload) {
        client?.close()
        statusText.text = "正在连接 ${payload.hosts.first()}:${payload.port} …"
        log("使用一次性配对码 ${payload.token}（不会保存）")
        val c = PairingClient(payload) { line -> runOnUiThread { log(line) } }
        client = c
        PairingBus.attach(c)
        Thread {
            c.connect(object : PairingClient.Listener {
                override fun onWelcome() = runOnUiThread {
                    statusText.text = "已与电脑建立加密会话。"
                    log("配对成功（加密会话已建立）")
                }

                override fun onRejected() = runOnUiThread {
                    statusText.text = "配对码被拒绝。请回到电脑重新生成配对。"
                    c.close()
                }

                override fun onError(message: String) = runOnUiThread {
                    statusText.text = "连接失败：$message"
                    c.close()
                }

                override fun onDisconnected() = runOnUiThread {
                    statusText.text = "会话已断开。"
                }
            })
        }.start()
    }

    private fun log(line: String) {
        logLines.appendLine(line)
        logText.text = logLines.toString()
    }
}
