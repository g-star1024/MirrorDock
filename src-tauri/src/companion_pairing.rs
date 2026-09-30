//! C4-01 伴侣 App 同网加密会话（桌面侧）；M4-1 起升级为持久互信。
//!
//! 设计要点（与威胁模型 TM-005 对齐）：
//! - 出带校验：配对二维码携带**桌面长期身份**公钥的 SHA-256 指纹，伴侣 App 用
//!   指纹锁定服务器身份，同网中间人无法伪造。
//! - 桌面长期身份：EC P-256 自签证书，密钥落 `app_data_dir/companion-identity/`
//!   （仅本机；丢失=互信作废，两端重新扫码配对）。
//! - 一次性 token：10 字节随机 → base32 16 字符；首配握手用，配对结束即失效。
//! - 挑战-响应互信（MDP2）：伴侣 App 在 Android Keystore 生成不可导出的
//!   EC P-256 身份，配对时桌面发 nonce 挑战、伴侣签名、桌面验证后把设备公钥
//!   存入 `paired-companions.json`。此后重连（RECONNECT）无需再扫码。
//! - 会话内容只有握手与统计 JSON 行，不传屏幕帧（屏幕帧不入日志/不落盘铁律）。
//! - 状态机 idle → listening → connected → idle，事件环形缓冲供前端展示。

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

/// 事件环形缓冲上限，与诊断日志口径一致。
const EVENT_LIMIT: usize = 200;

/// 握手协议版本前缀。MDP2 = 挑战-响应互信（M4-1）。
pub const PROTOCOL_TAG: &str = "MDP2";

/// 挑战 nonce 字节数（32 字节熵）。
const CHALLENGE_BYTES: usize = 32;

/// 单次配对暴露给前端的全部信息（就是二维码载荷的字段）。
#[derive(Debug, Clone, Serialize)]
pub struct PairingOffer {
    /// 协议版本，伴侣 App 校验。
    pub version: u8,
    /// 本机局域网 IPv4 候选（可能多个；伴侣 App 逐个尝试）。
    pub hosts: Vec<String>,
    pub port: u16,
    /// 一次性配对码（base32，16 字符）。
    pub token: String,
    /// 桌面长期身份公钥（SPKI）的 SHA-256（hex 小写）。
    pub fingerprint: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PairingPhase {
    #[default]
    Idle,
    Listening,
    Connected,
}

#[derive(Debug, Clone, Serialize)]
pub struct PairingStatus {
    pub phase: PairingPhase,
    /// 最近事件（时间戳 + 文案；不含 token、不含屏幕数据）。
    pub events: Vec<String>,
    pub offer: Option<PairingOffer>,
}

#[derive(Default)]
pub struct PairingState {
    inner: Mutex<PairingInner>,
}

#[derive(Default)]
struct PairingInner {
    phase: PairingPhase,
    events: Vec<String>,
    offer: Option<PairingOffer>,
    shutdown: Option<oneshot::Sender<()>>,
    /// 设备报到时触发的桥接回调（桌面侧注入）：入参为伴侣端来源 IP，
    /// 返回要追加进事件流的文案。伴侣模块不感知 adb——镜像通道的
    /// 自动连接逻辑（mDNS 匹配 + adb connect）由调用方闭包实现。
    device_bridge: Option<Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>>,
}

impl PairingState {
    /// 注入设备报到桥接回调；应在任何配对开始前完成（run() 启动时设置一次）。
    pub fn set_device_bridge(&self, bridge: Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>) {
        self.inner.lock().unwrap().device_bridge = Some(bridge);
    }

    fn take_device_bridge(&self) -> Option<Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>> {
        self.inner.lock().unwrap().device_bridge.clone()
    }

    fn push_event(&self, text: impl Into<String>) {
        let mut inner = self.inner.lock().unwrap();
        inner.events.push(format!("{} {}", hhmmss_now(), text.into()));
        if inner.events.len() > EVENT_LIMIT {
            let excess = inner.events.len() - EVENT_LIMIT;
            inner.events.drain(0..excess);
        }
    }

    pub fn status(&self) -> PairingStatus {
        let inner = self.inner.lock().unwrap();
        PairingStatus {
            phase: inner.phase.clone(),
            events: inner.events.clone(),
            offer: inner.offer.clone(),
        }
    }

    fn set_phase(&self, phase: PairingPhase) {
        self.inner.lock().unwrap().phase = phase;
    }
}

/// 桌面长期身份（M4-1）：EC P-256 密钥 + 自签证书，落 `companion-identity/`。
///
/// 指纹 = SHA-256(SPKI DER)，是二维码出带校验的对象；密钥不变则指纹不变，
/// 伴侣 App 首配后记住指纹，重连无需重新扫码。
pub struct PairingIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: String,
}

impl PairingIdentity {
    /// 读取或生成桌面身份。文件损坏/缺失时重新生成（互信作废需重新扫码）。
    ///
    /// 指纹独立落 `identity.fp`：读取路径无需解析 PKCS8（证书/指纹都以生成时
    /// 计算好的为准）；指纹文件被篡改只会导致出带校验不匹配（fail-closed）。
    pub fn load_or_create(dir: &Path) -> Result<(Self, bool), String> {
        let key_path = dir.join("identity.key");
        let cert_path = dir.join("identity.der");
        let fp_path = dir.join("identity.fp");
        if let (Ok(key_der), Ok(cert_der), Ok(fp)) =
            (std::fs::read(&key_path), std::fs::read(&cert_path), std::fs::read(&fp_path))
        {
            let fingerprint = String::from_utf8_lossy(&fp).trim().to_string();
            let valid = fingerprint.len() == 64
                && fingerprint.chars().all(|c| c.is_ascii_hexdigit());
            if valid {
                return Ok((
                    Self {
                        cert_der,
                        key_der,
                        fingerprint: fingerprint.to_lowercase(),
                    },
                    false,
                ));
            }
        }
        let identity = Self::generate()?;
        std::fs::create_dir_all(dir).map_err(|e| format!("创建身份目录失败：{e}"))?;
        std::fs::write(&key_path, &identity.key_der).map_err(|e| format!("写身份密钥失败：{e}"))?;
        std::fs::write(&cert_path, &identity.cert_der).map_err(|e| format!("写身份证书失败：{e}"))?;
        std::fs::write(&fp_path, &identity.fingerprint).map_err(|e| format!("写身份指纹失败：{e}"))?;
        restrict_owner_only(&key_path);
        Ok((identity, true))
    }

    /// 现场生成新身份（rcgen 默认即 ECDSA P-256，与伴侣 App Keystore 算法一致）。
    fn generate() -> Result<Self, String> {
        let key_pair = rcgen::KeyPair::generate().map_err(|e| format!("生成密钥失败：{e}"))?;
        let mut params = rcgen::CertificateParams::new(vec!["mirrordock-companion.local".into()])
            .map_err(|e| e.to_string())?;
        params.not_before = rcgen::date_time_ymd(2026, 1, 1);
        params.not_after = rcgen::date_time_ymd(2077, 1, 1);
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "MirrorDock Desktop Identity");
        let cert = params.self_signed(&key_pair).map_err(|e| e.to_string())?;
        let cert_der = cert.der().as_ref().to_vec();
        let key_der = key_pair.serialize_der();
        let spki_der = key_pair.public_key_der();
        let mut hasher = Sha256::new();
        hasher.update(&spki_der);
        Ok(Self {
            cert_der,
            key_der,
            fingerprint: hex_lower(&hasher.finalize()),
        })
    }
}

/// Unix 下把身份密钥限制为仅属主可读写（0600）；Windows 无对应位，跳过。
fn restrict_owner_only(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// 已配对伴侣设备的持久化台账（M4-1）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PairedCompanion {
    /// 设备身份指纹（SHA-256(SPKI) 前 16 hex），重连凭据。
    pub pairing_id: String,
    pub model: String,
    /// 设备公钥 SPKI DER 的 hex，验证重连签名用。
    pub pubkey_hex: String,
    pub added_at: u64,
    pub last_seen: u64,
}

/// `paired-companions.json` 的读写。损坏按空表处理（用户重新扫码即可恢复）。
pub struct PairedStore {
    path: PathBuf,
}

impl PairedStore {
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join("paired-companions.json"),
        }
    }

    pub fn load(&self) -> Vec<PairedCompanion> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, devices: &[PairedCompanion]) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败：{e}"))?;
        }
        let body = serde_json::to_vec_pretty(devices).map_err(|e| format!("序列化失败：{e}"))?;
        std::fs::write(&self.path, body).map_err(|e| format!("写配对台账失败：{e}"))
    }

    /// 按 pairing_id 插入或更新（last_seen 刷新），返回更新后的全表。
    pub fn upsert(&self, device: PairedCompanion) -> Result<Vec<PairedCompanion>, String> {
        let mut devices = self.load();
        match devices.iter_mut().find(|d| d.pairing_id == device.pairing_id) {
            Some(existing) => {
                existing.model = device.model;
                existing.pubkey_hex = device.pubkey_hex;
                existing.last_seen = device.last_seen;
            }
            None => devices.push(device),
        }
        self.save(&devices)?;
        Ok(devices)
    }

    pub fn remove(&self, pairing_id: &str) -> Result<Vec<PairedCompanion>, String> {
        let devices: Vec<PairedCompanion> = self
            .load()
            .into_iter()
            .filter(|d| d.pairing_id != pairing_id)
            .collect();
        self.save(&devices)?;
        Ok(devices)
    }
}

/// 由设备公钥 SPKI DER 计算 pairing_id（指纹前 16 hex）。
pub fn pairing_id_from_spki(spki_der: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(spki_der);
    hex_lower(&hasher.finalize())[..16].to_string()
}

/// 解码 hex 编码的公钥（SPKI DER）并构造验证密钥（校验失败=对端身份不可信）。
fn verifying_key_from_hex(pubkey_hex: &str) -> Option<p256::ecdsa::VerifyingKey> {
    use pkcs8::DecodePublicKey as _;
    let spki = hex_decode(pubkey_hex)?;
    let public_key = p256::PublicKey::from_public_key_der(&spki).ok()?;
    Some(p256::ecdsa::VerifyingKey::from(&public_key))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// 验证伴侣对 nonce 的 ECDSA P-256 签名。
///
/// Java `SHA256withECDSA` 输出 DER 编码签名；定宽 64 字节 r||s 编码同样接受
/// （同一签名的两种序列化，验证强度不变——都通过 ECDSA 验算）。
fn verify_challenge(pubkey_hex: &str, nonce: &[u8], sig_hex: &str) -> bool {
    use p256::ecdsa::signature::Verifier;
    let Some(verifying_key) = verifying_key_from_hex(pubkey_hex) else {
        return false;
    };
    let Some(sig_bytes) = hex_decode(sig_hex) else {
        return false;
    };
    let signature = match p256::ecdsa::Signature::from_der(&sig_bytes) {
        Ok(s) => s,
        Err(_) => match p256::ecdsa::Signature::from_slice(&sig_bytes) {
            Ok(s) => s,
            Err(_) => return false,
        },
    };
    verifying_key.verify(nonce, &signature).is_ok()
}

/// 生成一次性 token（10 字节熵 → base32 16 字符）。
pub fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 10];
    getrandom::getrandom(&mut bytes).expect("系统熵源不可用");
    crate::licensing::base32_encode(&bytes)
}

/// 枚举本机局域网 IPv4 候选（UDP connect 技巧 + 回环回退）。
pub fn local_ipv4_candidates() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
        // 不真正发包，仅借 connect 决定默认路由源地址。
        if sock.connect("223.5.5.5:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                out.push(addr.ip().to_string());
            }
        }
    }
    if !out.iter().any(|h| h == "127.0.0.1") {
        out.push("127.0.0.1".to_string());
    }
    out
}

/// 启动一次配对监听（自动结束/替换旧会话）。
///
/// `identity_dir`：桌面长期身份目录；`store`：已配对设备台账。
pub fn begin_pairing(
    state: &Arc<PairingState>,
    identity_dir: &Path,
    store: PairedStore,
) -> Result<PairingOffer, crate::AppError> {
    end_pairing(state);

    let (identity, created) =
        PairingIdentity::load_or_create(identity_dir).map_err(|e| crate::AppError {
            code: "pairing_cert_failed",
            message: format!("配对身份不可用：{e}"),
            recovery: "重试一次；持续失败请通过帮助与诊断反馈。".into(),
        })?;
    if created {
        state.push_event("已生成新的桌面配对身份（首次使用或原身份损坏）");
    }
    let fingerprint = identity.fingerprint.clone();
    let token = generate_pairing_token();
    let hosts = local_ipv4_candidates();

    // 同步 std 监听（快速、可在命令线程做），运行时再转 tokio。
    let std_listener =
        std::net::TcpListener::bind("0.0.0.0:0").map_err(|e| crate::AppError {
            code: "pairing_bind_failed",
            message: format!("配对端口监听失败：{e}"),
            recovery: "检查系统防火墙后重试。".into(),
        })?;
    let port = std_listener.local_addr().map_err(|e| crate::AppError {
        code: "pairing_bind_failed",
        message: format!("读取端口失败：{e}"),
        recovery: "重试一次。".into(),
    })?
    .port();

    let offer = PairingOffer {
        version: 1,
        hosts,
        port,
        token: token.clone(),
        fingerprint: fingerprint.clone(),
    };

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.offer = Some(offer.clone());
        inner.shutdown = Some(shutdown_tx);
        inner.phase = PairingPhase::Listening;
    }
    state.push_event(format!("配对监听已就绪（端口 {port}）"));

    let shared = state.clone();
    // JoinHandle 丢弃即脱管（tokio 语义），生命周期由 shutdown 通道管理。
    let handle = tauri::async_runtime::spawn(async move {
        let listener = match set_nonblocking_and_convert(std_listener) {
            Ok(l) => l,
            Err(e) => {
                shared.push_event(format!("监听器转换失败：{e}"));
                shared.set_phase(PairingPhase::Idle);
                return;
            }
        };
        run_accept_loop(listener, identity, token, store, shared, shutdown_rx).await;
    });
    drop(handle);

    Ok(offer)
}

/// std 监听器转 tokio：先切非阻塞，否则运行时拒绝注册。
fn set_nonblocking_and_convert(
    std_listener: std::net::TcpListener,
) -> std::io::Result<tokio::net::TcpListener> {
    std_listener.set_nonblocking(true)?;
    tokio::net::TcpListener::from_std(std_listener)
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hhmmss_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let secs_of_day = now % 86400;
    format!(
        "{:02}:{:02}:{:02}",
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60
    )
}

/// 接受循环：串行处理会话（POC 足够），shutdown 通道随时可打断。
async fn run_accept_loop(
    listener: tokio::net::TcpListener,
    identity: PairingIdentity,
    token_expected: String,
    store: PairedStore,
    state: Arc<PairingState>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let certs = vec![CertificateDer::from(identity.cert_der.clone())];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(identity.key_der.clone()));
    let config = match rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
    {
        Ok(c) => c,
        Err(e) => {
            state.push_event(format!("TLS 配置失败：{e}"));
            state.set_phase(PairingPhase::Idle);
            return;
        }
    };
    let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));

    loop {
        tokio::select! {
            _ = &mut shutdown_rx => {
                state.push_event("配对已手动结束");
                state.set_phase(PairingPhase::Idle);
                return;
            }
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        state.push_event(format!("接受连接失败：{e}"));
                        return;
                    }
                };
                state.push_event(format!("收到连接 {peer}"));
                state.set_phase(PairingPhase::Connected);
                tokio::select! {
                    _ = &mut shutdown_rx => {
                        state.push_event("配对已手动结束");
                        state.set_phase(PairingPhase::Idle);
                        return;
                    }
                    result = acceptor.accept(stream) => {
                        match result {
                            Ok(tls) => serve_session(tls, &token_expected, &identity, &store, &state, peer.ip().to_string()).await,
                            Err(e) => state.push_event(format!("TLS 握手失败：{e}")),
                        }
                    }
                }
                // 会话结束后回到监听状态，允许伴侣端重连直到手动结束。
                state.set_phase(PairingPhase::Listening);
            }
        }
    }
}

/// 单个伴侣会话：握手 + JSON 行事件流，直到 bye/断开。不接屏幕帧。
///
/// `peer_ip` 是伴侣端的局域网来源 IP，设备报到时交给桥接回调
/// （桌面侧用它自动连接这台手机的镜像通道）。
async fn serve_session(
    tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    token_expected: &str,
    identity: &PairingIdentity,
    store: &PairedStore,
    state: &PairingState,
    peer_ip: String,
) {
    let (reader, mut writer) = tokio::io::split(tls);
    let mut reader = BufReader::new(reader);

    // 第一行："MDP2 <token>"（首配，扫码）或 "MDP2 RECONNECT <pairing_id>"（互信重连）。
    let hello = match read_line(&mut reader, std::time::Duration::from_secs(10)).await {
        Some(line) => line,
        None => {
            state.push_event("握手超时或对端提前断开");
            return;
        }
    };
    let mut parts = hello.split_whitespace();
    let proto = parts.next().unwrap_or("");
    if proto != PROTOCOL_TAG {
        let _ = writer.write_all(b"{\"type\":\"rejected\"}\n").await;
        state.push_event("协议版本不支持，连接已拒绝");
        return;
    }

    // MDP2 互信握手：验证对端持有其申报公钥对应的私钥，才允许进入会话。
    let handshake = match parts.next().unwrap_or("") {
        word if word == token_expected => {
            establish_trust(EstablishMode::FreshPairing, store, &mut reader, &mut writer, state).await
        }
        "RECONNECT" => {
            let pairing_id = parts.next().unwrap_or("").to_string();
            establish_trust(EstablishMode::Reconnect { pairing_id }, store, &mut reader, &mut writer, state).await
        }
        _ => {
            let _ = writer.write_all(b"{\"type\":\"rejected\"}\n").await;
            state.push_event("配对码校验失败，连接已拒绝");
            return;
        }
    };
    let Some(device) = handshake else { return }; // 失败原因已写入事件流。

    let _ = writer
        .write_all(format!("{{\"type\":\"paired_ok\",\"pairing_id\":\"{}\"}}\n", device.pairing_id).as_bytes())
        .await;
    state.push_event(format!(
        "设备报到：{}（互信会话已建立，来源 {}）",
        device.model, peer_ip
    ));
    // 桥接可能阻塞（mDNS 扫描 + adb connect），丢进阻塞线程池，
    // 结果以事件形式回到事件流——前端面板直接可见。
    if let Some(bridge) = state.take_device_bridge() {
        let peer = peer_ip.clone();
        let join = tauri::async_runtime::spawn_blocking(move || bridge(&peer));
        match join.await {
            Ok(lines) => {
                for line in lines {
                    state.push_event(line);
                }
            }
            Err(e) => state.push_event(format!("桥接任务失败：{e}")),
        }
    }

    loop {
        match read_line(&mut reader, std::time::Duration::from_secs(300)).await {
            Some(line) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    state.push_event("收到无法解析的行（已忽略）");
                    continue;
                };
                match value.get("type").and_then(|t| t.as_str()) {
                    Some("bye") => {
                        state.push_event("伴侣端正常结束会话");
                        return;
                    }
                    Some("device_hello") => {
                        // 互信握手已把设备报到并入 establish_trust；这里再收到
                        // 说明对端走了旧协议流程，如实记录，不中断会话。
                        state.push_event("收到旧协议报到消息（已忽略）");
                    }
                    Some("capture_stats") => {
                        let frames = value.get("frames").and_then(|f| f.as_u64()).unwrap_or(0);
                        let audio = value
                            .get("audio_supported")
                            .and_then(|a| a.as_bool())
                            .unwrap_or(false);
                        let sample = value
                            .get("sample_jpeg_bytes")
                            .and_then(|b| b.as_u64())
                            .unwrap_or(0);
                        state.push_event(format!(
                            "捕获统计：{frames} 帧，音频捕获支持={audio}，JPEG 样本 {sample} 字节（不落盘）"
                        ));
                    }
                    Some(other) => {
                        state.push_event(format!("收到未知消息类型 {other}（已忽略）"));
                    }
                    None => {}
                }
            }
            None => {
                state.push_event("会话结束（超时或断开）");
                return;
            }
        }
    }
}

/// 互信建立模式：首配（凭一次性 token）或重连（凭 pairing_id）。
enum EstablishMode {
    FreshPairing,
    Reconnect { pairing_id: String },
}

/// MDP2 互信握手：
/// 1. 对端申报身份（首配：`device_hello{model,pubkey}`；重连：查台账取公钥）；
/// 2. 桌面发 `challenge{nonce}`，对端回 `challenge_response{sig}`（ECDSA P-256，DER）；
/// 3. 验签通过 → 台账登记（首配插入 / 重连刷新 last_seen）→ 返回设备信息。
///
/// 任何一步失败都写入事件流并返回 None（调用方直接断开，不发 paired_ok）。
async fn establish_trust<L, W>(
    mode: EstablishMode,
    store: &PairedStore,
    lines: &mut L,
    writer: &mut W,
    state: &PairingState,
) -> Option<PairedCompanion>
where
    L: tokio::io::AsyncBufRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let is_fresh = matches!(mode, EstablishMode::FreshPairing);
    let _ = writer
        .write_all(b"{\"type\":\"welcome\",\"protocol\":\"MDP2\"}\n")
        .await;

    // 已配对台账里的记录（重连路径用）；申报的公钥 hex 与机型。
    let (pubkey_hex, model, stored) = match mode {
        EstablishMode::FreshPairing => {
            let hello = match read_json_line(lines, state, "未收到设备报到").await {
                Some(v) => v,
                None => return None,
            };
            if hello.get("type").and_then(|t| t.as_str()) != Some("device_hello") {
                state.push_event("设备报到消息格式不正确");
                return None;
            }
            // 机型只保留安全字符并限长（进事件流与台账，不透传控制字符）。
            let model = hello
                .get("model")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown")
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-' || *c == '_')
                .take(64)
                .collect::<String>();
            let pubkey = hello.get("pubkey").and_then(|p| p.as_str()).unwrap_or("");
            (pubkey.to_string(), model, None)
        }
        EstablishMode::Reconnect { pairing_id } => {
            let found = store.load().into_iter().find(|d| d.pairing_id == pairing_id);
            match found {
                Some(device) => {
                    let pubkey = device.pubkey_hex.clone();
                    let model = device.model.clone();
                    (pubkey, model, Some(device))
                }
                None => {
                    state.push_event("重连被拒：这台设备未与本机配对（可能已被移除）");
                    let _ = writer.write_all(b"{\"type\":\"rejected\"}\n").await;
                    return None;
                }
            }
        }
    };

    // 公钥有效性：能解析成 P-256 公钥才继续（防止把垃圾字节写进台账）。
    if verifying_key_from_hex(&pubkey_hex).is_none() {
        state.push_event("设备公钥格式不正确，互信未建立");
        return None;
    }

    // 挑战：32 字节随机 nonce（hex 编码下发），要求对端用其私钥签名。
    let mut nonce = vec![0u8; CHALLENGE_BYTES];
    getrandom::getrandom(&mut nonce).expect("系统熵源不可用");
    let nonce_hex = hex_lower(&nonce);
    let _ = writer
        .write_all(format!("{{\"type\":\"challenge\",\"nonce\":\"{nonce_hex}\"}}\n").as_bytes())
        .await;

    let response = match read_json_line(lines, state, "未收到挑战签名").await {
        Some(v) => v,
        None => return None,
    };
    if response.get("type").and_then(|t| t.as_str()) != Some("challenge_response") {
        state.push_event("挑战响应消息格式不正确，互信未建立");
        return None;
    }
    let sig = response.get("sig").and_then(|s| s.as_str()).unwrap_or("");
    if !verify_challenge(&pubkey_hex, &nonce, sig) {
        state.push_event("挑战签名验证失败，连接已拒绝（对端不持有申报的身份）");
        return None;
    }

    // 验签通过：登记台账（首配插入，重连刷新 last_seen）。
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let device = match stored {
        Some(mut device) => {
            device.last_seen = now;
            if let Err(e) = store.upsert(device.clone()) {
                state.push_event(format!("台账更新失败：{e}"));
            }
            device
        }
        None => {
            let device = PairedCompanion {
                pairing_id: pairing_id_from_spki(&hex_decode(&pubkey_hex).unwrap_or_default()),
                model,
                pubkey_hex,
                added_at: now,
                last_seen: now,
            };
            if let Err(e) = store.upsert(device.clone()) {
                state.push_event(format!("台账写入失败：{e}"));
            }
            device
        }
    };
    state.push_event(format!(
        "互信已建立：{}（{}）",
        device.model,
        if is_fresh { "首次配对" } else { "重连" }
    ));
    Some(device)
}

/// 读一行 JSON（10 秒超时）；超时/断开写事件流并返回 None。
async fn read_json_line<L>(reader: &mut L, state: &PairingState, timeout_message: &str) -> Option<serde_json::Value>
where
    L: tokio::io::AsyncBufRead + Unpin,
{
    let line = read_line(reader, std::time::Duration::from_secs(10)).await?;
    serde_json::from_str::<serde_json::Value>(&line).ok()
}

/// 带超时读一行（UTF-8）；连接断开/超时返回 None。泛型同时服务生产与服务端/测试客户端。
async fn read_line<L>(reader: &mut L, timeout: std::time::Duration) -> Option<String>
where
    L: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::AsyncBufReadExt as _;
    let mut buf = String::new();
    match tokio::time::timeout(timeout, reader.read_line(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => Some(buf),
        _ => None,
    }
}

/// 结束当前配对（幂等）。
pub fn end_pairing(state: &Arc<PairingState>) {
    let mut inner = state.inner.lock().unwrap();
    if let Some(tx) = inner.shutdown.take() {
        let _ = tx.send(());
    }
    inner.phase = PairingPhase::Idle;
    inner.offer = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_name() -> tokio_rustls::rustls::pki_types::ServerName<'static> {
        tokio_rustls::rustls::pki_types::ServerName::try_from("mirrordock-companion.local".to_string()).unwrap()
    }

    #[test]
    fn token_is_16_base32_chars_and_random() {
        let a = generate_pairing_token();
        let b = generate_pairing_token();
        assert_eq!(a.len(), 16);
        assert_ne!(a, b);
        // base32 字母表（RFC 4648 无填充）：A-Z 与 2-7。
        assert!(a.chars().all(|c| c.is_ascii_uppercase() || ('2'..='7').contains(&c)));
    }

    #[test]
    fn identity_persists_and_fingerprint_is_stable() {
        let dir = std::env::temp_dir().join(format!("md-identity-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (identity, created) = PairingIdentity::load_or_create(&dir).unwrap();
        assert!(created);
        assert_eq!(identity.fingerprint.len(), 64);
        assert!(identity.key_der.starts_with(&[0x30])); // PKCS8 DER：SEQUENCE 开头。

        // 二次读取：不再生成（created=false），指纹一致（重连免扫码的前提）。
        let (again, created_again) = PairingIdentity::load_or_create(&dir).unwrap();
        assert!(!created_again);
        assert_eq!(again.fingerprint, identity.fingerprint);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paired_store_upsert_and_remove() {
        let dir = std::env::temp_dir().join(format!("md-paired-store-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = PairedStore::new(&dir);
        assert!(store.load().is_empty());

        let device = PairedCompanion {
            pairing_id: "AB12CD34EF56AB12".into(),
            model: "POCO F5".into(),
            pubkey_hex: "30".repeat(91),
            added_at: 1000,
            last_seen: 1000,
        };
        let devices = store.upsert(device.clone()).unwrap();
        assert_eq!(devices.len(), 1);

        // 同 id 二次报到只刷新字段，不产生重复行。
        let mut again = device.clone();
        again.last_seen = 2000;
        again.model = "POCO F5 renamed".into();
        let devices = store.upsert(again).unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].last_seen, 2000);
        assert_eq!(devices[0].model, "POCO F5 renamed");

        assert_eq!(store.remove(&device.pairing_id).unwrap().len(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ipv4_candidates_nonempty() {
        assert!(!local_ipv4_candidates().is_empty());
    }

    /// 测试用客户端身份：确定性 P-256 密钥（与伴侣 App Keystore 同算法）。
    fn test_device_key() -> p256::ecdsa::SigningKey {
        use p256::ecdsa::SigningKey;
        SigningKey::from_bytes(&[42u8; 32].into()).unwrap()
    }

    /// 客户端侧 TLS 连接（SPKI 指纹锁定，与伴侣 App 出带校验同构）。
    async fn tls_connect(
        port: u16,
        fingerprint: &str,
    ) -> tokio_rustls::client::TlsStream<tokio::net::TcpStream> {
        let verifier = Arc::new(FpVerifier {
            expected_fp: fingerprint.to_string(),
        });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let tcp = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        connector.connect(server_name(), tcp).await.unwrap()
    }

    /// 首配客户端：发 MDP2 <token>，走完 device_hello → challenge → 签名，
    /// 返回服务端最后一条回复（paired_ok 或无）。
    async fn fresh_pair_client(
        port: u16,
        fingerprint: &str,
        token: &str,
        model: &str,
        signing: &p256::ecdsa::SigningKey,
        sign_wrong_data: bool,
    ) -> Option<String> {
        use pkcs8::EncodePublicKey as _;
        use p256::ecdsa::signature::{SignatureEncoding, Signer};
        use tokio::io::AsyncWriteExt;

        let tls = tls_connect(port, fingerprint).await;
        let (read_half, mut write_half) = tokio::io::split(tls);
        let mut lines = BufReader::new(read_half).lines();

        write_half
            .write_all(format!("{PROTOCOL_TAG} {token}\n").as_bytes())
            .await
            .unwrap();
        let welcome = lines.next_line().await.unwrap().unwrap();
        assert!(welcome.contains("\"welcome\""), "got {welcome}");

        let spki = signing.verifying_key().to_public_key_der().unwrap();
        let pubkey_hex = hex_lower(spki.as_bytes());
        write_half
            .write_all(
                format!("{{\"type\":\"device_hello\",\"model\":\"{model}\",\"pubkey\":\"{pubkey_hex}\"}}\n")
                    .as_bytes(),
            )
            .await
            .unwrap();

        let challenge: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(challenge["type"], "challenge");
        let nonce = hex_decode(challenge["nonce"].as_str().unwrap()).unwrap();

        let signature: p256::ecdsa::Signature = if sign_wrong_data {
            signing.sign(b"not the nonce".as_slice())
        } else {
            signing.sign(nonce.as_slice())
        };
        let sig_hex = hex_lower(signature.to_vec().as_slice());
        write_half
            .write_all(format!("{{\"type\":\"challenge_response\",\"sig\":\"{sig_hex}\"}}\n").as_bytes())
            .await
            .unwrap();
        lines.next_line().await.ok().flatten()
    }

    #[tokio::test]
    async fn pairing_session_end_to_end() {
        let workdir = std::env::temp_dir().join(format!("md-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());
        let (identity, _) = PairingIdentity::load_or_create(&workdir).unwrap();
        let store = PairedStore::new(&workdir);
        let token = generate_pairing_token();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();

        let loop_state = state.clone();
        let loop_token = token.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_accept_loop(listener, identity, loop_token, store, loop_state, rx).await;
        });

        // 错误签名必须被拒（对端不持有申报身份）。
        let signing = test_device_key();
        let rejected = fresh_pair_client(port, &fingerprint_of(&workdir), &token, "Bad-Device", &signing, true)
            .await;
        assert!(rejected.is_none());
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(state.status().events.iter().any(|e| e.contains("挑战签名验证失败")));

        // 正确签名：完整握手 + 统计流 + bye。
        let paired = match fresh_pair_client(port, &fingerprint_of(&workdir), &token, "POC-Test", &signing, false).await {
            Some(v) => v,
            None => {
                eprintln!("服务端事件流：{:#?}", state.status().events);
                panic!("paired_ok");
            }
        };
        let pairing_id: serde_json::Value = serde_json::from_str(&paired).unwrap();
        assert_eq!(pairing_id["type"], "paired_ok");

        // 台账已登记该设备。
        let devices = PairedStore::new(&workdir).load();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].model, "POC-Test");
        assert_eq!(devices[0].pairing_id, pairing_id["pairing_id"].as_str().unwrap());
        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// 读取身份指纹（测试辅助：指纹文件由 load_or_create 落盘）。
    fn fingerprint_of(dir: &std::path::Path) -> String {
        std::fs::read_to_string(dir.join("identity.fp")).unwrap().trim().to_string()
    }

    /// 错误 token 必须被拒绝；未知 pairing_id 的重连同样被拒。
    #[tokio::test]
    async fn pairing_rejects_wrong_token() {
        use tokio::io::AsyncWriteExt;

        let workdir = std::env::temp_dir().join(format!("md-reject-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());
        let (identity, _) = PairingIdentity::load_or_create(&workdir).unwrap();
        let store = PairedStore::new(&workdir);
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();

        let loop_state = state.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_accept_loop(listener, identity, "CORRECTTOKEN1234".into(), store, loop_state, rx).await;
        });

        let verifier = Arc::new(FpVerifier { expected_fp: fingerprint_of(&workdir) });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

        // 错误 token。
        let tls = connector
            .connect(server_name(), tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap())
            .await
            .unwrap();
        let (mut read_half, mut write_half) = tokio::io::split(tls);
        write_half.write_all(b"MDP2 WRONGTOKEN00000\n").await.unwrap();
        let mut buf = vec![0u8; 128];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), read_half.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains("rejected"));
        assert!(state.status().events.iter().any(|e| e.contains("配对码校验失败")));

        // 未知 pairing_id 的重连。
        let tls = connector
            .connect(server_name(), tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap())
            .await
            .unwrap();
        let (mut read_half, mut write_half) = tokio::io::split(tls);
        write_half.write_all(b"MDP2 RECONNECT UNKNOWN00000000\n").await.unwrap();
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), read_half.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains("rejected"));
        assert!(state.status().events.iter().any(|e| e.contains("未与本机配对")));
        let _ = std::fs::remove_dir_all(&workdir);
    }

    #[derive(Debug)]
    struct FpVerifier {
        expected_fp: String,
    }

    impl rustls::client::danger::ServerCertVerifier for FpVerifier {
        fn verify_server_cert(
            &self,
            end_entity: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[tokio_rustls::rustls::pki_types::CertificateDer<'_>],
            _server_name: &tokio_rustls::rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: tokio_rustls::rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            let spki = x509_spki(end_entity.as_ref());
            let mut hasher = Sha256::new();
            hasher.update(&spki);
            let fp = hex_lower(&hasher.finalize());
            if fp == self.expected_fp {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::General("服务器指纹不匹配（疑似中间人）".into()))
            }
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &tokio_rustls::rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::RSA_PKCS1_SHA256,
                rustls::SignatureScheme::ED25519,
                // rcgen KeyPair::generate 默认 ECDSA P-256。
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            ]
        }
    }

    use tokio::io::AsyncReadExt as _;

    /// 从 DER 证书提取 SPKI（SubjectPublicKeyInfo）——最小 ASN.1 解析（测试用，
    /// 与伴侣 App 的出带校验逻辑同构）。
    fn x509_spki(cert_der: &[u8]) -> Vec<u8> {
        fn read_len(data: &[u8], pos: &mut usize) -> usize {
            let first = data[*pos];
            *pos += 1;
            if first & 0x80 == 0 {
                first as usize
            } else {
                let n = (first & 0x7f) as usize;
                let mut len = 0usize;
                for _ in 0..n {
                    len = (len << 8) | data[*pos] as usize;
                    *pos += 1;
                }
                len
            }
        }
        assert_eq!(cert_der[0], 0x30, "顶层必须是 SEQUENCE");
        let mut pos = 1usize;
        let _cert_len = read_len(cert_der, &mut pos);
        assert_eq!(cert_der[pos], 0x30, "tbsCertificate 必须是 SEQUENCE");
        pos += 1;
        let tbs_len = read_len(cert_der, &mut pos);
        let tbs_end = pos + tbs_len;
        while pos < tbs_end {
            let tag = cert_der[pos];
            let mut content_start = pos + 1;
            let len = read_len(cert_der, &mut content_start);
            let elem_end = content_start + len;
            if tag == 0x30 {
                // 在 SEQUENCE 元素内找 BIT STRING(0x03)，即 SPKI 的公钥位串。
                let inner = &cert_der[content_start..elem_end];
                let mut p = 1usize; // 跳过 inner 的 tag
                let _ = read_len(inner, &mut p);
                while p < inner.len() {
                    let t2 = inner[p];
                    let mut s2 = p + 1;
                    let l2 = read_len(inner, &mut s2);
                    if t2 == 0x03 && l2 > 1 && inner[s2] == 0 {
                        // 找到 SPKI：外层整个 SEQUENCE 元素。
                        return cert_der[pos..elem_end].to_vec();
                    }
                    p = s2 + l2;
                }
            }
            pos = elem_end;
        }
        panic!("SPKI not found in certificate DER");
    }
}
