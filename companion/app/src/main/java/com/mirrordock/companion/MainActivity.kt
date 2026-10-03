package com.mirrordock.companion

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Typeface
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import androidx.appcompat.app.AppCompatActivity
import androidx.annotation.DimenRes
import androidx.core.content.ContextCompat
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * 主界面（X10-73 重构）：三 Tab 设备中心。
 *
 * 结构变化（配色不变，用户拍板方案 B）：
 * - 设备页 = 状态主卡 + 连接质量面板 + **我的设备列表（多设备）** + 分组筛选 + 配对入口；
 * - 文件页 = 电脑发来的文件 + 发送文件到电脑（从旧版长滚里提升为一级入口）；
 * - 设置页 = 通知镜像 / 屏幕捕获 / 互信管理 / 隐私 / 运行日志。
 *
 * 红线（本次重构不动）：加密直连与配对协议、扫码与手动输入两条路径、
 * 文件收发目录、解除互信的双端语义、日志不落敏感数据。
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

        /** 三个 Tab 的枚举下标（与 layout 中 page_* 的顺序一致）。 */
        private const val TAB_DEVICES = 0
        private const val TAB_FILES = 1
        private const val TAB_SETTINGS = 2
    }

    // -- 设备页 ---------------------------------------------------------------
    private lateinit var statusText: TextView
    private lateinit var statusDot: View
    private lateinit var qualityRtt: TextView
    private lateinit var qualityDuration: TextView
    private lateinit var qualityRow: LinearLayout
    private lateinit var deviceList: LinearLayout
    private lateinit var deviceEmpty: View
    private lateinit var groupFilterScroll: View
    private lateinit var groupFilterRow: LinearLayout
    private lateinit var buttonLinkStart: Button
    private lateinit var buttonLinkStop: Button

    // -- 文件页 ---------------------------------------------------------------
    private lateinit var fileList: LinearLayout
    private lateinit var fileEmpty: View
    private lateinit var fileEmptyText: TextView
    private lateinit var grantFilesButton: Button

    // -- 设置页 ---------------------------------------------------------------
    private lateinit var logText: TextView
    private lateinit var manualInput: EditText
    private lateinit var crashCard: View
    private lateinit var crashDetail: TextView
    private lateinit var logScroller: View
    private lateinit var logToggle: Button
    private lateinit var notifyMirrorButton: Button
    private lateinit var notifyMirrorStatus: TextView
    private lateinit var grantListenerButton: Button
    // X10-76 版本信息：此前 App 里一处都没显示过版本号。
    private lateinit var appVersionRow: View
    private lateinit var appVersionLabel: TextView
    private lateinit var appVersionDetail: TextView

    private val logLines = StringBuilder()
    private var client: PairingClient? = null
    private var linkListener: (() -> Unit)? = null
    /** 本会话内已上报过崩溃堆栈（避免每次界面刷新重复推送）。 */
    @Volatile private var crashReported = false
    // 通知权限的后续动作：屏幕捕获与常驻连接都会请求 POST_NOTIFICATIONS，
    // 授权后按请求时的意图继续，而不是固定走某一条路。
    private var notificationFollowUp: (() -> Unit)? = null

    /** 当前所在 Tab；设备列表按它决定刷新时机（文件/设置页不重建设备列表）。 */
    private var currentTab = TAB_DEVICES
    /** 设备列表当前筛选的分组；空串 = 全部。 */
    private var groupFilter = ""

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
            if (result.sent > 0) {
                // X10-60：通知电脑「发送区有新文件」，桌面端工具页实时提示。
                // 常驻连接未建立时跳过——文件已就位，电脑端手动刷新同样可见。
                if (!pushLinkLine("{\"type\":\"files_changed\"}")) {
                    log("常驻连接未建立，电脑端打开「工具」页刷新列表即可取回。")
                }
            }
            refreshFiles()
        }

    // X10-62：网络写入必须在后台线程——Android 主线程做 socket 写会抛
    // NetworkOnMainThreadException，被 PrintWriter 静默吞掉后 checkError 恒真，
    // 表现为「心跳（工作线程）全通、UI 线程推送全失败」（真机验证桩实锤）。
    // 单线程串行执行，保证消息顺序与心跳下行不乱序。
    private val linkSendExecutor = java.util.concurrent.Executors.newSingleThreadExecutor()

    /**
     * 经常驻连接推一行 JSON 给电脑（异步，主线程安全）。
     * 返回 false = 当前没有可用的常驻连接（调用方据此走「手动刷新」兜底提示）；
     * 返回 true = 已交由后台串行发送，结果记录在 logcat。
     */
    private fun pushLinkLine(line: String): Boolean {
        val sender = LinkState.lineSender ?: return false
        linkSendExecutor.execute {
            val ok = runCatching { sender(line) }.getOrDefault(false)
            android.util.Log.i("PersistentLink", "pushLinkLine: sent=$ok line=$line")
        }
        return true
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
        bindViews()
        setStatusConnected(false)
        setupTabs()
        setupDevicePage()
        setupFilesPage()
        setupSettingsPage()
        setupPairing()

        if (isTv()) {
            // TV 模式（X10-28）：电视没有相机，扫码不可用；手动输入是唯一配对路径，
            // 默认展开（不强制弹键盘——电视输入法由用户按 OK 主动唤起）。
            findViewById<Button>(R.id.button_scan).visibility = View.GONE
            manualInput.visibility = View.VISIBLE
            log("TV 模式：已隐藏扫码入口，请使用手动输入配对码连接电脑。")
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
            log("检测到上次运行崩溃，详情见设备页红色卡片。")
        }
        log("伴侣 App 已启动。")
    }

    private fun bindViews() {
        statusText = findViewById(R.id.status_text)
        statusDot = findViewById(R.id.status_dot)
        qualityRow = findViewById(R.id.quality_row)
        qualityRtt = findViewById(R.id.quality_rtt)
        qualityDuration = findViewById(R.id.quality_duration)
        deviceList = findViewById(R.id.device_list)
        deviceEmpty = findViewById(R.id.device_empty)
        groupFilterScroll = findViewById(R.id.group_filter_scroll)
        groupFilterRow = findViewById(R.id.group_filter_row)
        buttonLinkStart = findViewById(R.id.button_link_start)
        buttonLinkStop = findViewById(R.id.button_link_stop)

        fileList = findViewById(R.id.file_list)
        fileEmpty = findViewById(R.id.file_empty)
        fileEmptyText = findViewById(R.id.file_empty_text)
        grantFilesButton = findViewById(R.id.button_grant_files)

        logText = findViewById(R.id.log_text)
        manualInput = findViewById(R.id.manual_input)
        crashCard = findViewById(R.id.crash_card)
        crashDetail = findViewById(R.id.crash_detail)
        logScroller = findViewById(R.id.log_scroller)
        logToggle = findViewById(R.id.button_log_toggle)
        notifyMirrorButton = findViewById(R.id.button_notify_mirror)
        notifyMirrorStatus = findViewById(R.id.notify_mirror_status)
        grantListenerButton = findViewById(R.id.button_grant_listener)
        bindAppVersion()
    }

    // -- 三 Tab 导航（X10-73） ------------------------------------------------

    private fun setupTabs() {
        val pages = listOf<View>(findViewById(R.id.page_devices), findViewById(R.id.page_files), findViewById(R.id.page_settings))
        val navs = listOf<View>(findViewById(R.id.nav_devices), findViewById(R.id.nav_files), findViewById(R.id.nav_settings))
        val icons = listOf<ImageView>(findViewById(R.id.nav_devices_icon), findViewById(R.id.nav_files_icon), findViewById(R.id.nav_settings_icon))
        val labels = listOf<TextView>(findViewById(R.id.nav_devices_label), findViewById(R.id.nav_files_label), findViewById(R.id.nav_settings_label))
        val accent = ContextCompat.getColor(this, R.color.accent)
        val idle = ContextCompat.getColor(this, R.color.nav_unselected)

        fun select(index: Int) {
            currentTab = index
            pages.forEachIndexed { i, page -> page.visibility = if (i == index) View.VISIBLE else View.GONE }
            navs.forEachIndexed { i, nav ->
                val active = i == index
                nav.isSelected = active
                // 选中态同时改图标 tint 与文字色，色弱用户不只靠颜色区分（有底色块）。
                icons[i].setColorFilter(if (active) accent else idle)
                labels[i].setTextColor(if (active) accent else idle)
            }
            // 切到文件页时刷新一次列表：桌面端可能在后台又推了新文件过来。
            if (index == TAB_FILES) refreshFiles()
            if (index == TAB_DEVICES) renderDeviceList()
        }

        navs.forEachIndexed { i, nav -> nav.setOnClickListener { select(i) } }
        select(TAB_DEVICES)
    }

    // -- 设备页 ---------------------------------------------------------------

    private fun setupDevicePage() {
        buttonLinkStart.setOnClickListener { startPersistentLink() }
        buttonLinkStop.setOnClickListener {
            val intent = Intent(this, PersistentConnectionService::class.java)
                .setAction(PersistentConnectionService.ACTION_STOP)
            ContextCompat.startForegroundService(this, intent)
        }
        findViewById<Button>(R.id.button_manage_groups).setOnClickListener { showGroupManager() }
        findViewById<Button>(R.id.button_unpair_computer).setOnClickListener { confirmUnpairComputer() }
        // X10-73 桌面快捷方式：由系统弹确认框，未获用户同意不会静默上桌面。
        findViewById<Button>(R.id.button_pin_shortcut).setOnClickListener {
            Toast.makeText(this, LinkShortcutManager.requestPin(this), Toast.LENGTH_LONG).show()
        }
    }

    /**
     * 设备列表（X10-73 多设备）：逐台渲染，点行即把它设为当前连接目标。
     *
     * 状态如实表达：当前连接目标打勾；能否连上不猜——只有常驻服务真的连上
     * 且 pairing_id 匹配时才显示"在线"，其余一律"离线"，避免用户以为能连。
     */
    private fun renderDeviceList() {
        val computers = ComputerStore.list(this)
        val activeId = ComputerStore.active(this)?.pairingId
        val connectedId = LinkQuality.currentPairingId.takeIf { LinkState.phase == LinkPhase.CONNECTED }

        val visible = if (groupFilter.isBlank()) computers else computers.filter { it.group == groupFilter }
        deviceList.removeAllViews()
        deviceEmpty.visibility = if (visible.isEmpty()) View.VISIBLE else View.GONE

        for (computer in visible) {
            deviceList.addView(buildDeviceRow(computer, computer.pairingId == activeId, computer.pairingId == connectedId))
        }
        renderGroupFilter(computers)
    }

    private fun buildDeviceRow(computer: PairedComputer, isActive: Boolean, isOnline: Boolean): View {
        val row = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(R.dimen.space_4), dp(R.dimen.space_3), dp(R.dimen.space_4), dp(R.dimen.space_3))
            background = ContextCompat.getDrawable(context, R.drawable.bg_device_item)
            isSelected = isActive
            isClickable = true
            isFocusable = true
            val params = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT,
            )
            params.bottomMargin = dp(R.dimen.space_2)
            layoutParams = params
        }

        // 左列：名称 + 徽标 + 元信息。
        val left = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            val params = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            layoutParams = params
        }
        val nameRow = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        nameRow.addView(TextView(this).apply {
            text = computer.displayName()
            setTextColor(ContextCompat.getColor(context, R.color.text_primary))
            textSize = 15f
            setTypeface(typeface, Typeface.BOLD)
            maxLines = 1
        })
        // 状态徽标：在线绿 / 离线灰，如实区分。
        nameRow.addView(TextView(this).apply {
            text = getString(if (isOnline) R.string.device_state_online else R.string.device_state_offline)
            setTextColor(
                ContextCompat.getColor(context, if (isOnline) R.color.success else R.color.text_tertiary),
            )
            textSize = 11f
            setTypeface(typeface, Typeface.BOLD)
            background = ContextCompat.getDrawable(context, R.drawable.bg_status_pill)
            setPadding(dp(R.dimen.space_2), dp(R.dimen.space_1), dp(R.dimen.space_2), dp(R.dimen.space_1))
            val lp = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT,
            )
            lp.leftMargin = dp(R.dimen.space_2)
            layoutParams = lp
        })
        if (computer.group.isNotBlank()) {
            nameRow.addView(TextView(this).apply {
                text = computer.group
                setTextColor(ContextCompat.getColor(context, R.color.accent_dark))
                textSize = 11f
                background = ContextCompat.getDrawable(context, R.drawable.bg_status_pill)
                setPadding(dp(R.dimen.space_2), dp(R.dimen.space_1), dp(R.dimen.space_2), dp(R.dimen.space_1))
                val lp = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT,
                )
                lp.leftMargin = dp(R.dimen.space_2)
                layoutParams = lp
            })
        }
        left.addView(nameRow)
        left.addView(TextView(this).apply {
            text = getString(R.string.device_meta, computer.endpoint(), computer.fingerprint)
            setTextColor(ContextCompat.getColor(context, R.color.text_secondary))
            textSize = 12f
        })
        left.addView(TextView(this).apply {
            text = if (computer.lastConnectedAt > 0) {
                getString(R.string.device_last_connected, relativeTime(computer.lastConnectedAt))
            } else {
                getString(R.string.device_never_connected)
            }
            setTextColor(ContextCompat.getColor(context, R.color.text_tertiary))
            textSize = 11.5f
        })
        row.addView(left)

        // 右列：操作（当前目标打勾 + 更多菜单）。
        val right = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
        }
        if (isActive) {
            right.addView(TextView(this).apply {
                text = "✓"
                setTextColor(ContextCompat.getColor(context, R.color.accent))
                textSize = 16f
                setTypeface(typeface, Typeface.BOLD)
            })
        }
        // 「⋯」更多操作用 TextView 而非 Button：Button 自带最小宽高与内边距，
        // 为了塞进设备行右侧得反过来跟系统默认样式搏斗；TextView 干净可控。
        right.addView(TextView(this).apply {
            text = "⋯"
            setTextColor(ContextCompat.getColor(context, R.color.text_secondary))
            textSize = 18f
            gravity = Gravity.CENTER
            setPadding(dp(R.dimen.space_2), 0, dp(R.dimen.space_2), 0)
            isClickable = true
            isFocusable = true
            minWidth = dp(R.dimen.space_8)
            setOnClickListener { showDeviceMenu(computer) }
        })
        row.addView(right)

        // 点整行 = 设为当前连接目标（与「连接上次配对的电脑」一致）。
        row.setOnClickListener {
            ComputerStore.setActive(this, computer.pairingId)
            renderDeviceList()
            Toast.makeText(this, getString(R.string.device_connected_toast, computer.displayName()), Toast.LENGTH_SHORT).show()
        }
        return row
    }

    /** 分组筛选 chips：全部 + 各分组。 */
    private fun renderGroupFilter(computers: List<PairedComputer>) {
        val groups = computers.map { it.group }.filter { it.isNotBlank() }.distinct().sorted()
        groupFilterRow.removeAllViews()
        // 只有一台以上、或存在分组时才显示筛选条（单台电脑时筛选无意义）。
        if (computers.size <= 1 && groups.isEmpty()) {
            groupFilterScroll.visibility = View.GONE
            return
        }
        groupFilterScroll.visibility = View.VISIBLE
        val options = listOf("" to getString(R.string.group_all)) +
            groups.map { it to it } +
            (if (computers.any { it.group.isBlank() }) listOf("__none__" to getString(R.string.group_ungrouped)) else emptyList())
        for ((value, label) in options) {
            val selected = if (value == "__none__") groupFilter == "__none__" else groupFilter == value
            groupFilterRow.addView(TextView(this).apply {
                text = label
                textSize = 12.5f
                setTextColor(
                    ContextCompat.getColor(context, if (selected) R.color.accent_dark else R.color.text_secondary),
                )
                setTypeface(typeface, if (selected) Typeface.BOLD else Typeface.NORMAL)
                background = ContextCompat.getDrawable(context, R.drawable.bg_status_pill)
                setPadding(dp(R.dimen.space_3), dp(R.dimen.space_2), dp(R.dimen.space_3), dp(R.dimen.space_2))
                isClickable = true
                setOnClickListener {
                    groupFilter = value
                    renderDeviceList()
                }
                val params = LinearLayout.LayoutParams(
                    LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT,
                )
                params.rightMargin = dp(R.dimen.space_2)
                layoutParams = params
            })
        }
    }

    /** 设备行「⋯」菜单：重命名 / 归入分组 / 解除这台。 */
    private fun showDeviceMenu(computer: PairedComputer) {
        val actions = arrayOf(getString(R.string.action_rename), getString(R.string.action_group), getString(R.string.action_unpair))
        android.app.AlertDialog.Builder(this)
            .setTitle(computer.displayName())
            .setItems(actions) { _, which ->
                when (which) {
                    0 -> showRenameDialog(computer)
                    1 -> showGroupPicker(computer)
                    2 -> confirmUnpairComputer(computer)
                }
            }
            .show()
    }

    private fun showRenameDialog(computer: PairedComputer) {
        val input = EditText(this).apply {
            setText(computer.label)
            hint = getString(R.string.device_name_hint)
            setSingleLine()
        }
        android.app.AlertDialog.Builder(this)
            .setTitle(R.string.device_rename_title)
            .setView(input)
            .setPositiveButton(android.R.string.ok) { _, _ ->
                ComputerStore.rename(this, computer.pairingId, input.text.toString())
                renderDeviceList()
            }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    private fun showGroupPicker(computer: PairedComputer) {
        val existing = ComputerStore.groups(this)
        val options = (listOf("") + existing).toTypedArray()
        val preselected = options.indexOf(computer.group).coerceAtLeast(0)
        android.app.AlertDialog.Builder(this)
            .setTitle(R.string.action_group)
            .setSingleChoiceItems(options, preselected) { dialog, which ->
                ComputerStore.setGroup(this, computer.pairingId, options[which])
                renderDeviceList()
                dialog.dismiss()
                // 空串是「移出分组」，不是错误路径，文案如实区分。
                Toast.makeText(
                    this,
                    if (options[which].isBlank()) getString(R.string.group_removed)
                    else getString(R.string.group_saved, options[which]),
                    Toast.LENGTH_SHORT,
                ).show()
            }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    /** 分组管理入口：一次看完所有设备及其分组，并可直接改。 */
    private fun showGroupManager() {
        val computers = ComputerStore.list(this)
        if (computers.isEmpty()) {
            Toast.makeText(this, getString(R.string.unpair_none_title), Toast.LENGTH_SHORT).show()
            return
        }
        val labels = computers.map { computer ->
            val group = computer.group.ifBlank { getString(R.string.group_ungrouped) }
            "${computer.displayName()} — $group"
        }.toTypedArray()
        android.app.AlertDialog.Builder(this)
            .setTitle(R.string.group_manage_title)
            .setItems(labels) { _, which -> showGroupPicker(computers[which]) }
            .setMessage(R.string.group_manage_hint)
            .setPositiveButton(android.R.string.ok, null)
            .show()
    }

    /** 相对时间（用于「上次连接」）。与桌面端口径一致。 */
    private fun relativeTime(epochMs: Long): String {
        val diff = System.currentTimeMillis() - epochMs
        val minutes = diff / 60_000
        return when {
            minutes < 1 -> "刚刚"
            minutes < 60 -> "$minutes 分钟前"
            minutes < 60 * 24 -> "${minutes / 60} 小时前"
            else -> "${minutes / (60 * 24)} 天前"
        }
    }

    // -- 文件页 ---------------------------------------------------------------

    private fun setupFilesPage() {
        findViewById<Button>(R.id.button_files_refresh).setOnClickListener { refreshFiles() }
        findViewById<Button>(R.id.button_send_to_pc).setOnClickListener { openSendPicker() }
        grantFilesButton.setOnClickListener {
            ReceivedFiles.allFilesAccessIntent(this)?.let { intent ->
                runCatching { startActivity(intent) }
                    .onFailure { Toast.makeText(this, "无法打开系统设置", Toast.LENGTH_SHORT).show() }
            }
        }
    }

    // -- 设置页 ---------------------------------------------------------------

    private fun setupSettingsPage() {
        notifyMirrorButton.setOnClickListener { toggleNotifyMirror() }
        grantListenerButton.setOnClickListener {
            NotificationMirrorService.openListenerSettings(this)
        }
        findViewById<Button>(R.id.button_capture).setOnClickListener { startCaptureFlow() }
        // 运行日志默认收起：正常使用时不需要看；点「查看」展开、再点「收起」。
        logToggle.setOnClickListener {
            val expanded = logScroller.visibility == View.VISIBLE
            logScroller.visibility = if (expanded) View.GONE else View.VISIBLE
            logToggle.setText(if (expanded) R.string.action_show else R.string.action_hide)
        }
    }

    // -- 配对（设备页） -------------------------------------------------------

    private fun setupPairing() {
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
    }

    override fun onResume() {
        super.onResume()
        // 从系统设置（授权文件访问）或安装器返回时刷新列表。
        refreshFiles()
        // X10-66：从系统通知授权页返回时，如实刷新镜像开关状态。
        refreshNotifyMirrorUi()
        // 设备中心（M4-2 / X10-73）。
        renderDeviceList()
        linkListener?.let { LinkState.addListener(it) }
        linkListener = {
            runOnUiThread {
                renderLinkState()
                // 设备行上的「在线」标记依赖连接状态，连接变化要跟着重画。
                if (currentTab == TAB_DEVICES) renderDeviceList()
            }
        }
        renderLinkState()
    }

    override fun onPause() {
        super.onPause()
        linkListener?.let { LinkState.removeListener(it) }
    }

    // -- 常驻通道（M4-2）与解除互信（M4-4） ----------------------------------

    private fun renderLinkState() {
        linkStatusText().text = when (LinkState.phase) {
            LinkPhase.IDLE -> getString(R.string.link_state_idle)
            LinkPhase.CONNECTING -> getString(R.string.link_state_connecting) + LinkState.detail
            LinkPhase.CONNECTED -> getString(R.string.link_state_connected) + LinkState.detail
            LinkPhase.RETRYING -> getString(R.string.link_state_retrying) + LinkState.detail
        }
        val running = LinkState.phase != LinkPhase.IDLE
        buttonLinkStart.visibility = if (running) View.GONE else View.VISIBLE
        buttonLinkStop.visibility = if (running) View.VISIBLE else View.GONE
        renderQualityPanel()
        maybeReportLastCrash()
    }

    /**
     * 连接质量面板（X10-73）：延迟 / 已连接时长 / 加密直连。
     *
     * 未连接时显示引导语而不是空行；RTT 没采到样显示「—」，不显示 0 ——
     * 0 会被读成"延迟极低"，那是假的。
     */
    private fun renderQualityPanel() {
        val connected = LinkState.phase == LinkPhase.CONNECTED
        qualityRow.visibility = View.VISIBLE
        if (!connected) {
            qualityRtt.text = getString(R.string.quality_row_idle)
            qualityDuration.text = ""
            return
        }
        val rtt = LinkQuality.rttMs
        qualityRtt.text = if (rtt != null) getString(R.string.quality_rtt, rtt) else getString(R.string.quality_unknown)
        val seconds = LinkQuality.connectedSeconds()
        qualityDuration.text = getString(R.string.quality_duration, formatDuration(seconds))
    }

    private fun formatDuration(seconds: Long): String {
        val minutes = seconds / 60
        return when {
            minutes < 1 -> "不到 1 分钟"
            minutes < 60 -> getString(R.string.quality_duration_minutes, minutes.toInt())
            else -> getString(
                R.string.quality_duration_hours,
                (minutes / 60).toInt(), (minutes % 60).toInt(),
            )
        }
    }

    /** 状态主卡的状态行（X10-73 里就是 statusText，单独取以便阅读）。 */
    private fun linkStatusText(): TextView = statusText

    /**
     * 崩溃堆栈经本地点对点通道报给电脑（X10-60）：常驻连接就绪且有未上报的
     * 崩溃记录时推一次。堆栈只落手机本机与电脑界面，不经任何云端；用户点
     * 「清除」后不再上报。截断到 8000 字符以内，避免超出协议单行上限。
     */
    private fun maybeReportLastCrash() {
        if (crashReported || LinkState.phase != LinkPhase.CONNECTED) return
        val stack = CrashGuard.lastCrash(applicationContext) ?: return
        crashReported = true
        val payload = org.json.JSONObject().apply {
            put("type", "last_crash")
            put("stack", stack.take(8000))
        }
        if (pushLinkLine(payload.toString())) {
            log("已把上次的崩溃记录报给电脑（可在电脑端查看）。")
        }
    }

    // -- 通知镜像（X10-66 一期） ------------------------------------------------

    /** 开关切换：开启时若还没授予「读取通知」，引导去系统设置（授权后回来自动刷新）。 */
    private fun toggleNotifyMirror() {
        val prefs = getSharedPreferences(NotificationMirrorService.PREFS, MODE_PRIVATE)
        val turningOn = !prefs.getBoolean(NotificationMirrorService.KEY_NOTIFY_MIRROR, false)
        prefs.edit().putBoolean(NotificationMirrorService.KEY_NOTIFY_MIRROR, turningOn).apply()
        if (turningOn && !NotificationMirrorService.isListenerGranted(this)) {
            Toast.makeText(this, getString(R.string.notify_mirror_toast_need_permission), Toast.LENGTH_LONG).show()
            NotificationMirrorService.openListenerSettings(this)
        } else if (turningOn) {
            log("通知镜像已开启：连接电脑后，新通知将实时转发。")
        } else {
            log("通知镜像已关闭。")
        }
        refreshNotifyMirrorUi()
    }

    /** 状态行如实反映三态：关 / 开（缺授权）/ 开（就绪）。 */
    private fun refreshNotifyMirrorUi() {
        val prefs = getSharedPreferences(NotificationMirrorService.PREFS, MODE_PRIVATE)
        val on = prefs.getBoolean(NotificationMirrorService.KEY_NOTIFY_MIRROR, false)
        val granted = NotificationMirrorService.isListenerGranted(this)
        notifyMirrorButton.setText(
            if (on) R.string.action_notify_mirror_off else R.string.action_notify_mirror_on,
        )
        notifyMirrorStatus.setText(
            when {
                !on -> R.string.notify_mirror_status_off
                granted -> R.string.notify_mirror_status_on
                else -> R.string.notify_mirror_status_need_permission
            },
        )
        grantListenerButton.visibility = if (on && !granted) View.VISIBLE else View.GONE
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
     * 解除互信（M4-4 / X10-73）：停常驻服务 + 删**这一台**的凭据 + 销毁本机
     * Keystore 身份。与桌面端「移除互信」双向对齐——两端都作废后，下次连接
     * 必须重新扫码。确认对话框明示后果，不静默执行。
     *
     * 传 null = 解除当前连接目标那台。
     */
    private fun confirmUnpairComputer(target: PairedComputer? = null) {
        val computer = target ?: ComputerStore.active(this)
        if (computer == null) {
            Toast.makeText(this, getString(R.string.unpair_none_title), Toast.LENGTH_SHORT).show()
            return
        }
        android.app.AlertDialog.Builder(this)
            .setTitle(getString(R.string.unpair_title))
            .setMessage(getString(R.string.unpair_message) + "\n\n" + computer.displayName())
            .setPositiveButton(R.string.action_unpair) { _, _ -> unpairComputer(computer) }
            .setNegativeButton(android.R.string.cancel, null)
            .show()
    }

    private fun unpairComputer(computer: PairedComputer) {
        // ① 停常驻连接（还在重试也要立刻停，避免解除后继续重连被拒刷通知）。
        val intent = Intent(this, PersistentConnectionService::class.java)
            .setAction(PersistentConnectionService.ACTION_STOP)
        ContextCompat.startForegroundService(this, intent)
        // ② 删这一台互信凭据（其它已配对电脑不受影响）。
        ComputerStore.remove(this, computer.pairingId)
        // ③ 销毁 Keystore 身份（私钥不可导出，销毁即作废）。
        //    身份是**本机**的而非每台电脑一份，所以任何一台解除后重新配对都要
        //    重新握手；但已保存的其它电脑凭据不会被连带清掉，重新扫码即可各自恢复。
        PairingIdentity.destroy()
        renderDeviceList()
        renderLinkState()
        log("已解除与「${computer.displayName()}」的互信：本地凭据与身份已删除，下次连接需重新扫码。")
        Toast.makeText(this, getString(R.string.device_removed_toast, computer.displayName()), Toast.LENGTH_SHORT).show()
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
                        log("电脑常驻通道已开启（端口 $residentPort），之后可在「我的设备」里一键重连。")
                    } else {
                        // X10-64：文案必须与真实能力一致——本机会回退默认端口 47017
                        // 重连；只有电脑端口被占用回退到随机端口时才需要重新扫码。
                        log("电脑常驻通道未开启：在电脑端打开「常驻通道」后，「我的设备」里即可免扫码重连（本机按默认端口 47017 直连；仅当电脑端口被占用回退到其他端口时需要重新扫一次码）。")
                    }
                    renderDeviceList()
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
     * 本地记住这台电脑（M4-1 / X10-73）：按 pairing_id 存进列表。
     *
     * 旧版把凭据平铺进一份 prefs，配第二台就覆盖第一台；现在 upsert 到列表并
     * 设为当前连接目标。桌面常驻端口（residentPort）一并存下，重连凭它免扫码
     * 直连；桌面端「移除互信」后这份记录随之作废（对端会拒绝 RECONNECT）。
     */
    private fun rememberComputer(pairingId: String, payload: PairingPayload, residentPort: Int?) {
        if (pairingId.isBlank()) return
        val host = payload.hosts.firstOrNull().orEmpty()
        val existing = ComputerStore.find(this, pairingId)
        ComputerStore.upsert(
            this,
            PairedComputer(
                pairingId = pairingId,
                // 重新配对同一台时保留用户改过的显示名与分组，别把人家的整理成果冲掉。
                label = existing?.label.orEmpty(),
                fingerprint = payload.fingerprint,
                host = host,
                residentPort = residentPort,
                pairedAt = existing?.pairedAt ?: System.currentTimeMillis(),
                lastConnectedAt = System.currentTimeMillis(),
                group = existing?.group.orEmpty(),
            ),
        )
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
            setPadding(0, dp(R.dimen.space_3), 0, dp(R.dimen.space_3))
            val margin = LinearLayout.LayoutParams(
                LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT,
            )
            margin.topMargin = dp(R.dimen.space_1)
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
            setPadding(dp(R.dimen.space_2), dp(R.dimen.badge_pad_v), dp(R.dimen.space_2), dp(R.dimen.badge_pad_v))
        }
        row.addView(badge)
        val info = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            val params = LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f)
            params.leftMargin = dp(R.dimen.space_3)
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

    // -- 版本信息（X10-76） -------------------------------------------------
    //
    // 为什么用 PackageInfo 而不是 BuildConfig.VERSION_NAME：
    // - BuildConfig 是**编译期常量**（内联进字节码），反映的是打包时的值；
    //   一旦 APK 被重打包或走多渠道分发就会失真。而且 AGP 8.0 起默认
    //   **不生成** BuildConfig 类，要用它得先加 buildFeatures { buildConfig = true }。
    // - PackageInfo 读的是**手机上实际安装的 APK** 的元数据 —— 报障时
    //   「这台手机上装的到底是什么」只有它是真相来源。
    // 结论：用 PackageInfo，顺带不用改构建配置，耦合更低。

    /** 版本信息。显示与复制共用同一个数据类，避免两处逻辑漂移。 */
    private data class AppVersion(val name: String, val code: Long) {
        /** 屏幕显示：0.3.0（17） */
        fun label(): String = "$name（$code）"
    }

    private fun appVersion(): AppVersion? = try {
        val pm = packageManager
        // API 33 起 getPackageInfo(String,int) 被 PackageInfoFlags 取代，
        // 必须分支 —— targetSdk 34 上旧重载的行为不确定。
        val info = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            pm.getPackageInfo(packageName, PackageManager.PackageInfoFlags.of(0L))
        } else {
            @Suppress("DEPRECATION")
            pm.getPackageInfo(packageName, 0)
        }
        // versionCode 在 API 28 起被 longVersionCode 取代（Int 会溢出）
        val code = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            info.longVersionCode
        } else {
            @Suppress("DEPRECATION")
            info.versionCode.toLong()
        }
        AppVersion(info.versionName ?: "?", code)
    } catch (error: Exception) {
        // 读不到就如实说读不到，不显示空白 —— 空白会被读成"没有版本"。
        android.util.Log.w("AppVersion", "读取版本信息失败", error)
        null
    }

    private fun bindAppVersion() {
        appVersionRow = findViewById(R.id.row_app_version)
        appVersionLabel = findViewById(R.id.app_version_label)
        appVersionDetail = findViewById(R.id.app_version_detail)

        val version = appVersion()
        appVersionLabel.text = version?.label() ?: getString(R.string.app_version_unknown)

        // 点一下展开完整信息（版本 + 系统 + 架构）。小白报障时被要求
        // 「告诉我你的版本」，往下翻到底就能看到 —— 这是这个功能的真实路径。
        appVersionRow.setOnClickListener {
            if (appVersionDetail.visibility == View.VISIBLE) {
                appVersionDetail.visibility = View.GONE
                return@setOnClickListener
            }
            val v = version ?: return@setOnClickListener
            appVersionDetail.text = getString(
                R.string.app_version_detail,
                v.label(),
                Build.VERSION.RELEASE,
                Build.SUPPORTED_ABIS.firstOrNull() ?: "?",
            )
            appVersionDetail.visibility = View.VISIBLE
        }

        // 长按复制完整信息 —— 小白报障的真实动作是复制粘贴，不是朗读。
        appVersionRow.setOnLongClickListener {
            val clip = getSystemService(android.content.ClipboardManager::class.java)
            clip.setPrimaryClip(
                android.content.ClipData.newPlainText(
                    getString(R.string.app_name),
                    buildString {
                        append(getString(R.string.app_name))
                        append(' ')
                        append(version?.label() ?: "?")
                        append(" · Android ").append(Build.VERSION.RELEASE)
                        append(" (API ").append(Build.VERSION.SDK_INT).append(')')
                        append(" · ").append(Build.SUPPORTED_ABIS.firstOrNull() ?: "?")
                    },
                ),
            )
            // Android 13+ 系统会自己弹"已复制"提示，我们就不再重复弹一次。
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                Toast.makeText(this, R.string.app_version_copied, Toast.LENGTH_SHORT).show()
            }
            true
        }
    }

    /**
     * 读间距/尺寸令牌。
     *
     * 旧实现是 `(value * density)` —— 它**绕过资源限定符**：values-night 与
     * values-sw600dp 永远不会生效。暗色主题要靠同一批令牌在 night 下取到
     * 不同值，就必须走 getDimensionPixelSize(@DimenRes)。
     */
    private fun dp(@DimenRes id: Int): Int = resources.getDimensionPixelSize(id)

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
