package com.mirrordock.companion

import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.provider.Settings
import android.webkit.MimeTypeMap
import androidx.core.content.FileProvider
import java.io.File

/**
 * 电脑推送目录（与桌面端 send_file_to_device 的固定目标一致）：下载/MirrorDock。
 *
 * 访问策略（Android 存储分区的现实约束，逐级降级、如实呈现）：
 * 1. 直接文件系统：API ≤ 29（含 legacy flag）或已授予「所有文件访问」时可用；
 * 2. MediaStore 查询：API ≥ 29 的兜底（adb push 的文件经媒体库索引后可见）；
 * 3. 两者都拿不到时，界面提供「授权文件访问」入口（MANAGE_EXTERNAL_STORAGE，
 *    伴侣 App 是侧载工具类应用，不做商店分发；由用户在系统设置里明确授予）。
 */
object ReceivedFiles {

    const val DIR_NAME = "MirrorDock"

    data class Entry(
        val name: String,
        val size: Long,
        val modifiedAt: Long,
        val uri: Uri,
        val isApk: Boolean,
    )

    fun dir(): File =
        File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DOWNLOADS), DIR_NAME)

    /** 是否具备直接列目录的能力（决定空状态里是否提示去授权）。 */
    fun canReadDirectly(): Boolean = dir().let { it.isDirectory && it.list() != null }

    /** 是否已持有「所有文件访问」（API 30+ 专用判定）。 */
    fun hasAllFilesAccess(): Boolean =
        Build.VERSION.SDK_INT < 30 || Environment.isExternalStorageManager()

    fun list(context: Context): List<Entry> {
        val out = mutableListOf<Entry>()
        val d = dir()
        val files = runCatching { d.listFiles()?.filter { it.isFile } }.getOrNull().orEmpty()
        for (f in files) {
            out += Entry(f.name, f.length(), f.lastModified(), fileUri(context, f), f.name.endsWith(".apk", true))
        }
        if (out.isEmpty() && Build.VERSION.SDK_INT >= 29) out += listViaMediaStore(context)
        return out.sortedByDescending { it.modifiedAt }
    }

    private fun listViaMediaStore(context: Context): List<Entry> {
        val out = mutableListOf<Entry>()
        val collection = MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)
        val projection = arrayOf(
            MediaStore.MediaColumns._ID,
            MediaStore.MediaColumns.DISPLAY_NAME,
            MediaStore.MediaColumns.SIZE,
            MediaStore.MediaColumns.DATE_MODIFIED,
        )
        val selection = "${MediaStore.MediaColumns.RELATIVE_PATH} LIKE ?"
        val args = arrayOf("Download/$DIR_NAME%")
        runCatching {
            context.contentResolver.query(collection, projection, selection, args, null)?.use { c ->
                val idCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns._ID)
                val nameCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DISPLAY_NAME)
                val sizeCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.SIZE)
                val dateCol = c.getColumnIndexOrThrow(MediaStore.MediaColumns.DATE_MODIFIED)
                while (c.moveToNext()) {
                    val name = c.getString(nameCol) ?: continue
                    out += Entry(
                        name = name,
                        size = c.getLong(sizeCol),
                        modifiedAt = c.getLong(dateCol) * 1000,
                        uri = ContentUris.withAppendedId(collection, c.getLong(idCol)),
                        isApk = name.endsWith(".apk", true),
                    )
                }
            }
        }
        return out
    }

    fun fileUri(context: Context, file: File): Uri =
        FileProvider.getUriForFile(context, "${context.packageName}.files", file)

    /** 打开文件；APK 走系统安装器（需用户允许「安装未知应用」）。 */
    fun open(context: Context, entry: Entry): Boolean {
        val mime = if (entry.isApk) "application/vnd.android.package-archive" else mimeOf(entry.name)
        val intent = Intent(Intent.ACTION_VIEW).setDataAndType(entry.uri, mime)
        intent.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_ACTIVITY_NEW_TASK)
        return runCatching { context.startActivity(intent); true }.isSuccess
    }

    /** 「所有文件访问」的系统设置入口（API 30+；低版本不需要也不可用）。 */
    fun allFilesAccessIntent(context: Context): Intent? {
        if (Build.VERSION.SDK_INT < 30) return null
        val specific = Intent(
            Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
            Uri.parse("package:${context.packageName}"),
        )
        return specific.takeIf { it.resolveActivity(context.packageManager) != null }
            ?: Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION)
    }

    private fun mimeOf(name: String): String =
        MimeTypeMap.getSingleton().getMimeTypeFromExtension(name.substringAfterLast('.', "").lowercase())
            ?: "application/octet-stream"
}
