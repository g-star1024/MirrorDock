//! 应用内访问鉴权（X10-119）。
//!
//! 【为什么需要】
//! NAS 版的全部能力——看实时画面、远程触控、删文件、装 APK——此前零门槛开放。
//! 在局域网里这还能接受，一旦挂到公网（DDNS/反代），拿到地址的任何人都能
//! 完全控制手机。本模块给服务加上一层「访问密码 + 会话 Token」的鉴权：
//!
//! - 密码只以 PBKDF2-HMAC-SHA256 哈希落盘（随机 16B 盐，600k 迭代）；
//! - 登录签发 16B 随机会话 Token（内存会话表，12 小时滑动过期）；
//! - HTTP 走 `Authorization: Bearer <token>`，WebSocket（不能自定义头）走
//!   `?token=` 查询参数；
//! - 未设置密码时鉴权整体停用，局域网老用户零感知；
//! - `MIRRORDOCK_ACCESS_PASSWORD` 环境变量是部署者兜底：显式设置时以 env 为准，
//!   Web 改密码无效（与保留天数的 env 锁定同一套语义）。
//!
//! 【抗爆破】
//! 登录按来源 IP 限速：1 分钟内错 5 次即拒绝（429）。密码哈希用恒定时间比较，
//! 防旁路计时攻击。
//!
//! 【日志纪律】
//! 密码、哈希、Token 一律不进日志（工程铁律：不打 access token）。

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State as AxumState};
use axum::http::{header, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use base64::Engine as _;
use ring::{pbkdf2, rand};

use crate::State;

/// PBKDF2 迭代次数（OWASP 对 HMAC-SHA256 的建议值）。
const PBKDF2_ITERATIONS: u32 = 600_000;
/// 随机盐长度。
const SALT_LEN: usize = 16;
/// 派生密钥长度。
const KEY_LEN: usize = 32;
/// 会话 Token 长度（128 位随机，base64url 后 22 字符）。
const TOKEN_LEN: usize = 16;
/// 会话空闲多久后过期。
const SESSION_TTL: Duration = Duration::from_secs(12 * 3600);
/// 登录限速窗口。
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// 窗口内允许的最大失败次数。
const RATE_MAX_FAILURES: u32 = 5;

const CREDENTIAL: pbkdf2::Algorithm = pbkdf2::PBKDF2_HMAC_SHA256;

/// 鉴权状态：密码来源判定在启动时完成一次，会话表与限速表常驻内存。
///
/// 【为什么用同步互斥锁】会话/限速两张表只存「查-改-放」的纯内存操作，
/// 临界区里没有 await——用 tokio 的异步锁反而把每个请求都卷进调度器。
pub struct AuthState {
    /// env 显式设置了访问密码（此时 Web 改密码无效）。
    env_locked: bool,
    /// 生效的密码哈希：env > Web 设置；在「密码设置/清除」时更新。
    ///
    /// 与 settings.json 双写：settings 管持久化，这里管请求热路径——
    /// 否则每个请求都要等一次设置读锁。
    effective_hash: StdMutex<Option<String>>,
    /// 会话表：token → 最近活跃时刻。滑动过期。
    sessions: StdMutex<HashMap<String, Instant>>,
    /// 登录失败计数：IP → (窗口起点, 失败数)。
    failures: StdMutex<HashMap<IpAddr, (Instant, u32)>>,
}

impl AuthState {
    /// 启动时装配：env 密码优先，其次 settings.json 里已存的 Web 密码。
    pub fn new(settings_hash: Option<String>) -> Self {
        let env_password = std::env::var("MIRRORDOCK_ACCESS_PASSWORD")
            .ok()
            .filter(|p| !p.is_empty());
        let env_hash = env_password.as_deref().map(hash_password);
        let env_locked = env_hash.is_some();
        if env_locked {
            println!("  访问鉴权：已启用（密码由环境变量 MIRRORDOCK_ACCESS_PASSWORD 提供，Web 修改无效）");
        } else if settings_hash.is_some() {
            println!("  访问鉴权：已启用（Web 设置的访问密码）");
        }
        Self {
            env_locked,
            effective_hash: StdMutex::new(env_hash.or(settings_hash)),
            sessions: StdMutex::new(HashMap::new()),
            failures: StdMutex::new(HashMap::new()),
        }
    }

    /// 访问密码是否被 env 钉死。
    pub fn locked_by_env(&self) -> bool {
        self.env_locked
    }

    /// 鉴权是否启用（有无生效密码）。
    pub fn enabled(&self) -> bool {
        self.effective_hash.lock().unwrap().is_some()
    }

    /// 校验一个密码是否与当前生效密码匹配（改密码前的「当前密码」确认用）。
    pub fn verify_password(&self, password: &str) -> bool {
        let hash = self.effective_hash.lock().unwrap().clone();
        hash.is_some_and(|h| verify_password(password, &h))
    }

    /// 校验密码。通过则签发会话 Token。
    pub fn login(&self, ip: IpAddr, password: &str) -> Result<String, AuthFail> {
        if self.rate_limited(ip) {
            return Err(AuthFail::RateLimited);
        }
        let hash = self.effective_hash.lock().unwrap().clone();
        let Some(hash) = hash else {
            return Err(AuthFail::Disabled);
        };
        if !verify_password(password, &hash) {
            self.record_failure(ip);
            return Err(AuthFail::BadPassword);
        }
        // 登录成功：清掉该 IP 的失败计数。
        self.failures.lock().unwrap().remove(&ip);
        let token = random_b64(TOKEN_LEN);
        self.sessions
            .lock()
            .unwrap()
            .insert(token.clone(), Instant::now());
        Ok(token)
    }

    /// 校验会话 Token（滑动续期）。未启用鉴权时恒真。
    pub fn check(&self, token: Option<&str>) -> bool {
        if !self.enabled() {
            return true;
        }
        let Some(token) = token else { return false };
        let mut sessions = self.sessions.lock().unwrap();
        match sessions.get_mut(token) {
            Some(last) if last.elapsed() < SESSION_TTL => {
                *last = Instant::now();
                true
            }
            Some(_) => {
                sessions.remove(token);
                false
            }
            None => false,
        }
    }

    /// 注销会话。
    pub fn logout(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }

    /// Web 提交的新密码哈希（None = 清除，关闭鉴权）。env 锁定时拒绝。
    ///
    /// 双写内存与 settings.json（持久化由调用方 settings 完成后的视图保证）。
    pub fn apply_password_change(&self, new_hash: Option<String>) -> Result<(), &'static str> {
        if self.env_locked {
            return Err("访问密码由环境变量 MIRRORDOCK_ACCESS_PASSWORD 钉死，网页修改不生效");
        }
        *self.effective_hash.lock().unwrap() = new_hash;
        if !self.enabled() {
            // 关闭鉴权：已签发的会话一并作废，免得旧 Token 在新密码下继续有效。
            self.sessions.lock().unwrap().clear();
        } else {
            // 换密码同样作废旧会话：只有知道新密码的人才能继续。
            self.sessions.lock().unwrap().clear();
        }
        Ok(())
    }

    fn rate_limited(&self, ip: IpAddr) -> bool {
        let failures = self.failures.lock().unwrap();
        match failures.get(&ip) {
            Some((start, count)) => start.elapsed() < RATE_WINDOW && *count >= RATE_MAX_FAILURES,
            None => false,
        }
    }

    fn record_failure(&self, ip: IpAddr) {
        let mut failures = self.failures.lock().unwrap();
        let entry = failures.entry(ip).or_insert((Instant::now(), 0));
        if entry.0.elapsed() >= RATE_WINDOW {
            *entry = (Instant::now(), 0);
        }
        entry.1 += 1;
    }
}

#[derive(Debug)]
pub enum AuthFail {
    BadPassword,
    RateLimited,
    Disabled,
}

// ---------------------------------------------------------------------------
// 密码哈希（PBKDF2-HMAC-SHA256）
// ---------------------------------------------------------------------------

/// 生成随机盐并派生哈希，输出 `salt_b64.key_b64`。
pub fn hash_password(password: &str) -> String {
    let rng = rand::SystemRandom::new();
    let mut salt = [0u8; SALT_LEN];
    rand::SecureRandom::fill(&rng, &mut salt).expect("系统随机源不可用");
    let mut key = [0u8; KEY_LEN];
    pbkdf2::derive(
        CREDENTIAL,
        std::num::NonZeroU32::new(PBKDF2_ITERATIONS).unwrap(),
        &salt,
        password.as_bytes(),
        &mut key,
    );
    let b64 = base64::engine::general_purpose::STANDARD;
    format!("{}.{}", b64.encode(salt), b64.encode(key))
}

/// 校验密码是否匹配 `salt_b64.key_b64` 格式的哈希。格式损坏恒为不匹配。
pub fn verify_password(password: &str, stored: &str) -> bool {
    let Some((salt_b64, key_b64)) = stored.split_once('.') else {
        return false;
    };
    let b64 = base64::engine::general_purpose::STANDARD;
    let (Ok(salt), Ok(expected)) = (b64.decode(salt_b64), b64.decode(key_b64)) else {
        return false;
    };
    pbkdf2::verify(
        CREDENTIAL,
        std::num::NonZeroU32::new(PBKDF2_ITERATIONS).unwrap(),
        &salt,
        password.as_bytes(),
        &expected,
    )
    .is_ok()
}

/// 生成 n 字节随机数的 base64（Token 用）。
fn random_b64(n: usize) -> String {
    let rng = rand::SystemRandom::new();
    let mut buf = vec![0u8; n];
    rand::SecureRandom::fill(&rng, &mut buf).expect("系统随机源不可用");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

// ---------------------------------------------------------------------------
// 请求侧 Token 提取
// ---------------------------------------------------------------------------

/// 从请求里提取会话 Token：`Authorization: Bearer` 优先，`?token=` 兜底
/// （浏览器 WebSocket 不能自定义请求头，镜像流只能走查询参数）。
pub fn extract_token(req: &Request) -> Option<String> {
    if let Some(v) = req.headers().get(header::AUTHORIZATION) {
        if let Ok(s) = v.to_str() {
            if let Some(t) = s.strip_prefix("Bearer ") {
                let t = t.trim();
                if !t.is_empty() {
                    return Some(t.to_owned());
                }
            }
        }
    }
    req.uri().query().and_then(|q| {
        q.split('&').find_map(|kv| {
            let (k, v) = kv.split_once('=')?;
            (k == "token" && !v.is_empty()).then(|| v.to_owned())
        })
    })
}

// ---------------------------------------------------------------------------
// axum 中间件：未启用鉴权时零开销直通；启用后校验 Token，失败 401。
// ---------------------------------------------------------------------------

pub async fn require_auth(
    AxumState(state): AxumState<std::sync::Arc<State>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let token = extract_token(&req);
    if state.auth.check(token.as_deref()) {
        return next.run(req).await;
    }
    (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "code": "auth_required",
            "message": "需要登录",
            "recovery": "请在页面输入访问密码登录。",
        })),
    )
        .into_response()
}

/// `POST /api/auth/login`：校验密码、签发 Token。带 IP 限速。
pub async fn login(
    AxumState(state): AxumState<std::sync::Arc<State>>,
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    Json(body): Json<LoginRequest>,
) -> Response {
    match state.auth.login(addr.ip(), &body.password) {
        Ok(token) => Json(serde_json::json!({ "token": token })).into_response(),
        Err(AuthFail::BadPassword) => (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "code": "auth_bad_password",
                "message": "密码不正确",
                "recovery": "确认访问密码后重试；连续错误过多会被临时限制。",
            })),
        )
            .into_response(),
        Err(AuthFail::RateLimited) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "code": "auth_rate_limited",
                "message": "尝试次数过多，请 1 分钟后再试",
                "recovery": "稍等片刻再登录。",
            })),
        )
            .into_response(),
        Err(AuthFail::Disabled) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "code": "auth_disabled",
                "message": "未设置访问密码，无需登录",
                "recovery": "在设置页启用访问密码后才需要登录。",
            })),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub struct LoginRequest {
    #[serde(default)]
    pub password: String,
}

/// `POST /api/auth/logout`：注销当前会话。
pub async fn logout(
    AxumState(state): AxumState<std::sync::Arc<State>>,
    req: Request<Body>,
) -> Json<serde_json::Value> {
    if let Some(token) = extract_token(&req) {
        state.auth.logout(&token);
    }
    Json(serde_json::json!({ "ok": true }))
}

/// `GET /api/auth/state`：未登录可用——前端据此决定要不要弹登录框。
pub async fn auth_state(
    AxumState(state): AxumState<std::sync::Arc<State>>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "auth_enabled": state.auth.enabled(),
        "password_locked_by_env": state.auth.locked_by_env(),
    }))
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_then_verify_roundtrip() {
        let h = hash_password("correct horse");
        assert!(verify_password("correct horse", &h));
        assert!(!verify_password("wrong", &h));
    }

    #[test]
    fn hashes_use_random_salt() {
        // 同一密码两次哈希必须不同（随机盐），否则会被彩虹表一把梭。
        assert_ne!(hash_password("same"), hash_password("same"));
    }

    #[test]
    fn verify_rejects_malformed() {
        assert!(!verify_password("x", "not-a-hash"));
        assert!(!verify_password("x", "!!!.???"));
        assert!(!verify_password("x", ""));
    }

    #[test]
    fn session_lifecycle() {
        let auth = AuthState::new(Some(hash_password("pw123456")));
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        assert!(auth.enabled());
        assert!(matches!(
            auth.login(ip, "wrong"),
            Err(AuthFail::BadPassword)
        ));
        let token = auth.login(ip, "pw123456").unwrap();
        assert!(auth.check(Some(&token)));
        assert!(!auth.check(Some("no-such-token")));
        assert!(!auth.check(None));
        auth.logout(&token);
        assert!(!auth.check(Some(&token)));
    }

    #[test]
    fn no_password_means_open_access() {
        let auth = AuthState::new(None);
        assert!(!auth.enabled());
        // 未启用鉴权：任何请求（含无 Token）都放行。
        assert!(auth.check(None));
        assert!(matches!(
            auth.login("1.2.3.4".parse().unwrap(), "x"),
            Err(AuthFail::Disabled)
        ));
    }

    #[test]
    fn password_change_invalidates_sessions() {
        let auth = AuthState::new(Some(hash_password("old-password")));
        let ip: IpAddr = "127.0.0.2".parse().unwrap();
        let token = auth.login(ip, "old-password").unwrap();
        auth.apply_password_change(Some(hash_password("new-password")))
            .unwrap();
        // 换密码后旧 Token 必须作废。
        assert!(!auth.check(Some(&token)));
        assert!(auth.login(ip, "new-password").is_ok());
        // 清除密码 = 关闭鉴权，自由进出。
        auth.apply_password_change(None).unwrap();
        assert!(auth.check(None));
    }

    #[test]
    fn wrong_password_is_rate_limited() {
        let auth = AuthState::new(Some(hash_password("pw123456")));
        let ip: IpAddr = "10.0.0.9".parse().unwrap();

        for _ in 0..RATE_MAX_FAILURES {
            assert!(matches!(auth.login(ip, "bad"), Err(AuthFail::BadPassword)));
        }
        // 第 6 次（哪怕密码正确）也被限速挡下。
        assert!(matches!(
            auth.login(ip, "pw123456"),
            Err(AuthFail::RateLimited)
        ));
    }

    #[test]
    fn token_extraction_prefers_header() {
        let req = Request::builder()
            .uri("/api/status?token=query-token")
            .header(header::AUTHORIZATION, "Bearer header-token")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_token(&req).as_deref(), Some("header-token"));

        let req2 = Request::builder()
            .uri("/api/stream/ws?serial=x&token=query-token")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_token(&req2).as_deref(), Some("query-token"));

        let req3 = Request::builder()
            .uri("/api/status")
            .body(Body::empty())
            .unwrap();
        assert!(extract_token(&req3).is_none());
    }
}
