package com.mirrordock.companion

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * 主界面：配对入口 + 捕获入口 + 电脑发来的文件 + 日志展示（C4-01/C4-02 POC）。
 */
class MainActivity : AppCompatActivity() {

    companion object {
        private const val REQUEST_SCAN = 41
        private const val REQUEST_CAMERA = 42
        private const val REQUEST_CAPTURE = 43
        private const val REQUEST_NOTIFICATION = 44
        private const val REQUEST_READ_FILES = 45
        /** 上一次崩溃堆栈的落盘文件名（见 CrashGuard）。 */
        const val CRASH_FILE = "last_crash.txt"
    }

    private lateinit var statusText: TextView
    private lateinit var statusDot: View
    private lateinit var logText: TextView
    private lateinit var manualInput: EditText
    private lateinit var crashCard: View
    private lateinit var crashDetail: TextView
    private lateinit var logScroller: View
    private lateinit var logToggle: Button
    private lateinit var fileList: LinearLayout
    private lateinit var fileEmpty: View
    private lateinit var fileEmptyText: TextView
    private lateinit var grantFilesButton: Button

    private val logLines = StringBuilder()
    private var client: PairingClient? = null
    // M4-2/M4-3：常驻通道入口与状态显示。
    private lateinit var computerCard: View
    private lateinit var computerInfo: TextView
    private lateinit var linkStatus: TextView
    private lateinit var buttonLinkStart: Button
    private lateinit var buttonLinkStop: Button
    private var linkListener: (() -> Unit)? = null
    // 通知权限的后续动作：屏幕捕获与常驻连接都会请求 POST_NOTIFICATIONS，
    // 授权后按请求时的意图继续，而不是固定走某一条路。
    private var notificationFollowUp: (() -> Unit)? = null

    /**
     * 「发送文件到电脑」：系统选择器（SAF）选文件，结果交给 Outbox 复制进
     * 下载/MirrorDock。多选；取消时列表为空，不当作错误。
     */
    private val sendToPcPicker =
        registerForActivityResult(androidx.activity.result.contract.ActivityResultContracts.OpenMultipleDocuments()) { uris ->
            if (uris.isNullOrEmpty()) return@registerForActivityResult
            val result = Outbox.send(this, uris)
            log(result.message)
            Toast.makeText(this, result.message, Toast.LENGTH_LONG).show()
            refreshFiles()
        }

    private fun openSendPicker() {
        runCatching {
            sendToPcPicker.launch(arrayOf("*/*"))
        }.onFailure {
            log("无法打开系统文件选择器：${it.message}")
            Toast.makeText(this, "无法打开系统文件选择器", Toast.LENGTH_SHORT).show()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        CrashGuard.install(applicationContext)
        setContentView(R.layout.activity_main)
        statusText = findViewById(R.id.status_text)
        statusDot = findViewById(R.id.status_dot)
        logText = findViewById(R.id.log_text)
        manualInput = findViewById(R.id.manual_input)
        crashCard = findViewById(R.id.crash_card)
        crashDetail = findViewById(R.id.crash_detail)
        logScroller = findViewById(R.id.log_scroller)
        logToggle = findViewById(R.id.button_log_toggle)
        fileList = findViewById(R.id.file_list)
        fileEmpty = findViewById(R.id.file_empty)
        fileEmptyText = findViewById(R.id.file_empty_text)
        grantFilesButton = findViewById(R.id.button_grant_files)

        setStatusConnected(false)

        if (isTv()) {
            // TV 模式（X10-28）：电视没有相机，扫码不可用；手动输入是唯一配对路径，
            // 默认展开（不强制弹键盘——电视输入法由用户按 OK 主动唤起）。
            findViewById<Button>(R.id.button_scan).visibility = View.GONE
            manualInput.visibility = View.VISIBLE
            log("TV 模式：已隐藏扫码入口，请使用手动输入配对码连接电脑。")
        }

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
            val show = manualInput.visibility == View.GONE
            manualInput.visibility = if (show) View.VISIBLE else View.GONE
            val imm = getSystemService(android.view.inputmethod.InputMethodManager::class.java)
            if (show) {
                // 展开即聚焦并弹键盘：不弹键盘会被当成「点了没反应」。
                manualInput.post {
                    manualInput.requestFocus()
                    imm.showSoftInput(manualInput, android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT)
                }
            } else {
                imm.hideSoftInputFromWindow(manualInput.windowToken, 0)
            }
        }
        manualInput.setOnEditorActionListener { _, actionId, event ->
            // 只认键盘「前往」动作或实体回车，其余动作（如「下一步」）不触发连接。
            val isGo = actionId == android.view.inputmethod.EditorInfo.IME_ACTION_GO
            val isEnter = event != null &&
                event.keyCode == android.view.KeyEvent.KEYCODE_ENTER &&
                event.action == android.view.KeyEvent.ACTION_DOWN
            if (isGo || isEnter) {
                startWithPayload(manualInput.text.toString())
                true
            } else {
                false
            }
        }
        findViewById<Button>(R.id.button_capture).setOnClickListener { startCaptureFlow() }

        // 文件卡：刷新按钮 + 空状态里的「授权文件访问」+「发送文件到电脑」（a）。
        findViewById<Button>(R.id.button_files_refresh).setOnClickListener { refreshFiles() }
        findViewById<Button>(R.id.button_send_to_pc).setOnClickListener { openSendPicker() }
        grantFilesButton.setOnClickListener {
            ReceivedFiles.allFilesAccessIntent(this)?.let { intent ->
                runCatching { startActivity(intent) }
                    .onFailure { Toast.makeText(this, "无法打开系统设置", Toast.LENGTH_SHORT).show() }
            }
        }

        // 运行日志默认收起：正常使用时不需要看；点「查看」展开、再点「收起」。
        logToggle.setOnClickListener {
            val expanded = logScroller.visibility == View.VISIBLE
            logScroller.visibility = if (expanded) View.GONE else View.VISIBLE
            logToggle.setText(if (expanded) R.string.action_show else R.string.action_hide)
        }

        // 常驻通道（M4-2）：已配对电脑卡 + 免扫码直连入口；M4-4「解除这台电脑」。
        computerCard = findViewById(R.id.computer_card)
        computerInfo = findViewById(R.id.computer_info)
        linkStatus = findViewById(R.id.link_status)
        buttonLinkStart = findViewById(R.id.button_link_start)
        buttonLinkStop = findViewById(R.id.button_link_stop)
        buttonLinkStart.setOnClickListener { startPersistentLink() }
        buttonLinkStop.setOnClickListener {
            val intent = Intent(this, PersistentConnectionService::class.java)
                .setAction(PersistentConnectionService.ACTION_STOP)
            ContextCompat.startForegroundService(this, intent)
        }
        findViewById<Button>(R.id.button_unpair_computer).setOnClickListener { confirmUnpairComputer() }

        linkListener = {
            runOnUiThread { renderLinkState() }
        }

        // 崩溃取证：上次会话崩溃时显示一条可收敛的红卡——默认只显示标题，
        // 详情按需展开；点「清除」删掉落盘堆栈后立即消失，不影响后续正常使用。
        CrashGuard.lastCrash(applicationContext)?.let { last ->
            crashCard.visibility = View.VISIBLE
            crashDetail.text = last
            findViewById<Button>(R.id.button_crash_view).setOnClickListener {
                crashDetail.visibility =
                    if (crashDetail.visibility == View.GONE) View.VISIBLE else View.GONE
            }
            findViewById<Button>(R.id.button_crash_clear).setOnClickListener {
                CrashGuard.clear(applicationContext)
                crashCard.visibility = View.GONE
                log("已清除上次的崩溃记录。")
            }
            log("检测到上次运行崩溃，详情见上方红色卡片。")
        }
        log("伴侣 App 已启动。")
    }

    override fun onResume() {
        super.onResume()
        // 从系统设置（授权文件访问）或安装器返回时刷新列表。
        refreshFiles()
        // 已配对电脑卡与常驻连接状态（M4-2）。
        refreshComputerCard()
        linkListener?.let { LinkState.addListener(it) }
        renderLinkState()
    }

    override fun onPause() {
        super.onPause()
        linkListener?.let { LinkState.removeListener(it) }
    }

    // -- 常驻通道（M4-2）与解除互信（M4-4） ----------------------------------

    /** 已配对电脑卡：有互信凭据才显示；无凭据时隐藏（扫码配对入口照常）。 */
    private fun refreshComputerCard() {
        val prefs = getSharedPreferences("paired_computers", MODE_PRIVATE)
        val pairingId = prefs.getString("pairing_id", null)
        if (pairingId.isNullOrBlank()) {
            computerCard.visibility = View.GONE
            return
        }
        computerCard.visibility = View.VISIBLE
        val host = prefs.getString("last_host", null)?.substringBeforeLast(':') ?: "未知地址"
        val fingerprint = prefs.getString("desktop_fingerprint", null)?.take(16) ?: ""
        val residentPort = prefs.getInt("resident_port", -1).takeIf { it > 0 }
        val endpoint = if (residentPort != null) "$host:$residentPort" else host
        computerInfo.text = getString(R.string.computer_info, endpoint, fingerprint)
    }

    private fun renderLinkState() {
        linkStatus.text = when (LinkState.phase) {
            LinkPhase.IDLE -> getString(R.string.link_state_idle)
            LinkPhase.CONNECTING -> getString(R.string.link_state_connecting) + LinkState.detail
            LinkPhase.CONNECTED -> getString(R.string.link_state_connected) + LinkState.detail
            LinkPhase.RETRYING -> getString(R.string.link_state_retrying) + LinkState.detail
        }
        val running = LinkState.phase != LinkPhase.IDLE
        buttonLinkStart.visibility = if (running) View.GONE else View.VISIBLE
        buttonLinkStop.visibility = if (running) View.VISIBLE else View.GONE
    }

    /** 免扫码直连入口：Android 13+ 先请求通知权限（前台服务通知必须可见）。 */
    private fun startPersistentLink() {
        if (Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED
        ) {
            notificationFollowUp = { actuallyStartPersistentLink() }
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), REQUEST_NOTIFICATION)
            return
        }
        actuallyStartPersistentLink()
    }

    private fun actuallyStartPersistentLink() {
        val intent = Intent(this, PersistentConnectionService::class.java)
            .setAction(PersistentConnectionService.ACTION_START)
        ContextCompat.startForegroundService(this, intent)
    }

    /**
     * 解除这台电脑（M4-4）：停常驻服务 + 清互信凭据 + 销毁本机 Keystore 身份。
     * 与桌面端「移除互信」双向对齐——两端都作废后，下次连接必须重新扫码。
     * 确认对话框明示后果，不静默执行。
     */
    private fun confirmUnpairComputer() {
        android.app.AlertDialog.Builder(this)
            .setTitle(R.string.unpair_title)
            .setMessage(R.string.unpair_message)
            .setPositiveButton(R.string.action_unpair) { _, _ -> unpairComputer() }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    private fun unpairComputer() {
        // ① 停常驻连接（还在重试也要立刻停，避免解除后继续重连被拒刷通知）。
        val intent = Intent(this, PersistentConnectionService::class.java)
            .setAction(PersistentConnectionService.ACTION_STOP)
        ContextCompat.startForegroundService(this, intent)
        // ② 清互信凭据；③ 销毁 Keystore 身份（私钥不可导出，销毁即作废）。
        getSharedPreferences("paired_computers", MODE_PRIVATE).edit().clear().apply()
        PairingIdentity.destroy()
        refreshComputerCard()
        log("已解除与这台电脑的互信：本地凭据与身份已删除，下次连接需重新扫码。")
        Toast.makeText(this, "已解除互信", Toast.LENGTH_SHORT).show()
    }

    override fun onRequestPermissionsResult(
        requestCode: Int, permissions: Array<out String>, grantResults: IntArray,
    ) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        when (requestCode) {
            REQUEST_CAMERA -> {
                if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
                    openScanner()
                } else {
                    log("相机权限被拒绝：扫码需要相机，请在系统设置中允许后重试。")
                    Toast.makeText(this, "扫码需要相机权限", Toast.LENGTH_SHORT).show()
                }
            }
            REQUEST_NOTIFICATION -> {
                if (grantResults.firstOrNull() == PackageManager.PERMISSION_GRANTED) {
                    notificationFollowUp?.invoke()
                } else {
                    Toast.makeText(this, "需要通知权限才能显示连接状态", Toast.LENGTH_SHORT).show()
                }
                notificationFollowUp = null
            }
            REQUEST_READ_FILES -> refreshFiles()
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
            notificationFollowUp = { requestCaptureConsent() }
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
        // 发起连接后收起输入框和键盘，回到主状态。
        manualInput.visibility = View.GONE
        val imm = getSystemService(android.view.inputmethod.InputMethodManager::class.java)
        imm.hideSoftInputFromWindow(manualInput.windowToken, 0)
        connect(payload)
    }

    private fun connect(payload: PairingPayload) {
        client?.close()
        statusText.text = "正在连接 ${payload.hosts.first()}:${payload.port} …"
        setStatusConnected(false)
        log("使用一次性配对码 ${payload.token}（不会保存）")
        log("电脑身份指纹 ${payload.fingerprint.take(16)}…（与桌面长期身份核对）")
        val c = PairingClient(payload) { line -> runOnUiThread { log(line) } }
        client = c
        PairingBus.attach(c)
        Thread {
            c.connect(object : PairingClient.Listener {
                override fun onWelcome() = runOnUiThread {
                    statusText.text = "正在与电脑完成身份互验…"
                }

                override fun onPaired(pairingId: String, residentPort: Int?) = runOnUiThread {
                    statusText.text = "已与电脑建立互信会话。"
                    setStatusConnected(true)
                    log("配对成功（互信已建立，本机身份已登记到电脑）")
                    rememberComputer(pairingId, payload, residentPort)
                    // 边界如实告知（X10-31）：这条会话是伴侣通道，不等于镜像连接。
                    log("提示：这是伴侣助手通道，不会让手机出现在电脑的连接列表里；要镜像请用数据线或在电脑端完成无线调试配对。")
                    if (residentPort != null) {
                        log("电脑常驻通道已开启（端口 $residentPort），之后可在上方「上次配对的电脑」一键重连。")
                    } else {
                        log("电脑常驻通道未开启：在电脑端打开后，这里可以免扫码重连。")
                    }
                    refreshComputerCard()
                }

                override fun onRejected() = runOnUiThread {
                    statusText.text = "配对码被拒绝。请回到电脑重新生成配对。"
                    setStatusConnected(false)
                    c.close()
                }

                override fun onError(message: String) = runOnUiThread {
                    statusText.text = "连接失败：$message"
                    setStatusConnected(false)
                    c.close()
                }

                override fun onDisconnected() = runOnUiThread {
                    statusText.text = "会话已断开。"
                    setStatusConnected(false)
                }
            })
        }.start()
    }

    /**
     * 本地记住这台电脑（M4-1）：pairing_id + 桌面身份指纹 + 最近主机。
     * M4-2 起同时保存桌面常驻通道端口（residentPort），重连凭它免扫码直连；
     * 桌面端「移除互信」后这份记录随之作废（对端会拒绝 RECONNECT）。
     */
    private fun rememberComputer(pairingId: String, payload: PairingPayload, residentPort: Int?) {
        if (pairingId.isBlank()) return
        runCatching {
            val editor = getSharedPreferences("paired_computers", MODE_PRIVATE).edit()
                .putString("pairing_id", pairingId)
                .putString("desktop_fingerprint", payload.fingerprint)
                .putString("last_host", payload.hosts.firstOrNull())
                .putLong("paired_at", System.currentTimeMillis())
            if (residentPort != null) editor.putInt("resident_port", residentPort) else editor.remove("resident_port")
            editor.apply()
        }
    }

    private fun setStatusConnected(connected: Boolean) {
        statusDot.backgroundTintList =
            android.content.res.ColorStateList.valueOf(
                ContextCompat.getColor(this, if (connected) R.color.dot_ready else R.color.dot_idle),
            )
    }

    private fun log(line: String) {
        logLines.appendLine(line)
        logText.text = logLines.toString()
    }

    // -- 电脑发来的文件 ---------------------------------------------------------

    private fun refreshFiles() {
        // API ≤ 32 需要 READ_EXTERNAL_STORAGE；拒绝过就不再重复弹，走空状态提示。
        if (Build.VERSION.SDK_INT <= 32 &&
            ContextCompat.checkSelfPermission(this, Manifest.permission.READ_EXTERNAL_STORAGE)
            != PackageManager.PERMISSION_GRANTED
        ) {
            if (shouldShowRequestPermissionRationale(Manifest.permission.READ_EXTERNAL_STORAGE)) {
                showFilesEmpty(needsPermission = false)
                return
            }
            requestPermissions(arrayOf(Manifest.permission.READ_EXTERNAL_STORAGE), REQUEST_READ_FILES)
            return
        }
        Thread {
            val entries = ReceivedFiles.list(this)
            runOnUiThread { renderFiles(entries) }
        }.start()
    }

    private fun renderFiles(entries: List<ReceivedFiles.Entry>) {
        fileList.removeAllViews()
        if (entries.isEmpty()) {
            showFilesEmpty(needsPermission = !ReceivedFiles.canReadDirectly() && !ReceivedFiles.hasAllFilesAccess())
            return
        }
        fileEmpty.visibility = View.GONE
        val dateFormat = SimpleDateFormat("MM-dd HH:mm", Locale.getDefault())
        for (entry in entries) {
            fileList.addView(buildFileRow(entry, dateFormat))
        }
    }

    private fun showFilesEmpty(needsPermission: Boolean) {
        fileEmpty.visibility = View.VISIBLE
        fileEmptyText.setText(if (needsPermission) R.string.files_need_permission else R.string.files_empty)
        grantFilesButton.visibility = if (needsPermission) View.VISIBLE else View.GONE
    }

    /** 文件行：类型徽标 + 名称/元信息，整行可点（查看；APK 走系统安装器）。 */
    private fun buildFileRow(entry: ReceivedFiles.Entry, dateFormat: SimpleDateFormat): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = android.view.Gravity.CENTER_VERTICAL
            setPadding(0, dp(11), 0, dp(11))
            val margin = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT,
            )
            margin.topMargin = dp(4)
            layoutParams = margin
            isClickable = true
            isFocusable = true
            // TV 模式（X10-28）：D-pad 聚焦时用青碧描边高亮，焦点必须看得见。
            background = ContextCompat.getDrawable(context, R.drawable.bg_file_row)
        }
        val badge = TextView(this).apply {
            text = getString(if (entry.isApk) R.string.badge_apk else R.string.badge_file)
            setTextColor(ContextCompat.getColor(context, R.color.accent))
            textSize = 11f
            setTypeface(typeface, Typeface.BOLD)
            background = ContextCompat.getDrawable(context, R.drawable.bg_file_badge)
            setPadding(dp(9), dp(5), dp(9), dp(5))
        }
        row.addView(badge)
        val info = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            val params = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            params.leftMargin = dp(12)
            layoutParams = params
        }
        info.addView(TextView(this).apply {
            text = entry.name
            setTextColor(ContextCompat.getColor(context, R.color.text_primary))
            textSize = 14f
            setTypeface(typeface, Typeface.BOLD)
            maxLines = 1
            ellipsize = android.text.TextUtils.TruncateAt.MIDDLE
        })
        info.addView(TextView(this).apply {
            text = "${formatSize(entry.size)} · ${dateFormat.format(Date(entry.modifiedAt))}"
            setTextColor(ContextCompat.getColor(context, R.color.text_secondary))
            textSize = 12f
        })
        row.addView(info)
        row.addView(TextView(this).apply {
            text = if (entry.isApk) "安装" else "查看"
            setTextColor(ContextCompat.getColor(context, R.color.accent))
            textSize = 13f
            setTypeface(typeface, Typeface.BOLD)
        })
        row.setOnClickListener {
            if (!ReceivedFiles.open(this, entry)) {
                Toast.makeText(this, "无法打开这个文件。", Toast.LENGTH_SHORT).show()
            }
        }
        return row
    }

    private fun formatSize(bytes: Long): String = when {
        bytes >= 1 shl 20 -> "%.1f MB".format(bytes / 1048576f)
        bytes >= 1 shl 10 -> "%.0f KB".format(bytes / 1024f)
        else -> "$bytes B"
    }

    private fun dp(value: Int): Int = (value * resources.displayMetrics.density).toInt()

    /**
     * 是否运行在 Android TV 上（leanback 设备）。电视没有相机与触屏，
     * 界面据此裁剪扫码入口、默认展开手动输入；D-pad 焦点导航靠系统默认。
     */
    private fun isTv(): Boolean {
        val manager = getSystemService(android.app.UiModeManager::class.java) ?: return false
        return manager.currentModeType == android.content.res.Configuration.UI_MODE_TYPE_TELEVISION
    }
}

/**
 * 全局崩溃取证：任何未捕获异常（包括扫码页）先把堆栈写入应用私有目录，
 * 再交回系统默认处理（该崩还是崩）。下次启动把堆栈回显到主界面日志区。
 *
 * 为什么不静默吞掉：闪退必须能看到原因，否则无法排查；堆栈只落在本机，
 * 不自动上传，由用户拍照或手动发送。
 */
object CrashGuard {

    fun install(context: android.content.Context) {
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, throwable ->
            runCatching { save(context, throwable) }
            previous?.uncaughtException(thread, throwable)
        }
    }

    fun lastCrash(context: android.content.Context): String? {
        val file = java.io.File(context.getExternalFilesDir(null) ?: context.filesDir, MainActivity.CRASH_FILE)
        return runCatching { file.takeIf { it.exists() }?.readText() }.getOrNull()
    }

    /** 用户确认过崩溃信息后的清除入口；删文件而非写标记，避免下次启动再次出现。 */
    fun clear(context: android.content.Context) {
        val file = java.io.File(context.getExternalFilesDir(null) ?: context.filesDir, MainActivity.CRASH_FILE)
        runCatching { file.delete() }
    }

    private fun save(context: android.content.Context, throwable: Throwable) {
        val dir = context.getExternalFilesDir(null) ?: context.filesDir
        java.io.File(dir, MainActivity.CRASH_FILE).writeText(
            buildString {
                appendLine("时间: ${java.util.Date()}")
                appendLine("设备: ${android.os.Build.MANUFACTURER} ${android.os.Build.MODEL} · Android ${android.os.Build.VERSION.RELEASE} (API ${android.os.Build.VERSION.SDK_INT})")
                appendLine()
                appendLine(android.util.Log.getStackTraceString(throwable))
            },
        )
    }
}
