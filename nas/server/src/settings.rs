//! Web 可视化设置：画质预设与归档保留天数。
//!
//! 【为什么需要】
//! 此前画质（录制/镜像）与保留天数只能靠环境变量，改一次要重建容器——
//! 与「NAS 常开、浏览器远程管理」的立项场景相悖。这里把两项设置落到
//! 数据目录的 settings.json，Web 改完即生效，无需重启、无需 SSH。
//!
//! 【优先级】
//! 环境变量是「部署者兜底」：若 MIRRORDOCK_RETENTION_DAYS 被显式设置，保留天数以
//! env 为准（Web 修改会被忽略并在接口里标注），避免运维在 compose 里钉死的值
//! 被网页访客悄悄改掉。画质没有 env 历史包袱，直接以 settings.json 为准。
//!
//! 【fail-safe】
//! settings.json 损坏/缺字段 → 回落到默认值（Balanced / 默认保留天数），
//! 绝不因配置文件坏了让服务起不来。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

use crate::record::Quality;
use crate::retention;

/// 设置文件名（位于数据目录下）。
const FILE_NAME: &str = "settings.json";

/// 画质预设。与 record::Quality 同名同义，序列化为小写串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QualityPreset {
    Smooth,
    #[default]
    Balanced,
    Sharp,
}

impl From<QualityPreset> for Quality {
    fn from(p: QualityPreset) -> Self {
        match p {
            QualityPreset::Smooth => Quality::Smooth,
            QualityPreset::Balanced => Quality::Balanced,
            QualityPreset::Sharp => Quality::Sharp,
        }
    }
}

impl QualityPreset {
    /// 镜像用的 max_size（与 record::Quality::profile 同值的第一项）。
    pub fn max_size(self) -> u32 {
        match self {
            QualityPreset::Smooth => 1024,
            QualityPreset::Balanced => 1920,
            QualityPreset::Sharp => 2560,
        }
    }
}

/// 落盘的可变设置。
///
/// `retention_days` 为 None 表示「跟随环境变量/默认值」——只在用户于 Web 上
/// 显式改过之后才写入具体值。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct FileSettings {
    quality: Option<QualityPreset>,
    retention_days: Option<u64>,
}

/// 对外暴露的完整设置视图（含生效值与来源标注）。
#[derive(Debug, Clone, Serialize)]
pub struct SettingsView {
    /// 生效画质。
    pub quality: QualityPreset,
    /// 生效保留天数。
    pub retention_days: u64,
    /// 保留天数是否被环境变量钉死（true 时 Web 修改无效）。
    pub retention_locked_by_env: bool,
}

/// 全局设置句柄：内存缓存 + 落盘持久化。
pub struct Settings {
    path: PathBuf,
    inner: RwLock<FileSettings>,
    /// 启动时记录 env 是否显式设置了保留天数。
    retention_locked_by_env: bool,
    /// env 解析出的保留天数（locked 时作为生效值）。
    env_retention_days: u64,
}

impl Settings {
    /// 加载（不存在则默认）。retention_days() 的 env 解析在此完成一次。
    pub async fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(FILE_NAME);
        let inner = match tokio::fs::read_to_string(&path).await {
            Ok(text) => serde_json::from_str::<FileSettings>(&text).unwrap_or_else(|e| {
                eprintln!("[设置] settings.json 解析失败（{e}），已按默认设置继续。");
                FileSettings::default()
            }),
            Err(_) => FileSettings::default(),
        };
        Self {
            path,
            inner: RwLock::new(inner),
            retention_locked_by_env: std::env::var("MIRRORDOCK_RETENTION_DAYS").is_ok(),
            env_retention_days: retention::retention_days(),
        }
    }

    /// 生效画质。
    pub async fn quality(&self) -> QualityPreset {
        self.inner.read().await.quality.unwrap_or_default()
    }

    /// 生效保留天数：env 锁定 > Web 设置 > 默认。
    pub async fn retention_days(&self) -> u64 {
        if self.retention_locked_by_env {
            return self.env_retention_days;
        }
        self.inner
            .read()
            .await
            .retention_days
            .unwrap_or(self.env_retention_days)
    }

    /// 完整视图（给 /api/settings GET）。
    pub async fn view(&self) -> SettingsView {
        SettingsView {
            quality: self.quality().await,
            retention_days: self.retention_days().await,
            retention_locked_by_env: self.retention_locked_by_env,
        }
    }

    /// Web 提交的修改。返回修改后的完整视图。
    ///
    /// 仅更新传入的字段；retention 被 env 锁定时忽略该字段（但不报错，
    /// 由视图里的 locked 标记让前端提示）。
    pub async fn update(
        &self,
        quality: Option<QualityPreset>,
        retention_days: Option<u64>,
    ) -> SettingsView {
        {
            let mut w = self.inner.write().await;
            if let Some(q) = quality {
                w.quality = Some(q);
            }
            if retention_days.is_some() && !self.retention_locked_by_env {
                w.retention_days = retention_days;
            }
        }
        self.persist().await;
        self.view().await
    }

    /// 原子写盘：先写临时文件再 rename，避免中途断电留下半个 JSON。
    async fn persist(&self) {
        let snapshot = self.inner.read().await.clone();
        let text = match serde_json::to_string_pretty(&snapshot) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[设置] 序列化失败：{e}");
                return;
            }
        };
        let tmp = self.path.with_extension("json.tmp");
        match tokio::fs::write(&tmp, text).await {
            Ok(()) => {
                if let Err(e) = tokio::fs::rename(&tmp, &self.path).await {
                    eprintln!("[设置] 写入 settings.json 失败：{e}");
                }
            }
            Err(e) => eprintln!("[设置] 写入临时文件失败：{e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loads_defaults_when_file_missing() {
        let dir = std::env::temp_dir().join(format!("md-settings-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let s = Settings::load(&dir).await;
        assert_eq!(s.quality().await, QualityPreset::Balanced);
        // env 未设时 retention 跟随 retention::retention_days() 的默认 7。
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn update_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("md-settings-rw-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let s = Settings::load(&dir).await;
        s.update(Some(QualityPreset::Sharp), Some(30)).await;
        drop(s);
        // 重新加载：画质与保留天数都应当还在（retention 未被 env 锁定的前提下）。
        let s2 = Settings::load(&dir).await;
        assert_eq!(s2.quality().await, QualityPreset::Sharp);
        if std::env::var("MIRRORDOCK_RETENTION_DAYS").is_err() {
            assert_eq!(s2.retention_days().await, 30);
        }
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn corrupt_file_falls_back_to_default() {
        let dir = std::env::temp_dir().join(format!("md-settings-bad-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join(FILE_NAME), "{oops not json")
            .await
            .unwrap();
        let s = Settings::load(&dir).await;
        assert_eq!(s.quality().await, QualityPreset::Balanced);
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[test]
    fn preset_max_size_matches_record_profile() {
        assert_eq!(QualityPreset::Smooth.max_size(), 1024);
        assert_eq!(QualityPreset::Balanced.max_size(), 1920);
        assert_eq!(QualityPreset::Sharp.max_size(), 2560);
    }
}
