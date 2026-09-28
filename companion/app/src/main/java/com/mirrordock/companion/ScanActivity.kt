package com.mirrordock.companion

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.Executors

/**
 * 相机扫码页：CameraX 分析流 + zxing 解码（离线，无第三方识别服务）。
 *
 * 加固记录（修复「安装后扫码直接闪退」）：
 * - 相机权限在本页内用 ActivityResult API 请求；此前无权限直接静默 finish，
 *   在部分 ROM（尤其 MIUI）上权限弹窗与生命周期竞争会造成异常退出。
 * - 任何初始化失败都不再静默消失：错误写进屏幕上的提示条，用户能看到原因，
 *   也能配合主界面的 CrashGuard 堆栈回显定位。
 */
class ScanActivity : AppCompatActivity() {

    companion object {
        const val EXTRA_PAYLOAD = "payload"
    }

    private val executor = Executors.newSingleThreadExecutor()
    private val reader = MultiFormatReader().apply {
        setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE)))
    }
    private var delivered = false
    private lateinit var previewView: PreviewView
    private lateinit var hintText: TextView

    private val requestPermission =
        registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) {
                bindCamera()
            } else {
                showError("扫码需要相机权限。请在系统设置中允许后重试。")
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        previewView = PreviewView(this)
        hintText = TextView(this).apply {
            setPadding(48, 32, 48, 32)
            setTextAppearance(android.R.style.TextAppearance_Medium)
            visibility = View.GONE
        }
        val root = FrameLayout(this)
        root.addView(
            previewView,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT,
            ),
        )
        root.addView(
            hintText,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.WRAP_CONTENT,
                FrameLayout.LayoutParams.WRAP_CONTENT,
                Gravity.CENTER,
            ),
        )
        setContentView(root)
        supportActionBar?.title = getString(R.string.scan_hint)

        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            == PackageManager.PERMISSION_GRANTED
        ) {
            bindCamera()
        } else {
            requestPermission.launch(Manifest.permission.CAMERA)
        }
    }

    private fun bindCamera() {
        val future = ProcessCameraProvider.getInstance(this)
        future.addListener({
            try {
                val provider = future.get()
                val preview = Preview.Builder().build().also {
                    it.setSurfaceProvider(previewView.surfaceProvider)
                }
                val analysis = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                analysis.setAnalyzer(executor, ::analyze)
                provider.unbindAll()
                provider.bindToLifecycle(this, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
            } catch (e: Exception) {
                // 相机不可用（被占用、无摄像头、ROM 限制）如实告知，而不是黑屏或退出。
                showError("相机启动失败：${e.localizedMessage ?: e.javaClass.simpleName}")
            }
        }, ContextCompat.getMainExecutor(this))
    }

    private fun showError(message: String) {
        previewView.visibility = View.GONE
        hintText.visibility = View.VISIBLE
        hintText.text = message
    }

    private fun analyze(image: ImageProxy) {
        if (delivered) {
            image.close()
            return
        }
        try {
            val plane = image.planes[0]
            val buffer = plane.buffer
            val bytes = ByteArray(buffer.remaining())
            buffer.get(bytes)
            val source = PlanarYUVLuminanceSource(
                bytes, plane.getRowStride(), image.height, 0, 0, image.width, image.height, false,
            )
            val result = try {
                reader.decode(BinaryBitmap(HybridBinarizer(source)))
            } catch (_: Exception) {
                null
            } finally {
                reader.reset()
            }
            val text = result?.text
            if (text != null && PairingPayload.parse(text) != null) {
                delivered = true
                setResult(RESULT_OK, Intent().putExtra(EXTRA_PAYLOAD, text))
                runOnUiThread { finish() }
            }
        } catch (_: Exception) {
            // 单帧解码失败直接跳过。
        } finally {
            image.close()
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        executor.shutdown()
    }
}
