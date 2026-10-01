package com.mirrordock.companion

import android.content.Context
import android.net.Uri
import android.os.Build
import android.provider.MediaStore
import android.webkit.MimeTypeMap
import java.io.File

/**
 * 手机 → 电脑的发送区（与桌面端 list_device_files / fetch_file_from_device
 * 的固定目录一致）：下载/MirrorDock。
 *
 * 把用户经系统选择器（SAF）选中的文件复制进该目录；桌面端「工具」页刷新
 * 列表即可看到并取回。目录本身不新增权限：
 * - 已授予「所有文件访问」（ ReceivedFiles 的授权入口）：直接写文件系统；
 * - API ≥ 29 未授权：MediaStore Downloads 写入 RELATIVE_PATH
 *   Download/MirrorDock——应用对自己创建的媒体不需要任何存储权限；
 * - API < 29 且未授权：无法安全写入，如实返回引导文案（走 ReceivedFiles
 *   的授权入口后再试）。
 */
object Outbox {

    private const val TAG = "Outbox"

    data class Result(val sent: Int, val skipped: Int, val message: String)

    /** 发送：`uris` 来自 ACTION_OPEN_DOCUMENT（可多选）。 */
    fun send(context: Context, uris: List<Uri>): Result {
        var sent = 0
        var skipped = 0
        val failedNames = mutableListOf<String>()
        for (uri in uris) {
            val name = queryDisplayName(context, uri)
            if (name == null) {
                skipped++
                continue
            }
            val ok = if (ReceivedFiles.hasAllFilesAccess()) {
                writeDirectly(context, uri, name)
            } else if (Build.VERSION.SDK_INT >= 29) {
                writeViaMediaStore(context, uri, name)
            } else {
                android.util.Log.w(TAG, "无法写入 $name：无所有文件访问且 API<29")
                false
            }
            if (ok) sent++ else {
                skipped++
                failedNames += name
            }
        }
        val message = when {
            sent > 0 && skipped == 0 -> "已把 $sent 个文件放入发送区，在电脑端「工具」页点刷新即可取回。"
            sent > 0 -> "已发送 $sent 个文件；${skipped} 个失败（${failedNames.joinToString()}）。"
            else -> "没有文件被发送。若持续失败，请在文件卡里先授权文件访问再试。"
        }
        return Result(sent, skipped, message)
    }

    private fun queryDisplayName(context: Context, uri: Uri): String? =
        runCatching {
            context.contentResolver.query(
                uri,
                arrayOf(android.provider.OpenableColumns.DISPLAY_NAME),
                null, null, null,
            )?.use { c ->
                if (c.moveToFirst()) c.getString(0) else null
            }
        }.getOrNull()?.takeIf { it.isNotBlank() }

    /** 「所有文件访问」路径：直接写公共下载目录（与桌面端 adb pull 同一目录）。 */
    private fun writeDirectly(context: Context, uri: Uri, name: String): Boolean =
        try {
            val target = uniqueTarget(name)
            val input = context.contentResolver.openInputStream(uri)
            if (input == null) {
                android.util.Log.w(TAG, "$name：无输入流")
                false
            } else {
                input.use { stream ->
                    target.outputStream().use { output -> stream.copyTo(output) }
                }
                true
            }
        } catch (t: Throwable) {
            android.util.Log.w(TAG, "writeDirectly $name 失败", t)
            false
        }

    /** API 29+ 无权限路径：MediaStore 归档到 Download/MirrorDock。 */
    private fun writeViaMediaStore(context: Context, uri: Uri, name: String): Boolean {
        val resolver = context.contentResolver
        val values = android.content.ContentValues().apply {
            put(MediaStore.MediaColumns.DISPLAY_NAME, name)
            put(MediaStore.MediaColumns.RELATIVE_PATH, "Download/${ReceivedFiles.DIR_NAME}")
            put(MediaStore.MediaColumns.IS_PENDING, 1)
        }
        val collection = MediaStore.Downloads.getContentUri(MediaStore.VOLUME_EXTERNAL)
        val item = try {
            resolver.insert(collection, values)
        } catch (t: Throwable) {
            android.util.Log.w(TAG, "MediaStore insert $name 失败", t)
            null
        }
        if (item == null) {
            android.util.Log.w(TAG, "MediaStore insert $name 返回 null")
            return false
        }
        return try {
            val input = try {
                resolver.openInputStream(uri)
            } catch (t: Throwable) {
                android.util.Log.w(TAG, "$name：打开来源（SAF）失败", t)
                return false.also { runCatching { resolver.delete(item, null, null) } }
            }
            if (input == null) {
                android.util.Log.w(TAG, "$name：SAF 输入流为 null")
                return false.also { runCatching { resolver.delete(item, null, null) } }
            }
            input.use { stream ->
                val output = resolver.openOutputStream(item)
                if (output == null) {
                    android.util.Log.w(TAG, "$name：MediaStore 输出流为 null")
                    return false.also { runCatching { resolver.delete(item, null, null) } }
                }
                output.use { out -> stream.copyTo(out) }
            }
            values.clear()
            values.put(MediaStore.MediaColumns.IS_PENDING, 0)
            resolver.update(item, values, null, null)
            true
        } catch (t: Throwable) {
            android.util.Log.w(TAG, "MediaStore 写入 $name 失败", t)
            runCatching { resolver.delete(item, null, null) }
            false
        }
    }

    /** 同名不覆盖：追加序号（与桌面端 unique_file_path 行为一致）。 */
    private fun uniqueTarget(name: String): File {
        val dir = ReceivedFiles.dir()
        if (!dir.isDirectory) dir.mkdirs()
        var target = File(dir, name)
        if (!target.exists()) return target
        val stem = name.substringBeforeLast('.', missingDelimiterValue = name)
        val ext = name.substringAfterLast('.', "").let { if (it.isEmpty()) "" else ".$it" }
        var n = 1
        while (target.exists()) {
            target = File(dir, "$stem ($n)$ext")
            n++
        }
        return target
    }
}
