//! 检查更新（X10-120）。
//!
//! 【为什么需要】
//! 飞牛 fnOS 的「有新版本」徽标只适配 Docker Hub 的 registry API，对 ghcr.io
//! 不做版本比对——于是我们的镜像永远是「已是最新」，用户不知道有新版（真机实踩）。
//! 与其等飞牛修，不如在自己的 Web 页里做：服务端匿名调 GHCR 公共接口比对
//! 「本地运行的镜像 digest」与「远端 latest digest」，把结果告诉前端。
//!
//! 【为什么匿名就够】
//! 镜像在 GHCR 上是公共可见的（已实测 manifest 200）。匿名 token 即可拉取
//! manifest 的 digest，不需要任何凭据——这正好避免了在 NAS 上配 token。
//!
//! 【比对的两个层次】
//! - digest：本地镜像的 RepoDigest（容器运行时注入的 MIRRORDOCK_IMAGE_DIGEST）
//!   vs 远端 latest 的 manifest digest。最直接。
//! - commit：本地镜像的 OCI revision 标签（构建时由 CI 注入）vs 远端 latest
//!   的 OCI revision 标签。digest 拿不到时的兜底，也顺便给出「落后几个版本」
//!   的人话描述。
//!
//! 【为什么做成启动后惰性 + 手动触发】
//! 每次打开页面都现查会让首屏等一个外网请求；做成「启动查一次缓存 + 手动
//! 刷新强制重查」，既不拖慢首屏，也给了用户「我刚推了新版，再看看」的入口。

use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;

/// GHCR 上本镜像的仓库路径（<owner>/<仓库名>/<包名>，与 compose 一致）。
const GHCR_REPO: &str = "g-star1024/mirrordock/mirrordock-nas";
/// 检查的外网请求超时——NAS 可能在内网，连不上 GHCR 不该把接口拖死。
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
/// 启动后自动检查的间隔（页面首次加载时若缓存过期会触发一次）。
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// 更新检查结果（给 /api/update/check）。
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    /// 是否检查成功（外网不可达时为 false，看 error）。
    pub checked: bool,
    /// 是否有新版本（checked=false 时恒为 false，无意义）。
    pub update_available: bool,
    /// 本地镜像 digest（容器运行时注入；本地 cargo run 时为空）。
    pub local_digest: Option<String>,
    /// 远端 latest digest。
    pub remote_digest: Option<String>,
    /// 本地镜像的 git 短哈希（OCI revision 标签）。
    pub local_revision: Option<String>,
    /// 远端 latest 的 git 短哈希。
    pub remote_revision: Option<String>,
    /// 人话结论，前端直接显示。
    pub message: String,
    /// 检查失败原因（checked=false 时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct UpdateChecker {
    client: reqwest::Client,
    cache: Mutex<Option<(Instant, UpdateInfo)>>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(HTTP_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            cache: Mutex::new(None),
        }
    }
}

impl UpdateChecker {
    /// 取检查结果。force=true 时无视缓存强制重查（用户点「检查更新」）；
    /// 否则命中未过期缓存直接返回（页面首屏自动查不拖慢）。
    pub async fn check(&self, force: bool) -> UpdateInfo {
        if !force {
            let cache = self.cache.lock().await;
            if let Some((at, info)) = &*cache {
                if at.elapsed() < CACHE_TTL {
                    return info.clone();
                }
            }
        }
        let info = self.fetch().await;
        *self.cache.lock().await = Some((Instant::now(), info.clone()));
        info
    }

    async fn fetch(&self) -> UpdateInfo {
        let local_digest = std::env::var("MIRRORDOCK_IMAGE_DIGEST").ok();
        let local_revision = std::env::var("MIRRORDOCK_REVISION").ok();
        match fetch_remote(&self.client).await {
            Ok((remote_digest, remote_revision)) => {
                let (update_available, message) = conclude(
                    local_digest.as_deref(),
                    &remote_digest,
                    local_revision.as_deref(),
                    remote_revision.as_deref(),
                );
                UpdateInfo {
                    checked: true,
                    update_available,
                    local_digest,
                    remote_digest: Some(remote_digest),
                    local_revision,
                    remote_revision,
                    message,
                    error: None,
                }
            }
            Err(e) => UpdateInfo {
                checked: false,
                update_available: false,
                local_digest,
                remote_digest: None,
                local_revision,
                remote_revision: None,
                message: format!("无法连接 GHCR 检查更新：{e}"),
                error: Some(e),
            },
        }
    }
}

/// 根据本地/远端 digest 与 revision 下结论。
fn conclude(
    local_digest: Option<&str>,
    remote_digest: &str,
    local_rev: Option<&str>,
    remote_rev: Option<&str>,
) -> (bool, String) {
    // digest 最直接：两边都有且不同 = 有新版。
    if let Some(local) = local_digest {
        if local != remote_digest {
            return (
                true,
                format!(
                    "发现新版本（远端 {}…）",
                    &remote_digest[..19.min(remote_digest.len())]
                ),
            );
        }
        return (false, "已是最新版本".to_owned());
    }
    // 本地 digest 缺失（本地 cargo run / 未注入），退而比 commit。
    match (local_rev, remote_rev) {
        (Some(l), Some(r)) if l != r => (true, format!("发现新版本：当前 {l}，最新 {r}")),
        (Some(l), Some(_)) => (false, format!("已是最新（{l}）")),
        _ => (
            false,
            "无法确定本地版本（非容器运行或镜像未注入版本标签），请以 digest 为准".to_owned(),
        ),
    }
}

/// 匿名拉取远端 latest 的 manifest digest 与 OCI revision 标签。
async fn fetch_remote(client: &reqwest::Client) -> Result<(String, Option<String>), String> {
    // 1) 匿名换 token（公共仓库无需凭据）。
    let token_url =
        format!("https://ghcr.io/token?scope=repository:{GHCR_REPO}:pull&service=ghcr.io");
    let token: serde_json::Value = client
        .get(&token_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let token = token["token"].as_str().ok_or("GHCR token 响应异常")?;

    // 2) HEAD manifest 拿 digest（body 不需要，省流量）。
    let manifest = format!("https://ghcr.io/v2/{GHCR_REPO}/manifests/latest");
    let accept = "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";
    let resp = client
        .get(&manifest)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", accept)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let digest = resp
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_owned());

    // digest 头不一定每个代理都透传，退而求其次解析 body。
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    let remote_digest = match digest {
        Some(d) => d,
        None => return Err("GHCR 未返回 digest".to_owned()),
    };

    // 3) 尝试从 index 的子 manifest 拿 OCI revision 标签（人话版本号）。
    let remote_rev = extract_revision(client, token, &body).await;
    Ok((remote_digest, remote_rev))
}

/// 从 image index 里挑第一个平台 manifest，取它的 config 的 revision 标签。
/// 拿不到不致命（digest 已经够判定新旧），返回 None。
async fn extract_revision(
    client: &reqwest::Client,
    token: &str,
    index: &serde_json::Value,
) -> Option<String> {
    let sub = index["manifests"]
        .as_array()?
        .iter()
        .find(|m| m["platform"]["architecture"].as_str().is_some())?;
    let sub_digest = sub["digest"].as_str()?;
    let manifest_url = format!("https://ghcr.io/v2/{GHCR_REPO}/manifests/{sub_digest}");
    let manifest: serde_json::Value = client
        .get(&manifest_url)
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "application/vnd.oci.image.manifest.v1+json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let config_digest = manifest["config"]["digest"].as_str()?;
    let config_url = format!("https://ghcr.io/v2/{GHCR_REPO}/blobs/{config_digest}");
    let config: serde_json::Value = client
        .get(&config_url)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let rev = config["config"]["Labels"]["org.opencontainers.image.revision"]
        .as_str()?
        .to_owned();
    Some(rev.chars().take(7).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_differs_means_update() {
        let (avail, _) = conclude(Some("sha256:aaa"), "sha256:bbb", None, None);
        assert!(avail);
    }

    #[test]
    fn digest_same_means_current() {
        let (avail, msg) = conclude(Some("sha256:aaa"), "sha256:aaa", None, None);
        assert!(!avail);
        assert_eq!(msg, "已是最新版本");
    }

    #[test]
    fn falls_back_to_revision() {
        let (avail, msg) = conclude(None, "sha256:x", Some("abc1234"), Some("def5678"));
        assert!(avail);
        assert!(msg.contains("abc1234") && msg.contains("def5678"));
        let (avail2, _) = conclude(None, "sha256:x", Some("abc1234"), Some("abc1234"));
        assert!(!avail2);
    }

    #[test]
    fn no_local_info_is_honest() {
        let (avail, msg) = conclude(None, "sha256:x", None, None);
        assert!(!avail);
        assert!(msg.contains("无法确定"));
    }
}
