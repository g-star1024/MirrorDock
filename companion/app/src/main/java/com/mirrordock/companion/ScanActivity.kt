package com.mirrordock.companion

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
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

/** 相机扫码页：CameraX 分析流 + zxing 解码（离线，无第三方识别服务）。 */
class ScanActivity : AppCompatActivity() {

    companion object {
        const val EXTRA_PAYLOAD = "payload"
    }

    private val executor = Executors.newSingleThreadExecutor()
    private val reader = MultiFormatReader().apply {
        setHints(mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE)))
    }
    private var delivered = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val previewView = PreviewView(this)
        setContentView(previewView)
        supportActionBar?.title = getString(R.string.scan_hint)

        if (ContextCompat.checkSelfPermission(this, Manifest.permission.CAMERA)
            != PackageManager.PERMISSION_GRANTED
        ) {
            finish()
            return
        }

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
            } catch (_: Exception) {
                finish()
            }
        }, ContextCompat.getMainExecutor(this))
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
