package com.mirrordock.companion

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.graphics.Bitmap
import android.graphics.PixelFormat
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.Image
import android.media.ImageReader
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.Handler
import android.os.HandlerThread
import android.os.IBinder
import android.util.DisplayMetrics

/**
 * C4-02 捕获 POC：
 * - 同意/撤销状态机：NotRequested → Pending → Granted/Denied → (Revoked)
 *   —— 状态只存在于本服务运行期，不落盘。
 * - 前台服务（mediaProjection 类型，API 34 硬性要求）。
 * - ImageReader 逐帧计数，首帧压缩为 JPEG 作为样本上行（服务端只统计不落盘）。
 * - 音频能力探测：API 29+ 尝试创建播放捕获 AudioRecord，成败如实上报。
 *
 * POC 边界：不做持续视频上行，只统计帧数并回报——持续流化属后续阶段。
 */
class CaptureService : Service() {

    companion object {
        const val EXTRA_RESULT_CODE = "result_code"
        const val EXTRA_RESULT_DATA = "result_data"
        private const val CHANNEL_ID = "capture"
        private const val NOTIFICATION_ID = 11
    }

    /** 同意状态机（C4-02 核心交付之一）。 */
    enum class ConsentState { NOT_REQUESTED, PENDING, GRANTED, DENIED, REVOKED }

    @Volatile private var consentState = ConsentState.NOT_REQUESTED
    private var projection: MediaProjection? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var imageReader: ImageReader? = null
    private var workerThread: HandlerThread? = null
    private var frameCount = 0L
    private var sampleSent = false
    private var audioSupported: Boolean? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Int.MIN_VALUE) ?: Int.MIN_VALUE
        val resultData: Intent? = if (Build.VERSION.SDK_INT >= 33) {
            intent?.getParcelableExtra(EXTRA_RESULT_DATA, Intent::class.java)
        } else {
            @Suppress("DEPRECATION")
            intent?.getParcelableExtra(EXTRA_RESULT_DATA)
        }
        if (resultCode == Int.MIN_VALUE || resultData == null) {
            stopSelf()
            return START_NOT_STICKY
        }

        consentState = if (resultCode == android.app.Activity.RESULT_OK) ConsentState.PENDING else ConsentState.DENIED
        startInForeground()
        if (consentState == ConsentState.DENIED) {
            // 用户在系统对话框点了拒绝：如实记录并结束，不重试、不引导绕过。
            android.util.Log.i("CaptureService", "consent=DENIED")
            stopSelf()
            return START_NOT_STICKY
        }

        val manager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        try {
            projection = manager.getMediaProjection(resultCode, resultData!!).also { mp ->
                consentState = ConsentState.GRANTED
                mp.registerCallback(object : MediaProjection.Callback() {
                    override fun onStop() {
                        // 用户从系统面板撤销共享：状态机进入 REVOKED 并立即停采。
                        consentState = ConsentState.REVOKED
                        stopCapture()
                        stopSelf()
                    }
                }, Handler(workerThread!!.looper))
            }
        } catch (e: Exception) {
            android.util.Log.w("CaptureService", "getMediaProjection failed: ${e.message}")
            consentState = ConsentState.DENIED
            stopSelf()
            return START_NOT_STICKY
        }

        audioSupported = probeAudioCapture()
        startFrameCounting()
        return START_NOT_STICKY
    }

    private fun startInForeground() {
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= 26) {
            manager.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "屏幕捕获", NotificationManager.IMPORTANCE_LOW)
            )
        }
        val notification: Notification = if (Build.VERSION.SDK_INT >= 26) {
            Notification.Builder(this, CHANNEL_ID)
                .setContentTitle("MirrorDock 伴侣")
                .setContentText("正在与电脑共享屏幕内容")
                .setSmallIcon(R.mipmap.ic_launcher)
                .setOngoing(true)
                .build()
        } else {
            Notification.Builder(this)
                .setContentTitle("MirrorDock 伴侣")
                .setContentText("正在与电脑共享屏幕内容")
                .setSmallIcon(R.mipmap.ic_launcher)
                .build()
        }
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(NOTIFICATION_ID, notification)
        }
    }

    /** 音频能力探测：播放捕获仅 API 29+；以 AudioRecord 初始化成败为准。 */
    private fun probeAudioCapture(): Boolean {
        if (Build.VERSION.SDK_INT < 29) return false
        val mp = projection ?: return false
        return try {
            val config = android.media.AudioPlaybackCaptureConfiguration.Builder(mp)
                .addMatchingUsage(android.media.AudioAttributes.USAGE_MEDIA)
                .build()
            val format = android.media.AudioFormat.Builder()
                .setEncoding(android.media.AudioFormat.ENCODING_PCM_16BIT)
                .setSampleRate(44100)
                .setChannelMask(android.media.AudioFormat.CHANNEL_IN_MONO)
                .build()
            val minBuf = android.media.AudioRecord.getMinBufferSize(
                44100, android.media.AudioFormat.CHANNEL_IN_MONO, android.media.AudioFormat.ENCODING_PCM_16BIT
            )
            val record = android.media.AudioRecord.Builder()
                .setAudioFormat(format)
                .setBufferSizeInBytes(minBuf * 2)
                .setAudioPlaybackCaptureConfig(config)
                .build()
            val ok = record.state == android.media.AudioRecord.STATE_INITIALIZED
            record.release()
            ok
        } catch (e: Exception) {
            android.util.Log.i("CaptureService", "audio probe failed: ${e.message}")
            false
        }
    }

    private fun startFrameCounting() {
        val mp = projection ?: return
        val metrics: DisplayMetrics = resources.displayMetrics
        val width = minOf(metrics.widthPixels, 1280)
        val height = width * metrics.heightPixels / metrics.widthPixels
        workerThread = HandlerThread("capture").also { it.start() }
        val handler = Handler(workerThread!!.looper)
        imageReader = ImageReader.newInstance(width, height, PixelFormat.RGBA_8888, 3).also { reader ->
            reader.setOnImageAvailableListener({ r: ImageReader ->
                var image: Image? = null
                try {
                    image = r.acquireLatestImage()
                    if (image != null) {
                        frameCount++
                        if (!sampleSent) {
                            sampleSent = true
                            val jpeg = toJpeg(image, 40)
                            sendStats(jpeg.size)
                        }
                    }
                } catch (_: Exception) {
                } finally {
                    image?.close()
                }
            }, handler)
        }
        virtualDisplay = mp.createVirtualDisplay(
            "mirrordock-poc", width, height, metrics.densityDpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, imageReader!!.surface, null, handler,
        )
        // 没有伴侣连接时也允许计数（POC 独立运行），统计在日志可见。
        android.util.Log.i("CaptureService", "capture started (consent=GRANTED, audio=$audioSupported)")
    }

    private fun toJpeg(image: Image, quality: Int): ByteArray {
        val rowPad = image.planes[0].getRowStride() - image.width * 4
        val bitmap = Bitmap.createBitmap(image.width + rowPad / 4, image.height, Bitmap.Config.ARGB_8888)
        image.planes[0].buffer.rewind()
        bitmap.copyPixelsFromBuffer(image.planes[0].buffer)
        val cropped = Bitmap.createBitmap(bitmap, 0, 0, image.width, image.height)
        val out = java.io.ByteArrayOutputStream()
        cropped.compress(Bitmap.CompressFormat.JPEG, quality, out)
        return out.toByteArray()
    }

    private fun sendStats(sampleBytes: Int) {
        val line = "{\"type\":\"capture_stats\",\"frames\":$frameCount," +
            "\"audio_supported\":${audioSupported == true},\"sample_jpeg_bytes\":$sampleBytes}"
        // POC：发送经由 MainActivity 建立的连接；未连接时仅记录。
        PairingBus.dispatch(line)
    }

    private fun stopCapture() {
        runCatching { virtualDisplay?.release() }
        runCatching { imageReader?.close() }
        runCatching { projection?.stop() }
        virtualDisplay = null
        imageReader = null
        projection = null
        workerThread?.quitSafely()
        workerThread = null
    }

    override fun onDestroy() {
        stopCapture()
        super.onDestroy()
    }
}
