//! 归档保留策略：按天数自动删除最旧的录像（以及截图）。
//!
//! 【为什么必须有】
//! 立项场景是「24h 录制归档」——设备长期常开、人不在场。没有保留策略时录像
//! 只增不减，最终一定写满磁盘；而写满之后的表现不是「报错」，是 scrcpy 写出
//! 半个 MP4、adbd 推送失败这类离根因很远的故障。按天回收是让这套系统能真正
//! 无人值守跑下去的前提。
//!
//! 【为什么默认开启且默认 7 天】
//! 归档型场景里「保留最近 N 天」是绝大多数人的真实意图，而不是「永久保存」。
//! 默认关闭等于把「磁盘终将写满」这个必然结局留给用户自己发现。7 天是给
//! 回看留出足够窗口，同时把占用控制在可预期范围内。
//!
//! 【安全边界】
//! - 解析不出合法天数 → **不清理**（误删不可逆，磁盘满只是可恢复的告警）
//! - 只删指定扩展名（mp4 / png）的普通文件，不进子目录、不碰其他文件
//! - 单个文件删除失败只记录不中断，不会因一个坏文件卡住整轮清理

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// 默认保留天数。可用 MIRRORDOCK_RETENTION_DAYS 覆盖，0 表示不清理。
pub const DEFAULT_RETENTION_DAYS: u64 = 7;

/// 清理周期。归档是天级行为，没必要分钟级扫描。
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// 读取配置的保留天数。
///
/// **解析失败一律返回 0（不清理）**：这是刻意的取向——清错文件不可逆，
/// 而磁盘写满是可观测、可恢复的告警。宁可少删，不可错删。
/// 环境变量写成 `7d`、`七天` 这类非法值时会走到这条路径，并在启动日志里喊出来。
pub fn retention_days() -> u64 {
    match std::env::var("MIRRORDOCK_RETENTION_DAYS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(days) => days,
            Err(_) => {
                eprintln!(
                    "[警告] MIRRORDOCK_RETENTION_DAYS={raw:?} 不是合法天数，已按「不清理」处理。\
                     请填非负整数（0=不清理，例：7）。"
                );
                0
            }
        },
        Err(_) => DEFAULT_RETENTION_DAYS,
    }
}

/// 单轮清理的结果。启动时与每轮周期任务都会打印，便于在容器日志里追溯。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    pub scanned: usize,
    pub deleted: usize,
    pub freed_bytes: u64,
    /// 删除失败的条目（路径 + 原因）。失败不中断整轮清理。
    pub failed: Vec<String>,
}

impl Report {
    /// 是否无事发生——用于决定要不要往日志里写一行，避免每 6 小时刷屏。
    pub fn is_empty(&self) -> bool {
        self.deleted == 0 && self.failed.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "扫描 {} 个归档，删除 {} 个，释放 {:.1} MB{}",
            self.scanned,
            self.deleted,
            self.freed_bytes as f64 / 1024.0 / 1024.0,
            if self.failed.is_empty() {
                String::new()
            } else {
                format!(
                    "，{} 个删除失败：{}",
                    self.failed.len(),
                    self.failed.join("; ")
                )
            }
        )
    }
}

/// 按天回收 `data_dir` 下 recordings/ 与 screenshots/ 里的过期文件。
///
/// `days == 0` 时直接返回空报告（保留全部）——这与 retention_days() 的
/// fail-safe 取向一致。
pub fn sweep(data_dir: &Path, days: u64, now: SystemTime) -> Report {
    let mut report = Report::default();
    if days == 0 {
        return report;
    }
    let cutoff = match now.checked_sub(Duration::from_secs(days * 24 * 60 * 60)) {
        Some(t) => t,
        // 系统时间早于保留窗口（时钟异常）→ 不清理，避免误判成全部过期。
        None => return report,
    };

    for (sub, ext) in [("recordings", "mp4"), ("screenshots", "png")] {
        sweep_dir(&data_dir.join(sub), ext, cutoff, &mut report);
    }
    report
}

/// 只处理一层目录下的指定扩展名——不递归子目录，避免误伤用户自己放进去的东西。
fn sweep_dir(dir: &Path, ext: &str, cutoff: SystemTime, report: &mut Report) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        // 目录不存在（还没录过/没截过图）是正常状态，不是错误。
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(ext) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) if m.is_file() => m,
            _ => continue,
        };
        report.scanned += 1;
        let modified = match meta.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if modified >= cutoff {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => {
                report.deleted += 1;
                report.freed_bytes += meta.len();
            }
            Err(e) => report.failed.push(format!("{}（{e}）", path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn make_file(dir: &Path, name: &str, age_days: u64, now: SystemTime) -> u64 {
        let path = dir.join(name);
        let mut f = fs::File::create(&path).expect("创建测试文件");
        f.write_all(b"x".repeat(1024).as_slice()).expect("写入");
        f.flush().expect("flush");
        let mtime = now - Duration::from_secs(age_days * 24 * 60 * 60);
        f.set_modified(mtime).expect("设置 mtime");
        drop(f);
        1024
    }

    #[test]
    fn deletes_only_files_older_than_cutoff() {
        let tmp = std::env::temp_dir().join(format!("md-retention-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("recordings")).unwrap();
        let now = SystemTime::now();

        make_file(&tmp.join("recordings"), "old-1.mp4", 10, now);
        make_file(&tmp.join("recordings"), "old-2.mp4", 8, now);
        make_file(&tmp.join("recordings"), "fresh.mp4", 1, now);

        let report = sweep(&tmp, 7, now);
        assert_eq!(report.deleted, 2, "应只删超过 7 天的两个：{report:?}");
        assert!(report.failed.is_empty());
        assert!(tmp.join("recordings/fresh.mp4").exists(), "1 天的必须保留");

        let _ = fs::remove_dir_all(&tmp);
    }

    /// 边界：正好等于保留天数的文件不算过期（保留窗口是「最近 N 天内」）。
    #[test]
    fn boundary_day_is_kept() {
        let tmp = std::env::temp_dir().join(format!("md-retention-b-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("recordings")).unwrap();
        let now = SystemTime::now();
        make_file(&tmp.join("recordings"), "exact.mp4", 7, now);

        let report = sweep(&tmp, 7, now);
        assert_eq!(report.deleted, 0, "恰好 7 天应保留：{report:?}");

        let _ = fs::remove_dir_all(&tmp);
    }

    /// 0 = 不清理。这是显式关闭的语义，必须恒成立。
    #[test]
    fn zero_days_keeps_everything() {
        let tmp = std::env::temp_dir().join(format!("md-retention-z-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("recordings")).unwrap();
        let now = SystemTime::now();
        make_file(&tmp.join("recordings"), "ancient.mp4", 365, now);

        let report = sweep(&tmp, 0, now);
        assert_eq!(report.deleted, 0);
        assert!(tmp.join("recordings/ancient.mp4").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    /// 只认扩展名：同目录里的其他文件（含用户自己放的）一律不动。
    #[test]
    fn ignores_other_extensions_and_subdirs() {
        let tmp = std::env::temp_dir().join(format!("md-retention-x-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("recordings/nested")).unwrap();
        let now = SystemTime::now();
        make_file(&tmp.join("recordings"), "notes.txt", 100, now);
        make_file(&tmp.join("recordings/nested"), "deep.mp4", 100, now);

        let report = sweep(&tmp, 7, now);
        assert_eq!(report.deleted, 0, "非 mp4 与子目录内的都不该动：{report:?}");
        assert!(tmp.join("recordings/notes.txt").exists());
        assert!(tmp.join("recordings/nested/deep.mp4").exists());

        let _ = fs::remove_dir_all(&tmp);
    }

    /// 截图目录同样受保留策略管辖——长时间运行下它涨得比录像还快。
    #[test]
    fn sweeps_screenshots_too() {
        let tmp = std::env::temp_dir().join(format!("md-retention-s-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("screenshots")).unwrap();
        let now = SystemTime::now();
        make_file(&tmp.join("screenshots"), "old.png", 30, now);

        let report = sweep(&tmp, 7, now);
        assert_eq!(report.deleted, 1);
        assert!(!tmp.join("screenshots/old.png").exists());

        let _ = fs::remove_dir_all(&tmp);
    }
}
