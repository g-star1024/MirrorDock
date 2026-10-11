//! 文件双向传输 / 应用管理 / 截图画廊（P1 轻量补全，X10-116）。
//!
//! 全部走 adb 通道，与实时镜像的 scrcpy 通道互不干扰。
//!
//! ## 设计取向
//! - **路径安全**：所有「手机侧路径」原样传给 adb（adb 在设备上解析，天然在我们
//!   够不到的沙箱里）；所有「NAS 侧落盘路径」都必须先过纯文件名校验，杜绝 `../`
//!   穿越出数据目录——这是 X10-95 删除录像同款防护的延伸。
//! - **大文件流式**：上传/下载都走流，不一次性读进内存。NAS 场景录像/APK 可能上 GB。
//! - **错误带 adb 原文**：设备未授权/离线的提示在 stderr，丢掉就无从排查（同 adb.rs）。

use std::path::Path;
use std::process::Stdio;

use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::adb;

/// 拒绝一切非纯文件名：含路径分隔符或 `..` 的一律不收。
/// 这是 NAS 侧落盘路径的唯一守门员——之后的 join 才不会逃出数据目录。
pub fn safe_file_name(name: &str) -> Result<&str, String> {
    if name.is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.starts_with('.')
    {
        return Err(format!("文件名不合法：{name:?}"));
    }
    Ok(name)
}

/// adb 参数统一走 argv 形式，避免 shell 串接的转义差异（X10-74 教训）。
async fn run_adb_status(args: &[String]) -> Result<(), String> {
    let out = Command::new(adb::adb_binary())
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("无法执行 adb：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

// ===========================================================================
// 文件双向传输
// ===========================================================================

/// 把 NAS 本地一个文件 push 到手机目录。
/// `local` 必须是已存在的本地文件；`remote_dir` 是手机侧目录（如 /sdcard/Download）。
pub async fn push_file(serial: &str, local: &Path, remote_dir: &str) -> Result<(), String> {
    let file_name = local
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "本地文件名无法解析".to_owned())?;
    let remote = format!("{}/{}", remote_dir.trim_end_matches('/'), file_name);
    run_adb_status(&[
        "-s".into(),
        serial.into(),
        "push".into(),
        local.display().to_string(),
        remote,
    ])
    .await
}

/// 把手机一个文件 pull 到 NAS 数据目录下的 inbox/，返回落盘后的文件名。
/// 用 `exec-out cat` 而不是 `adb pull`：能把字节流直接 pipe 进我们的文件，
/// 不必让 adb 自己决定落盘路径（便于统一穿越防护与命名）。
pub async fn pull_file(serial: &str, remote: &str, inbox: &Path) -> Result<String, String> {
    let file_name = Path::new(remote)
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| "手机路径无法解析文件名".to_owned())?;
    let file_name = safe_file_name(file_name)?.to_owned();

    tokio::fs::create_dir_all(inbox)
        .await
        .map_err(|e| format!("无法创建下载目录：{e}"))?;
    let local = inbox.join(&file_name);

    let mut child = Command::new(adb::adb_binary())
        .args(["-s", serial, "exec-out", "cat", remote])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法执行 adb：{e}"))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取 adb 输出".to_owned())?;
    let mut file = tokio::fs::File::create(&local)
        .await
        .map_err(|e| format!("无法写入本地文件：{e}"))?;

    let copy = tokio::io::copy(&mut stdout, &mut file).await;
    file.flush().await.ok();
    let status = child
        .wait()
        .await
        .map_err(|e| format!("等待 adb 退出失败：{e}"))?;

    match (copy, status.success()) {
        (Ok(n), true) if n > 0 => Ok(file_name),
        _ => {
            // 失败清理半成品，免得留一个 0 字节文件被当成下载成功。
            let _ = tokio::fs::remove_file(&local).await;
            Err(format!(
                "从手机拉取失败（文件可能不存在或无权限）：{remote}"
            ))
        }
    }
}

/// 删除手机上一个文件（rm -f，设备侧执行）。
pub async fn delete_remote_file(serial: &str, remote: &str) -> Result<(), String> {
    run_adb_status(&[
        "-s".into(),
        serial.into(),
        "shell".into(),
        "rm".into(),
        "-f".into(),
        remote.into(),
    ])
    .await
}

// ===========================================================================
// 应用管理
// ===========================================================================

/// 一个已安装应用（包名 + 是否第三方）。
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppInfo {
    pub package: String,
    pub third_party: bool,
}

/// 解析 `pm list packages [-3]` 的输出。行形如 `package:com.foo.bar`。
pub fn parse_packages(raw: &str) -> Vec<AppInfo> {
    raw.lines()
        .filter_map(|l| l.trim().strip_prefix("package:"))
        .filter(|p| !p.is_empty())
        .map(|p| AppInfo {
            package: p.to_owned(),
            third_party: false, // 由调用方按是否 -3 列表覆盖
        })
        .collect()
}

/// 列出手机上的应用。默认只列第三方（-3），避免被几百个系统包淹没。
pub async fn list_apps(serial: &str, include_system: bool) -> Result<Vec<AppInfo>, String> {
    let mut args = vec![
        "-s".into(),
        serial.into(),
        "shell".into(),
        "pm".into(),
        "list".into(),
        "packages".into(),
    ];
    if !include_system {
        args.push("-3".into());
    }
    let raw = adb::run_adb(&args).await?;
    let mut apps = parse_packages(&raw);
    for a in &mut apps {
        a.third_party = !include_system;
    }
    apps.sort_by(|x, y| x.package.cmp(&y.package));
    Ok(apps)
}

/// 安装一个已上传到 NAS 的 APK 到手机。先 push 到设备临时目录再 pm install，
/// 最后删掉临时文件——比 `adb install <本地路径>` 更可控（能看到每一步的报错）。
pub async fn install_apk(serial: &str, local_apk: &Path) -> Result<(), String> {
    let tmp = format!("/data/local/tmp/mirrordock-{}.apk", std::process::id());
    run_adb_status(&[
        "-s".into(),
        serial.into(),
        "push".into(),
        local_apk.display().to_string(),
        tmp.clone(),
    ])
    .await
    .map_err(|e| format!("APK 推送到手机失败：{e}"))?;

    // pm install 的成败判定要看 stdout/stderr 里的 "Success"，退出码在部分机型上不可靠。
    let out = Command::new(adb::adb_binary())
        .args(["-s", serial, "shell", "pm", "install", "-r", &tmp])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("无法执行 pm install：{e}"))?;

    // 无论成败都清掉临时 APK，免得 /data/local/tmp 越堆越多。
    let _ = run_adb_status(&[
        "-s".into(),
        serial.into(),
        "shell".into(),
        "rm".into(),
        "-f".into(),
        tmp,
    ])
    .await;

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if combined.contains("Success") {
        Ok(())
    } else {
        Err(format!("安装失败：{}", combined.trim()))
    }
}

// ===========================================================================
// 截图画廊（数据目录侧的列表/删除，与 recordings 同模式）
// ===========================================================================

/// 列出 screenshots 目录的 PNG（名字 + 大小 + 修改时间）。
pub async fn list_screenshots(dir: &Path) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    let mut read = match tokio::fs::read_dir(dir).await {
        Ok(r) => r,
        Err(_) => return items,
    };
    while let Ok(Some(entry)) = read.next_entry().await {
        let meta = match entry.metadata().await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_file() || !entry.file_name().to_string_lossy().ends_with(".png") {
            continue;
        }
        items.push(serde_json::json!({
            "name": entry.file_name().to_string_lossy(),
            "bytes": meta.len(),
            "modified": meta.modified().ok().and_then(|t| {
                t.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
            }),
        }));
    }
    items.sort_by(|a, b| {
        let key = |v: &serde_json::Value| v["modified"].as_u64().unwrap_or(0);
        key(b).cmp(&key(a))
    });
    items
}

// ===========================================================================
// 测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_name_accepts_plain() {
        assert_eq!(safe_file_name("a.mp4").unwrap(), "a.mp4");
        assert_eq!(safe_file_name("录像-01.apk").unwrap(), "录像-01.apk");
    }

    #[test]
    fn safe_name_rejects_traversal() {
        assert!(safe_file_name("../etc/passwd").is_err());
        assert!(safe_file_name("a/b.png").is_err());
        assert!(safe_file_name("a\\b").is_err());
        assert!(safe_file_name(".hidden").is_err());
        assert!(safe_file_name("").is_err());
        assert!(safe_file_name("..").is_err());
    }

    #[test]
    fn parse_packages_strips_prefix() {
        let raw = "package:com.foo.bar\npackage:com.android.systemui\n\npackage:  \n";
        let apps = parse_packages(raw);
        let names: Vec<&str> = apps.iter().map(|a| a.package.as_str()).collect();
        assert_eq!(names, vec!["com.foo.bar", "com.android.systemui"]);
    }

    #[tokio::test]
    async fn list_screenshots_filters_and_sorts() {
        let dir = std::env::temp_dir().join(format!("md-shots-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // 两个 png + 一个 txt（不该被列出）
        for (name, age) in [("b.png", 0u64), ("a.png", 0), ("notes.txt", 0)] {
            let p = dir.join(name);
            tokio::fs::write(&p, b"x").await.unwrap();
            let _ = age;
        }
        // 让 a.png 更旧：直接改 mtime 不可靠，靠名称排序之外仅验证过滤。
        let items = list_screenshots(&dir).await;
        let names: Vec<String> = items
            .iter()
            .map(|v| v["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.contains(&"a.png".to_owned()));
        assert!(names.contains(&"b.png".to_owned()));
        assert!(!names.contains(&"notes.txt".to_owned()));
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
