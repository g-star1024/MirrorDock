//! 录制管理（headless scrcpy）。
//!
//! 容器里没有显示器，所以用的是 scrcpy 的**无窗录制**通道（X10-92 双通道方案
//! 在桌面端是这么做的）：`--no-window --no-playback --record=...`。这条路在
//! 桌面端已经被验证过——正是它让「录制不再因为开关窗口而重启会话」。

use chrono::Local;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::{Child, Command};

/// 录制参数与桌面端 record_arguments 保持一致（max-size / 码率 / h264）。
/// 注意这里是有意的参数对齐而非代码复用：SessionOptions 深埋在桌面端 lib.rs 里，
/// 抽出它需要连带搬动大量界面相关的默认值；NAS 端只需这三个固定值，
/// 保持取值一致即可保证两端录像规格相同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quality {
    Smooth,
    Balanced,
    Sharp,
}

impl Quality {
    /// 返回 (max_size, bit_rate)。与桌面端 SessionOptions::record_arguments 同值。
    fn profile(self) -> (u32, &'static str) {
        match self {
            Quality::Smooth => (1024, "2M"),
            Quality::Balanced => (1920, "8M"),
            Quality::Sharp => (2560, "16M"),
        }
    }
}

/// 生成录像文件名：`设备短名-YYYYMMDD-HHMMSS.mp4`。
///
/// 序列号里的 `:`（无线端点）不能出现在文件名里——Windows/SMB 分享场景下会
/// 直接拒绝，故统一替换掉。
pub fn recording_name(serial: &str, now: &str) -> String {
    let safe: String = serial
        .chars()
        .map(|c| if c == ':' || c == '/' { '-' } else { c })
        .collect();
    format!("{safe}-{now}.mp4")
}

/// 当前时间戳片段，独立成函数便于测试注入。
pub fn now_stamp() -> String {
    Local::now().format("%Y%m%d-%H%M%S").to_string()
}

pub struct Recording {
    pub path: PathBuf,
    child: Option<Child>,
}

impl Recording {
    pub async fn start(
        serial: &str,
        dir: &Path,
        quality: Quality,
        scrcpy: &str,
        adb: &str,
    ) -> Result<Self, String> {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| format!("无法创建录像目录：{e}"))?;
        let path = dir.join(recording_name(serial, &now_stamp()));
        let (max_size, bit_rate) = quality.profile();

        let mut command = Command::new(scrcpy);
        command
            .env("ADB", adb)
            .arg("--serial")
            .arg(serial)
            .arg("--no-window")
            .arg("--no-playback")
            .arg("--record-format=mp4")
            .arg(format!("--record={}", path.display()))
            .arg(format!("--max-size={max_size}"))
            .arg(format!("--video-bit-rate={bit_rate}"))
            .arg("--video-codec=h264")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // X10-95（桌面端血的教训）：父进程可能继承「忽略 SIGINT」的信号处置，
        // 而 scrcpy 写出 MP4 收尾索引（moov）只在走 SIGINT 正常退出路径时执行。
        // 不在 pre_exec 里把 disposition 重置为默认，录制文件就会缺 moov 变砖。
        // 这条经验必须跟着 scrcpy 录制一起走，所以 NAS 端照做。
        #[cfg(unix)]
        {
            unsafe {
                command.pre_exec(|| {
                    libc_reset_signal();
                    Ok(())
                });
            }
        }

        let child = command
            .spawn()
            .map_err(|e| format!("无法启动 scrcpy 录制（路径 {scrcpy}）：{e}"))?;
        Ok(Self { path, child: Some(child) })
    }

    /// 停止录制。**必须走 SIGINT 优雅退出**，见上方 X10-95 说明。
    pub async fn stop(&mut self) -> Result<(), String> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        if let Some(pid) = child.id() {
            // 用外部 kill 发 INT：tokio 的 Child 没有直接的 signal 接口。
            let _ = Command::new("kill")
                .args(["-INT", &pid.to_string()])
                .status()
                .await;
        }
        match tokio::time::timeout(std::time::Duration::from_secs(15), child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("等待录制进程退出失败：{e}")),
            // 超时才动用 SIGKILL——此时 moov 已经写不出来，只能如实告知。
            Err(_) => {
                let _ = child.kill().await;
                Err("录制进程未能在 15 秒内优雅退出，已强制结束（文件可能不完整）。".into())
            }
        }
    }

}

#[cfg(unix)]
fn libc_reset_signal() {
    // SAFETY: pre_exec 只允许调用 async-signal-safe 函数；signal() 属于此类。
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_DFL);
        libc::signal(libc::SIGTERM, libc::SIG_DFL);
        libc::signal(libc::SIGHUP, libc::SIG_DFL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_name_uses_timestamp_and_serial() {
        assert_eq!(recording_name("ABC123", "20261011-010000"),
                   "ABC123-20261011-010000.mp4");
    }

    /// 无线端点里的冒号必须替换：冒号在 SMB/Windows 分享上是非法文件名字符。
    #[test]
    fn recording_name_sanitizes_wireless_colon() {
        let name = recording_name("192.168.1.9:5555", "20261011-010000");
        assert!(!name.contains(':'), "冒号会导致 SMB/Windows 分享失败：{name}");
        assert!(name.ends_with(".mp4"));
    }

    #[test]
    fn quality_profiles_match_desktop_defaults() {
        assert_eq!(Quality::Smooth.profile(), (1024, "2M"));
        assert_eq!(Quality::Balanced.profile(), (1920, "8M"));
        assert_eq!(Quality::Sharp.profile(), (2560, "16M"));
    }
}
