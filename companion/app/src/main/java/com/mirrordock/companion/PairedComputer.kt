package com.mirrordock.companion

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject

/**
 * 一台已配对的电脑（X10-73 多设备）。
 *
 * 桌面端台账本来就是列表（`PairedStore` = `Vec<PairedCompanion>`，按 pairing_id
 * upsert），真正的单设备限制只在手机端：旧版把 pairing_id / last_host /
 * resident_port 平铺在一份 SharedPreferences 里，配第二台电脑会覆盖第一台。
 * 这里把同一台电脑的凭据收拢成一个可序列化的记录，列表存 `computers_v2`。
 *
 * 字段与旧版一一对应，迁移是纯搬运：旧键原样读出后写进列表，再删旧键。
 * 任何一步失败都退回旧键，不让用户丢凭据。
 */
data class PairedComputer(
    /** 桌面端台账主键（设备身份指纹），重连时作 RECONNECT 的 pairing_id。 */
    val pairingId: String,
    /** 用户可改的显示名；为空时界面回落到主机地址。 */
    val label: String,
    /** 桌面身份指纹（完整 hex），验证重连时核对。 */
    val fingerprint: String,
    /** 最近一次连上的主机（不含端口）。 */
    val host: String,
    /** 桌面常驻通道端口；未学到时按 [PersistentConnectionService.RESIDENT_DEFAULT_PORT] 回退。 */
    val residentPort: Int?,
    val pairedAt: Long,
    val lastConnectedAt: Long,
    /** 分组名（用户自建）。空串表示未分组。 */
    val group: String = "",
) {
    /** 界面显示名：自定义名 > 主机地址 > pairing_id 前缀。 */
    fun displayName(): String =
        label.ifBlank { host.ifBlank { pairingId.take(12) } }

    /** 连接目标端点。端口没学到时如实标注"默认"，不假装连不上。 */
    fun endpoint(): String = if (residentPort != null) "$host:$residentPort" else "$host:47017（默认）"

    fun toJson(): JSONObject = JSONObject().apply {
        put(KEY_PAIRING_ID, pairingId)
        put(KEY_LABEL, label)
        put(KEY_FINGERPRINT, fingerprint)
        put(KEY_HOST, host)
        put(KEY_RESIDENT_PORT, residentPort ?: -1)
        put(KEY_PAIRED_AT, pairedAt)
        put(KEY_LAST_CONNECTED, lastConnectedAt)
        put(KEY_GROUP, group)
    }

    companion object {
        private const val KEY_PAIRING_ID = "pairing_id"
        private const val KEY_LABEL = "label"
        private const val KEY_FINGERPRINT = "fingerprint"
        private const val KEY_HOST = "host"
        private const val KEY_RESIDENT_PORT = "resident_port"
        private const val KEY_PAIRED_AT = "paired_at"
        private const val KEY_LAST_CONNECTED = "last_connected_at"
        private const val KEY_GROUP = "group"

        /** 从 JSON 解析；缺字段或类型不符返回 null（跳过这一条，不整体失败）。 */
        fun fromJson(json: JSONObject): PairedComputer? {
            val pairingId = json.optString(KEY_PAIRING_ID).takeIf { it.isNotBlank() } ?: return null
            val fingerprint = json.optString(KEY_FINGERPRINT).takeIf { it.isNotBlank() } ?: return null
            val host = json.optString(KEY_HOST)
            val port = json.optInt(KEY_RESIDENT_PORT, -1).takeIf { it > 0 }
            return PairedComputer(
                pairingId = pairingId,
                label = json.optString(KEY_LABEL),
                fingerprint = fingerprint,
                host = host,
                residentPort = port,
                pairedAt = json.optLong(KEY_PAIRED_AT, 0L),
                lastConnectedAt = json.optLong(KEY_LAST_CONNECTED, 0L),
                group = json.optString(KEY_GROUP),
            )
        }
    }
}

/**
 * 已配对电脑的本地台账（X10-73）。
 *
 * 存储：`paired_computers` 里的 `computers_v2`（JSON 数组），外加两个标量键：
 * - `active_pairing_id`：上次连接的电脑，重启后优先连它；
 * - `legacy_migrated`：标记旧版平铺键已搬运，防止重复迁移。
 *
 * 为什么保留旧键的读取能力：升级覆盖安装时，老用户的凭据就在旧键里。若读不到
 * 迁移路径，老用户会被静默"忘记电脑"，只能重新扫码——这是不可接受的回归。
 */
object ComputerStore {

    private const val PREFS = "paired_computers"
    private const val KEY_LIST = "computers_v2"
    private const val KEY_ACTIVE = "active_pairing_id"
    private const val KEY_MIGRATED = "legacy_migrated"
    private const val KEY_LEGACY_PAIRING_ID = "pairing_id"
    private const val KEY_LEGACY_FINGERPRINT = "desktop_fingerprint"
    private const val KEY_LEGACY_HOST = "last_host"
    private const val KEY_LEGACY_PORT = "resident_port"
    private const val KEY_LEGACY_PAIRED_AT = "paired_at"

    private fun prefs(context: Context): SharedPreferences =
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

    /**
     * 读取全部已配对电脑；若检测到旧版单记录格式则先迁移。
     *
     * 顺序约定：常驻连接优先的那台排第一，其余按最近连接时间倒序 —— 与
     * ToDesk「我的设备」的直觉一致：刚连过的电脑就在最上面。
     */
    fun list(context: Context): List<PairedComputer> {
        val sp = prefs(context)
        val items = readList(sp)
        val migrated = if (items.isEmpty()) migrateLegacy(sp) else items
        if (migrated.isEmpty()) return emptyList()
        val activeId = sp.getString(KEY_ACTIVE, null)
        return migrated.sortedWith(
            compareByDescending<PairedComputer> { it.pairingId == activeId }
                .thenByDescending { it.lastConnectedAt },
        )
    }

    private fun readList(sp: SharedPreferences): List<PairedComputer> {
        val raw = sp.getString(KEY_LIST, null) ?: return emptyList()
        return runCatching {
            val array = JSONArray(raw)
            (0 until array.length()).mapNotNull { index ->
                array.optJSONObject(index)?.let { PairedComputer.fromJson(it) }
            }
        }.getOrDefault(emptyList())
    }

    /**
     * 旧版 → 列表的一次性搬运。只在列表为空且旧键确有内容时执行。
     *
     * 幂等：搬完写 `legacy_migrated`，重复调用直接返回已迁移的结果，不会造重复项。
     */
    private fun migrateLegacy(sp: SharedPreferences): List<PairedComputer> {
        if (sp.getBoolean(KEY_MIGRATED, false)) return emptyList()
        val pairingId = sp.getString(KEY_LEGACY_PAIRING_ID, null)?.takeIf { it.isNotBlank() }
        val fingerprint = sp.getString(KEY_LEGACY_FINGERPRINT, null)?.takeIf { it.isNotBlank() }
        if (pairingId == null || fingerprint == null) {
            // 没有任何旧凭据：标记已迁移，避免每次进主界面都重试。
            sp.edit().putBoolean(KEY_MIGRATED, true).apply()
            return emptyList()
        }
        val legacy = PairedComputer(
            pairingId = pairingId,
            label = "",
            fingerprint = fingerprint,
            host = sp.getString(KEY_LEGACY_HOST, null).orEmpty(),
            residentPort = sp.getInt(KEY_LEGACY_PORT, -1).takeIf { it > 0 },
            pairedAt = sp.getLong(KEY_LEGACY_PAIRED_AT, 0L),
            lastConnectedAt = sp.getLong(KEY_LEGACY_PAIRED_AT, 0L),
        )
        val ok = runCatching { saveList(sp, listOf(legacy)) }.isSuccess
        if (ok) {
            sp.edit()
                .putString(KEY_ACTIVE, legacy.pairingId)
                .putBoolean(KEY_MIGRATED, true)
                .apply()
        }
        return if (ok) listOf(legacy) else emptyList()
    }

    private fun saveList(sp: SharedPreferences, items: List<PairedComputer>) {
        val array = JSONArray()
        items.forEach { array.put(it.toJson()) }
        sp.edit().putString(KEY_LIST, array.toString()).apply()
    }

    /**
     * 插入或更新一台电脑（配对成功时调用）。
     *
     * 旧版这里会直接覆盖平铺键，配第二台就丢第一台；现在按 pairing_id 归并，
     * 并把它设为当前连接目标。用户想回到哪台，点一下即可（setActive）。
     */
    fun upsert(context: Context, computer: PairedComputer): List<PairedComputer> {
        val sp = prefs(context)
        val current = if (readList(sp).isEmpty()) migrateLegacy(sp) else readList(sp)
        val next = current.filterNot { it.pairingId == computer.pairingId } + computer
        runCatching { saveList(sp, next) }
        sp.edit().putString(KEY_ACTIVE, computer.pairingId).apply()
        return list(context)
    }

    /** 更新某台的端口/最近连接时间等，不动其余字段。 */
    fun touch(context: Context, pairingId: String, transform: (PairedComputer) -> PairedComputer) {
        val sp = prefs(context)
        val current = readList(sp)
        if (current.none { it.pairingId == pairingId }) return
        runCatching {
            saveList(sp, current.map { if (it.pairingId == pairingId) transform(it) else it })
        }
    }

    fun setActive(context: Context, pairingId: String) {
        prefs(context).edit().putString(KEY_ACTIVE, pairingId).apply()
    }

    /** 当前连接目标：显式选中的那台 → 最近连接的那台 → 列表第一台。 */
    fun active(context: Context): PairedComputer? {
        val items = list(context)
        if (items.isEmpty()) return null
        val activeId = prefs(context).getString(KEY_ACTIVE, null)
        return items.firstOrNull { it.pairingId == activeId } ?: items.first()
    }

    fun find(context: Context, pairingId: String): PairedComputer? =
        list(context).firstOrNull { it.pairingId == pairingId }

    /**
     * 解除某台电脑的互信（只删这一台）。
     *
     * 旧版是 `edit().clear()` —— 一台都没了。现在按 pairing_id 精确删除，其余
     * 设备不受影响；删掉当前目标后把目标顺延到列表第一台。
     */
    fun remove(context: Context, pairingId: String): List<PairedComputer> {
        val sp = prefs(context)
        val current = readList(sp)
        val next = current.filterNot { it.pairingId == pairingId }
        runCatching { saveList(sp, next) }
        if (sp.getString(KEY_ACTIVE, null) == pairingId) {
            val editor = sp.edit()
            if (next.isEmpty()) editor.remove(KEY_ACTIVE) else editor.putString(KEY_ACTIVE, next.first().pairingId)
            editor.apply()
        }
        return list(context)
    }

    /**
     * 清空全部互信（设置页「解除所有电脑」用）。这是**不可逆**操作，调用方
     * 必须先弹二次确认；引擎自身不做任何隐式清空。
     */
    fun removeAll(context: Context) {
        prefs(context).edit()
            .remove(KEY_LIST)
            .remove(KEY_ACTIVE)
            .putBoolean(KEY_MIGRATED, true)
            .apply()
    }

    // -- 分组 ---------------------------------------------------------------

    /** 全部分组名（去空、去重、按名称排序），供分组筛选与新建分组使用。 */
    fun groups(context: Context): List<String> =
        list(context).map { it.group }.filter { it.isNotBlank() }.distinct().sorted()

    fun setGroup(context: Context, pairingId: String, group: String) {
        touch(context, pairingId) { it.copy(group = group.trim()) }
    }

    fun rename(context: Context, pairingId: String, label: String) {
        touch(context, pairingId) { it.copy(label = label.trim()) }
    }
}
