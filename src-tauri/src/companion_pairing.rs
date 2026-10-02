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
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// 事件环形缓冲上限，与诊断日志口径一致。
const EVENT_LIMIT: usize = 200;

/// 握手协议版本前缀。MDP2 = 挑战-响应互信（M4-1）。
pub const PROTOCOL_TAG: &str = "MDP2";

/// 挑战 nonce 字节数（32 字节熵）。
const CHALLENGE_BYTES: usize = 32;

/// 单行消息硬上限（8 KiB）。
///
/// 配对监听期间端口对局域网开放，且服务端**不校验客户端身份**（互信靠握手阶段的
/// 挑战签名建立）——即任意同网主机都能完成 TLS 并发送数据。协议里最大的行是
/// `device_hello`（SPKI hex 约 180 字符 + 机型），远小于 8 KiB；设上限是为了
/// 让「不发换行、持续灌数据」无法把进程内存吃满（fail-closed：超限即断开并如实告知）。
const MAX_LINE_BYTES: usize = 8 * 1024;

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
    /// 常驻通道（M4-2）是否处于监听中：开启后已配对设备可免扫码直连。
    pub resident: bool,
    /// 最近事件（时间戳 + 文案；不含 token、不含屏幕数据）。
    pub events: Vec<String>,
    pub offer: Option<PairingOffer>,
}

#[derive(Default)]
pub struct PairingState {
    inner: Mutex<PairingInner>,
}

/// 伴侣端上行事件的转发钩子（X10-60）：收到 `files_changed` / `last_crash` 等
/// 需要到达前端的系统消息时调用。伴侣模块不感知 AppHandle，由调用方（lib.rs
/// setup 阶段）注入；payload 是已解析的完整 JSON。
pub type CompanionEventHook = Arc<dyn Fn(&serde_json::Value) + Send + Sync>;

#[derive(Default)]
struct PairingInner {
    phase: PairingPhase,
    /// 常驻通道监听中（与一次性扫码配对互斥，开启会先结束旧监听）。
    resident: bool,
    events: Vec<String>,
    offer: Option<PairingOffer>,
    shutdown: Option<oneshot::Sender<()>>,
    device_bridge: Option<DeviceBridge>,
    /// 当前活跃伴侣会话的下行通道（会话 id → 发送端）。
    /// 录制状态等桌面事件经此推给伴侣端；无活跃会话时为 None。
    session_out: Option<(u64, mpsc::UnboundedSender<String>)>,
    /// 会话计数器（配合 session_out 做只清自己的清除）。
    session_seq: u64,
    /// 常驻通道监听端口（None = 未开启）。随 paired_ok 下发给伴侣端持久化，
    /// 是免扫码重连的端口依据。
    resident_port: Option<u16>,
    /// 伴侣端上行事件转发钩子（X10-60，见 CompanionEventHook）。
    event_hook: Option<CompanionEventHook>,
}

/// 设备报到时触发的桥接回调（桌面侧注入）：入参为伴侣端来源 IP，返回要追加进
/// 事件流的文案。伴侣模块不感知 adb——镜像通道的自动连接逻辑（mDNS 匹配 +
/// adb connect）由调用方闭包实现。
pub type DeviceBridge = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

impl PairingState {
    /// 注入设备报到桥接回调；应在任何配对开始前完成（run() 启动时设置一次）。
    pub fn set_device_bridge(&self, bridge: DeviceBridge) {
        self.inner.lock().unwrap().device_bridge = Some(bridge);
    }

    /// 注入伴侣端上行事件转发钩子（setup 阶段设置一次，见 CompanionEventHook）。
    pub fn set_event_hook(&self, hook: CompanionEventHook) {
        self.inner.lock().unwrap().event_hook = Some(hook);
    }

    fn forward_event(&self, value: &serde_json::Value) {
        let hook = self.inner.lock().unwrap().event_hook.clone();
        if let Some(hook) = hook {
            hook(value);
        }
    }

    fn take_device_bridge(&self) -> Option<DeviceBridge> {
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
            resident: inner.resident,
            events: inner.events.clone(),
            offer: inner.offer.clone(),
        }
    }

    /// 向当前活跃的伴侣会话下发一行 JSON（无活跃会话或通道关闭返回 false，
    /// 调用方静默忽略即可——伴侣通道是附加信息，不构成错误）。
    ///
    /// 行协议以换行分帧：这里统一补齐尾部 `\n`（调用方只给 JSON 本体）。
    /// 缺换行的下行会让伴侣端 readLine 永远等不到行尾（2026-10-01 测试实锤：
    /// 广播「到了但黏在下一条消息上」，表现为客户端收不到推送）。
    pub fn broadcast_line(&self, line: impl Into<String>) -> bool {
        let mut line = line.into();
        if !line.ends_with('\n') {
            line.push('\n');
        }
        let sender = {
            let inner = self.inner.lock().unwrap();
            inner.session_out.as_ref().map(|(_, tx)| tx.clone())
        };
        match sender {
            Some(tx) => tx.send(line).is_ok(),
            None => false,
        }
    }

    /// 登记会话下行通道（握手成功后调用）；返回本次会话的 id，结束时凭 id 清除。
    fn register_session_out(&self, tx: mpsc::UnboundedSender<String>) -> u64 {
        let mut inner = self.inner.lock().unwrap();
        inner.session_seq = inner.session_seq.wrapping_add(1);
        let id = inner.session_seq;
        inner.session_out = Some((id, tx));
        id
    }

    /// 会话结束时清除下行通道：只清自己的（并发会话下避免误清新会话的通道）。
    fn unregister_session_out(&self, id: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.session_out.as_ref().map(|(sid, _)| *sid) == Some(id) {
            inner.session_out = None;
        }
    }

    /// 当前常驻监听端口（未开启为 None）。随 paired_ok 下发。
    pub fn resident_port(&self) -> Option<u16> {
        self.inner.lock().unwrap().resident_port
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
#[derive(Clone)]
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

/// hex 解码；奇数长度直接判为非法（不补齐、不猜测）。
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
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
        version: 2,
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
        run_accept_loop(listener, identity, token, store, shared, shutdown_rx, false).await;
    });
    drop(handle);

    Ok(offer)
}

/// 常驻监听的**默认固定端口**。伴侣端免扫码重连按这个端口直连（见
/// `begin_resident`）：固定端口让重连跨桌面端重启仍然有效；被占用时回退
/// 随机端口并如实告知（此时手机需要重新扫码一次以学到新端口）。
pub const RESIDENT_DEFAULT_PORT: u16 = 47017;

/// 启动常驻通道（M4-2）：与一次性扫码配对同构的监听器，但：
/// - 不出二维码、不发一次性 token（offer 保持 None）；
/// - 只接受 `MDP2 RECONNECT <pairing_id>` 的免扫码重连（首配必须走扫码，
///   信任锚是二维码出带校验，常驻通道没有这个环节，不能降低门槛）；
/// - 并发处理多个会话（每连接独立任务），支持心跳长连（见 serve_session）；
/// - 优先绑定固定端口 [`RESIDENT_DEFAULT_PORT`]，伴侣端重连按它直连。
///
/// 返回监听端口。
pub fn begin_resident(
    state: &Arc<PairingState>,
    identity_dir: &Path,
    store: PairedStore,
) -> Result<u16, crate::AppError> {
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

    let std_listener =
        std::net::TcpListener::bind(("0.0.0.0", RESIDENT_DEFAULT_PORT))
            .or_else(|_| std::net::TcpListener::bind("0.0.0.0:0"))
            .map_err(|e| crate::AppError {
                code: "pairing_bind_failed",
                message: format!("常驻通道监听失败：{e}"),
                recovery: "检查系统防火墙后重试。".into(),
            })?;
    let port = std_listener.local_addr().map_err(|e| crate::AppError {
        code: "pairing_bind_failed",
        message: format!("读取端口失败：{e}"),
        recovery: "重试一次。".into(),
    })?
    .port();
    let used_fallback = port != RESIDENT_DEFAULT_PORT;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    {
        let mut inner = state.inner.lock().unwrap();
        inner.resident = true;
        inner.resident_port = Some(port);
        inner.shutdown = Some(shutdown_tx);
        inner.phase = PairingPhase::Listening;
    }
    state.push_event(format!(
        "常驻通道已开启（端口 {port}{}）：此后扫码配对的伴侣设备可免扫码直连（此前在关闭状态下配过对的手机需重新扫一次码）",
        if used_fallback {
            format!("，默认端口 {RESIDENT_DEFAULT_PORT} 被占用已回退")
        } else {
            String::new()
        },
    ));

    let shared = state.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let listener = match set_nonblocking_and_convert(std_listener) {
            Ok(l) => l,
            Err(e) => {
                shared.push_event(format!("监听器转换失败：{e}"));
                shared.set_phase(PairingPhase::Idle);
                let mut inner = shared.inner.lock().unwrap();
                inner.resident = false;
                inner.resident_port = None;
                return;
            }
        };
        run_accept_loop(listener, identity, String::new(), store, shared, shutdown_rx, true).await;
    });
    drop(handle);

    // 已有活跃伴侣会话（扫码配对刚完成）：把端口推给手机持久化，
    // 会话断开后它就能按新端口免扫码重连。
    state.broadcast_line(format!("{{\"type\":\"resident_port\",\"port\":{port}}}"));

    Ok(port)
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

/// 接受循环：一次性配对模式串行处理会话（POC 足够）；常驻通道模式并发处理
/// （每连接独立任务——心跳长连会一直占用会话，串行会挡住后续重连）。
/// shutdown 通道随时可打断。
async fn run_accept_loop(
    listener: tokio::net::TcpListener,
    identity: PairingIdentity,
    token_expected: String,
    store: PairedStore,
    state: Arc<PairingState>,
    mut shutdown_rx: oneshot::Receiver<()>,
    resident: bool,
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
                if resident {
                    state.push_event("常驻通道已关闭");
                } else {
                    state.push_event("配对已手动结束");
                }
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
                if resident {
                    // 常驻模式：并发会话，监听器不被单个长连占用；phase 保持
                    // Listening（会话进展看事件流，避免与扫码配对的状态语义混淆）。
                    // 用 tokio::spawn 跟随 accept loop 所在运行时（生产 = tauri
                    // async runtime，测试 = 测试 runtime），保证会话一定会被轮询。
                    state.push_event(format!("常驻通道收到连接 {peer}"));
                    let acceptor = acceptor.clone();
                    let store = store.clone();
                    let session_state = state.clone();
                    let peer_ip = peer.ip().to_string();
                    tokio::spawn(async move {
                        match acceptor.accept(stream).await {
                            Ok(tls) => {
                                serve_session(tls, "", &store, &session_state, peer_ip, true).await
                            }
                            Err(e) => session_state.push_event(format!("TLS 握手失败：{e}")),
                        }
                    });
                    continue;
                }
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
                            Ok(tls) => serve_session(tls, &token_expected, &store, &state, peer.ip().to_string(), false).await,
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
///
/// `resident`（M4-2）：常驻通道会话——只接受 RECONNECT；握手成功后登记
/// 下行通道（桌面事件可推给伴侣端），并响应 `{"type":"ping"}` 心跳
/// （回 pong；300 秒没有任何入站数据则判死连接断开）。
///
/// 并发说明：整条 TLS 流由本任务独占（**不做** `tokio::io::split`——其 BiLock
/// 会在「读挂起等待客户端数据」期间挡住写半边，桌面事件永远送不出去，
/// 2026-10-01 测试实锤）。读与下行写用 select! 在单任务内轮转。
async fn serve_session(
    tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    token_expected: &str,
    store: &PairedStore,
    state: &PairingState,
    peer_ip: String,
    resident: bool,
) {
    // BufReader 同时提供 AsyncBufRead（握手/消息读）与 AsyncWrite（透传写）。
    let mut stream = BufReader::new(tls);

    // 第一行："MDP2 <token>"（首配，扫码）或 "MDP2 RECONNECT <pairing_id>"（互信重连）。
    let hello = match read_line_bounded(&mut stream, std::time::Duration::from_secs(10), MAX_LINE_BYTES).await
    {
        Ok(line) => line,
        Err(err) => {
            state.push_event(format!("握手未完成（{}）", err.describe()));
            return;
        }
    };
    let mut parts = hello.split_whitespace();
    let proto = parts.next().unwrap_or("");
    if proto != PROTOCOL_TAG {
        let _ = stream.write_all(b"{\"type\":\"rejected\"}\n").await;
        state.push_event("协议版本不支持，连接已拒绝");
        return;
    }

    // MDP2 互信握手：验证对端持有其申报公钥对应的私钥，才允许进入会话。
    let handshake = match parts.next().unwrap_or("") {
        word if !resident && word == token_expected => {
            establish_trust(EstablishMode::FreshPairing, store, &mut stream, state).await
        }
        "RECONNECT" => {
            let pairing_id = parts.next().unwrap_or("").to_string();
            establish_trust(EstablishMode::Reconnect { pairing_id }, store, &mut stream, state).await
        }
        _ => {
            let _ = stream.write_all(b"{\"type\":\"rejected\"}\n").await;
            if resident {
                state.push_event("常驻通道只接受已配对设备的免扫码重连，连接已拒绝");
            } else {
                state.push_event("配对码校验失败，连接已拒绝");
            }
            return;
        }
    };
    let Some(device) = handshake else { return }; // 失败原因已写入事件流。

    // paired_ok 带上常驻端口（开启时）：伴侣端持久化后即可免扫码直连。
    // 有活跃会话但常驻未开启时下发 null——伴侣端清除旧端口，避免拿着过期
    // 端口反复重连失败（fail-closed：宁可不连也不乱试）。
    let paired_ok = match state.resident_port() {
        Some(port) => format!(
            "{{\"type\":\"paired_ok\",\"pairing_id\":\"{}\",\"resident_port\":{port}}}\n",
            device.pairing_id
        ),
        None => format!(
            "{{\"type\":\"paired_ok\",\"pairing_id\":\"{}\",\"resident_port\":null}}\n",
            device.pairing_id
        ),
    };
    let _ = stream.write_all(paired_ok.as_bytes()).await;
    state.push_event(format!(
        "设备报到：{}（互信会话已建立，来源 {}，{}）",
        device.model,
        peer_ip,
        if resident { "免扫码重连" } else { "扫码配对" },
    ));

    // 登记下行通道：此后桌面事件（录制状态等）经 broadcast_line 推给伴侣端，
    // 由下面的 select! 循环就地写出。
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
    let session_id = state.register_session_out(out_tx.clone());

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

    let mut ended = false;
    while !ended {
        tokio::select! {
            biased;
            // 下行（桌面 → 伴侣）：优先送出，避免事件积压。
            line = out_rx.recv() => {
                match line {
                    Some(line) => {
                        // tokio-rustls 的 write 可能滞留在会话缓冲，必须显式
                        // flush 才落 TCP（漏掉 flush = 客户端永远收不到）。
                        if stream.write_all(line.as_bytes()).await.is_err()
                            || stream.flush().await.is_err()
                        {
                            state.push_event("会话结束（下行写出失败）");
                            ended = true;
                        }
                    }
                    // 发送端全部丢弃：本函数持有的 out_tx 尚在，理论不发生；
                    // 真发生说明通道已废，结束会话。
                    None => ended = true,
                }
            }
            // 入站（伴侣 → 桌面）：300 秒静默判死。
            result = read_line_bounded(&mut stream, std::time::Duration::from_secs(300), MAX_LINE_BYTES) => {
                match result {
                    Ok(line) => {
                        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                            state.push_event("收到无法解析的行（已忽略）");
                            continue;
                        };
                        match value.get("type").and_then(|t| t.as_str()) {
                            Some("bye") => {
                                state.push_event("伴侣端正常结束会话");
                                ended = true;
                            }
                            // 心跳（M4-2）：回 pong 即可，内容不进事件流（15s 一条会刷屏）。
                            // 写出失败（含 flush 滞留）⇒ 会话结束；成功则无事可做，
                            // 由紧随的空臂承接（不得落入「未知消息类型」）。
                            Some("ping")
                                if stream.write_all(PONG_LINE.as_bytes()).await.is_err()
                                    || stream.flush().await.is_err() =>
                            {
                                state.push_event("会话结束（心跳写出失败）");
                                ended = true;
                            }
                            Some("ping") => {}
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
                            // 发送区有新文件（X10-60）：转发给前端刷新工具页列表。
                            Some("files_changed") => {
                                state.push_event("伴侣端发送区有文件更新");
                                state.forward_event(&value);
                            }
                            // 崩溃堆栈上报（X10-60）：本地点对点展示，不落盘、不上云。
                            Some("last_crash") => {
                                state.push_event("伴侣端上报了崩溃记录（仅本机界面展示）");
                                state.forward_event(&value);
                            }
                            // 手机通知转发（X10-66 一期）：只进前端通知面板。事件流
                            // 只写「来了一条」——标题/内容不入事件流、不落盘、不进日志。
                            Some("notification") => {
                                state.push_event("伴侣端转来一条手机通知");
                                state.forward_event(&value);
                            }
                            Some(other) => {
                                state.push_event(format!("收到未知消息类型 {other}（已忽略）"));
                            }
                            None => {}
                        }
                    }
                    Err(err) => {
                        state.push_event(format!("会话结束（{}）", err.describe()));
                        ended = true;
                    }
                }
            }
        }
    }
    // 会话收尾：只清自己的下行通道；丢弃本地发送端后，out_rx.recv() 归 None。
    state.unregister_session_out(session_id);
    drop(out_tx);
}

/// 互信建立模式：首配（凭一次性 token）或重连（凭 pairing_id）。
enum EstablishMode {
    FreshPairing,
    Reconnect { pairing_id: String },
}

/// 心跳应答（M4-2）。
const PONG_LINE: &str = "{\"type\":\"pong\"}\n";

/// MDP2 互信握手：
/// 1. 对端申报身份（首配：`device_hello{model,pubkey}`；重连：查台账取公钥）；
/// 2. 桌面发 `challenge{nonce}`，对端回 `challenge_response{sig}`（ECDSA P-256，DER）；
/// 3. 验签通过 → 台账登记（首配插入 / 重连刷新 last_seen）→ 返回设备信息。
///
/// 任何一步失败都写入事件流并返回 None（调用方直接断开，不发 paired_ok）。
/// 读写共用同一条流（读一行、写一行交替，无需 split）。
async fn establish_trust<S>(
    mode: EstablishMode,
    store: &PairedStore,
    stream: &mut S,
    state: &PairingState,
) -> Option<PairedCompanion>
where
    S: tokio::io::AsyncBufRead + tokio::io::AsyncWrite + Unpin,
{
    let is_fresh = matches!(mode, EstablishMode::FreshPairing);
    let _ = stream
        .write_all(b"{\"type\":\"welcome\",\"protocol\":\"MDP2\"}\n")
        .await;

    // 已配对台账里的记录（重连路径用）；申报的公钥 hex 与机型。
    let (pubkey_hex, model, stored) = match mode {
        EstablishMode::FreshPairing => {
            let hello = match read_json_line(stream, state, "未收到设备报到").await {
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
                    let _ = stream.write_all(b"{\"type\":\"rejected\"}\n").await;
                    return None;
                }
            }
        }
    };

    // 公钥有效性：能解析成 P-256 公钥才继续（防止把垃圾字节写进台账）。
    if verifying_key_from_hex(&pubkey_hex).is_none() {
        let _ = stream.write_all(b"{\"type\":\"rejected\"}\n").await;
        state.push_event("设备公钥格式不正确，互信未建立");
        return None;
    }

    // 挑战：32 字节随机 nonce（hex 编码下发），要求对端用其私钥签名。
    let mut nonce = vec![0u8; CHALLENGE_BYTES];
    getrandom::getrandom(&mut nonce).expect("系统熵源不可用");
    let nonce_hex = hex_lower(&nonce);
    let _ = stream
        .write_all(format!("{{\"type\":\"challenge\",\"nonce\":\"{nonce_hex}\"}}\n").as_bytes())
        .await;

    let response = match read_json_line(stream, state, "未收到挑战签名").await {
        Some(v) => v,
        None => return None,
    };
    if response.get("type").and_then(|t| t.as_str()) != Some("challenge_response") {
        state.push_event("挑战响应消息格式不正确，互信未建立");
        return None;
    }
    let sig = response.get("sig").and_then(|s| s.as_str()).unwrap_or("");
    if !verify_challenge(&pubkey_hex, &nonce, sig) {
        // 显式拒绝：伴侣端据此显示「电脑没有接受本机的身份证明」，
        // 而不是等到连接被关掉后误报「电脑没有回复」。
        let _ = stream.write_all(b"{\"type\":\"rejected\"}\n").await;
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

/// 带超时读一行 JSON（10 秒）；失败原因写入事件流并返回 None。
async fn read_json_line<L>(reader: &mut L, state: &PairingState, what: &str) -> Option<serde_json::Value>
where
    L: tokio::io::AsyncBufRead + Unpin,
{
    match read_line_bounded(reader, std::time::Duration::from_secs(10), MAX_LINE_BYTES).await {
        Ok(line) => serde_json::from_str::<serde_json::Value>(&line).ok(),
        Err(err) => {
            state.push_event(format!("{what}（{}）", err.describe()));
            None
        }
    }
}

/// 读一行的失败原因。区分「超时 / 对端断开 / 超长 / 非文本 / IO 错误」，
/// 便于在事件流里给出可照做的说明，而不是统一成「连接失败」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineError {
    Closed,
    TimedOut,
    TooLong,
    NotUtf8,
    Io,
}

impl LineError {
    fn describe(&self) -> String {
        match self {
            LineError::Closed => "对端提前断开".into(),
            LineError::TimedOut => "等待超时".into(),
            LineError::TooLong => format!("对端消息超过 {MAX_LINE_BYTES} 字节上限，已断开"),
            LineError::NotUtf8 => "消息不是有效文本".into(),
            LineError::Io => "读取出错".into(),
        }
    }
}

/// 带超时、带上限读一行（UTF-8）。泛型同时服务生产路径与测试客户端。
///
/// 与 `read_line` 的差别：**先看缓冲区里有没有换行，再看累计长度**——超过
/// `max_bytes` 立即返回 `TooLong`，不会把无换行的长数据一直堆进内存（同网任意
/// 主机都能连上这个端口，上限是必需的 fail-closed 措施，见 `MAX_LINE_BYTES`）。
async fn read_line_bounded<L>(
    reader: &mut L,
    timeout: std::time::Duration,
    max_bytes: usize,
) -> Result<String, LineError>
where
    L: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::AsyncBufReadExt as _;

    let read = async {
        let mut out: Vec<u8> = Vec::new();
        loop {
            let mut line_complete = false;
            let mut too_long = false;
            let chunk_len: usize;
            {
                // 借用结束于本块：把该拿的都取出来（长度/是否含换行/内容），
                // 之后再 consume，避免同时持有 `&mut reader` 与其借出的切片。
                let available = reader.fill_buf().await.map_err(|_| LineError::Io)?;
                if available.is_empty() {
                    return Err(LineError::Closed);
                }
                chunk_len = match available.iter().position(|b| *b == b'\n') {
                    Some(index) => {
                        line_complete = true;
                        index + 1
                    }
                    None => available.len(),
                };
                if out.len() + chunk_len > max_bytes {
                    too_long = true;
                } else {
                    out.extend_from_slice(&available[..chunk_len]);
                }
            }
            reader.consume(chunk_len);
            if too_long {
                return Err(LineError::TooLong);
            }
            if line_complete {
                break;
            }
        }
        String::from_utf8(out).map_err(|_| LineError::NotUtf8)
    };

    match tokio::time::timeout(timeout, read).await {
        Ok(result) => result,
        Err(_) => Err(LineError::TimedOut),
    }
}

/// 结束当前配对（幂等）：扫码配对与常驻通道共用同一关停通道，谁在监听就关谁。
pub fn end_pairing(state: &Arc<PairingState>) {
    let mut inner = state.inner.lock().unwrap();
    if let Some(tx) = inner.shutdown.take() {
        let _ = tx.send(());
    }
    inner.phase = PairingPhase::Idle;
    inner.offer = None;
    inner.resident = false;
    inner.resident_port = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt as _;

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
    fn event_hook_receives_forwarded_companion_events() {
        // X10-60：伴侣端上行事件经钩子转发；未设置钩子时静默忽略不 panic。
        let state = PairingState::default();
        state.forward_event(&serde_json::json!({"type": "files_changed"}));

        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink = std::sync::Arc::clone(&received);
        state.set_event_hook(Arc::new(move |value: &serde_json::Value| {
            sink.lock().unwrap().push(
                value.get("type").and_then(|t| t.as_str()).unwrap_or("").to_owned(),
            );
        }));
        state.forward_event(&serde_json::json!({"type": "files_changed"}));
        state.forward_event(&serde_json::json!({"type": "last_crash", "stack": "boom"}));
        assert_eq!(
            *received.lock().unwrap(),
            vec!["files_changed".to_owned(), "last_crash".to_owned()]
        );
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

    /// 签名的序列化编码。真机（Java `SHA256withECDSA`）用 DER；Rust 侧的
    /// `to_vec()` 是定宽 r||s。两条路都必须能验签，故测试两种都覆盖。
    #[derive(Clone, Copy, PartialEq)]
    enum SigEncoding {
        FixedWidth,
        Der,
    }

    /// 起一个被测配对服务端（随机端口）。返回值里的 `_alive` 是关停通道的持有者，
    /// 必须活到测试结束——丢弃即触发优雅关停（tokio 语义）。
    async fn spawn_pairing_server(
        state: &Arc<PairingState>,
        workdir: &std::path::Path,
    ) -> (u16, String, oneshot::Sender<()>) {
        let (identity, _) = PairingIdentity::load_or_create(workdir).unwrap();
        let store = PairedStore::new(workdir);
        let token = generate_pairing_token();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();

        let loop_state = state.clone();
        let loop_token = token.clone();
        let (tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_accept_loop(listener, identity, loop_token, store, loop_state, rx, false).await;
        });
        (port, token, tx)
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
    /// 返回服务端最后一条回复（paired_ok / rejected 或无）。
    async fn fresh_pair_client(
        port: u16,
        fingerprint: &str,
        token: &str,
        model: &str,
        signing: &p256::ecdsa::SigningKey,
        sign_wrong_data: bool,
        encoding: SigEncoding,
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
        let sig_hex = match encoding {
            SigEncoding::FixedWidth => hex_lower(signature.to_vec().as_slice()),
            // 真机（Android Keystore / Java SHA256withECDSA）走这条。
            SigEncoding::Der => hex_lower(signature.to_der().to_vec().as_slice()),
        };
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
        let (port, token, _alive) = spawn_pairing_server(&state, &workdir).await;

        // 错误签名必须被拒（对端不持有申报身份），且要给对端一条**显式**拒绝：
        // 否则伴侣端只能等到连接关闭，显示成「电脑没有回复配对结果」。
        let signing = test_device_key();
        let rejected = fresh_pair_client(
            port,
            &fingerprint_of(&workdir),
            &token,
            "Bad-Device",
            &signing,
            true,
            SigEncoding::FixedWidth,
        )
        .await
        .expect("服务端必须显式拒绝错误签名");
        let rejected: serde_json::Value = serde_json::from_str(&rejected).unwrap();
        assert_eq!(rejected["type"], "rejected");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert!(state.status().events.iter().any(|e| e.contains("挑战签名验证失败")));

        // 正确签名：完整握手 + 统计流 + bye。
        let paired = match fresh_pair_client(
            port,
            &fingerprint_of(&workdir),
            &token,
            "POC-Test",
            &signing,
            false,
            SigEncoding::FixedWidth,
        )
        .await
        {
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

    /// 真机（Android Keystore）用 Java `SHA256withECDSA` 签名，输出 **DER** 编码，
    /// 而 Rust 测试客户端默认发定宽 r||s——两条编码都必须验签通过。
    /// 这条用例专门锁住「实际生产路径」（DER），避免只测到自家客户端的写法。
    #[tokio::test]
    async fn der_encoded_signature_is_accepted() {
        let workdir = std::env::temp_dir().join(format!("md-der-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());
        let (port, token, _alive) = spawn_pairing_server(&state, &workdir).await;

        let reply = fresh_pair_client(
            port,
            &fingerprint_of(&workdir),
            &token,
            "DER-Device",
            &test_device_key(),
            false,
            SigEncoding::Der,
        )
        .await
        .expect("DER 编码签名必须被接受");
        let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["type"], "paired_ok");
        assert_eq!(PairedStore::new(&workdir).load().len(), 1);
        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// 单行上限的单元级证明：无换行的长数据在**等于上限**时尚可，一旦超过即
    /// 立刻返回 `TooLong`（而不是无限堆进内存等换行）。
    #[tokio::test]
    async fn read_line_bounded_stops_at_the_cap() {
        // 正常一行。
        let mut reader = BufReader::new(&b"MDP2 AAAA\n"[..]);
        let line = read_line_bounded(&mut reader, std::time::Duration::from_secs(1), 64)
            .await
            .unwrap();
        assert_eq!(line, "MDP2 AAAA\n");

        // 恰好等于上限（63 字节 + 换行）通过。
        let mut exact = vec![b'C'; 63];
        exact.push(b'\n');
        let mut reader = BufReader::new(exact.as_slice());
        let line = read_line_bounded(&mut reader, std::time::Duration::from_secs(1), 64)
            .await
            .unwrap();
        assert_eq!(line.len(), 64);

        // 超限（无换行）立即失败。
        let big = vec![b'B'; 4096];
        let mut reader = BufReader::new(big.as_slice());
        let err = read_line_bounded(&mut reader, std::time::Duration::from_secs(1), 64)
            .await
            .unwrap_err();
        assert_eq!(err, LineError::TooLong);

        // 对端断开（读到 EOF）如实区分，不与超限混淆。
        let mut reader = BufReader::new(&b""[..]);
        let err = read_line_bounded(&mut reader, std::time::Duration::from_secs(1), 64)
            .await
            .unwrap_err();
        assert_eq!(err, LineError::Closed);
    }

    /// 端到端：配对端口对同网开放且不校验客户端身份，因此「不发换行、持续灌数据」
    /// 必须被**快速**掐断（在 10 秒读超时之前），且事件流如实说明原因。
    #[tokio::test]
    async fn an_oversized_line_ends_the_session_before_the_timeout() {
        use tokio::io::AsyncWriteExt;

        let workdir = std::env::temp_dir().join(format!("md-too-long-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());
        let (port, _token, _alive) = spawn_pairing_server(&state, &workdir).await;

        let tls = tls_connect(port, &fingerprint_of(&workdir)).await;
        let (_read_half, mut write_half) = tokio::io::split(tls);

        let started = std::time::Instant::now();
        // 2 MiB、无换行。服务端一旦超限就断开，写入可能因此失败——忽略即可。
        let payload = vec![b'A'; 2 * 1024 * 1024];
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            write_half.write_all(&payload),
        )
        .await;

        let mut seen = false;
        for _ in 0..30 {
            if state.status().events.iter().any(|e| e.contains("字节上限")) {
                seen = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(seen, "事件流应如实告知超限：{:#?}", state.status().events);
        assert!(
            started.elapsed() < std::time::Duration::from_secs(6),
            "超限必须在 10 秒读超时之前触发，实测 {:?}",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(&workdir);
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
            run_accept_loop(listener, identity, "CORRECTTOKEN1234".into(), store, loop_state, rx, false).await;
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

    /// 常驻通道客户端（M4-2）：发 `MDP2 RECONNECT <id>`，走挑战-响应，
    /// 之后可选发一条 ping 并读回应（心跳）。返回服务端最后读到的行。
    async fn reconnect_client(
        port: u16,
        fingerprint: &str,
        pairing_id: &str,
        signing: &p256::ecdsa::SigningKey,
        ping_after_paired: bool,
    ) -> Option<String> {
        use p256::ecdsa::signature::{SignatureEncoding, Signer};
        use tokio::io::AsyncWriteExt;

        let tls = tls_connect(port, fingerprint).await;
        let (read_half, mut write_half) = tokio::io::split(tls);
        let mut lines = BufReader::new(read_half).lines();

        write_half
            .write_all(format!("{PROTOCOL_TAG} RECONNECT {pairing_id}\n").as_bytes())
            .await
            .unwrap();
        let welcome = lines.next_line().await.unwrap().unwrap();
        assert!(welcome.contains("\"welcome\""), "got {welcome}");

        let challenge: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(challenge["type"], "challenge");
        let nonce = hex_decode(challenge["nonce"].as_str().unwrap()).unwrap();
        let signature: p256::ecdsa::Signature = signing.sign(nonce.as_slice());
        let sig_hex = hex_lower(signature.to_der().to_vec().as_slice());
        write_half
            .write_all(format!("{{\"type\":\"challenge_response\",\"sig\":\"{sig_hex}\"}}\n").as_bytes())
            .await
            .unwrap();

        let verdict = lines.next_line().await.ok().flatten()?;
        if !ping_after_paired {
            return Some(verdict);
        }
        assert!(verdict.contains("paired_ok"), "got {verdict}");
        write_half.write_all(b"{\"type\":\"ping\"}\n").await.unwrap();
        lines.next_line().await.ok().flatten()
    }

    /// 常驻通道（M4-2）端到端：首配 token 在常驻通道被拒（降门槛即拒）；
    /// 台账内设备的 RECONNECT 握手通过；paired 后的 ping 得到 pong。
    #[tokio::test]
    async fn resident_channel_reconnect_ping_pong_and_fresh_reject() {
        use pkcs8::EncodePublicKey as _;
        use tokio::io::AsyncWriteExt;

        let workdir = std::env::temp_dir().join(format!("md-resident-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());

        // 预置台账：测试身份的 pairing_id 与公钥。
        let signing = test_device_key();
        let spki = signing.verifying_key().to_public_key_der().unwrap();
        let pubkey_hex = hex_lower(spki.as_bytes());
        let store = PairedStore::new(&workdir);
        store
            .upsert(PairedCompanion {
                pairing_id: "RESIDENTTEST0001".into(),
                model: "Resident-Test".into(),
                pubkey_hex,
                added_at: 1,
                last_seen: 1,
            })
            .unwrap();

        let (identity, _) = PairingIdentity::load_or_create(&workdir).unwrap();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();
        let loop_state = state.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        // 与 begin_resident 一致：进入常驻监听时置 resident 标志（本测试绕过
        // begin_resident 直接起 loop，标志要自己置）。
        loop_state.inner.lock().unwrap().resident = true;
        tokio::spawn(async move {
            // 常驻模式：没有 token（空串），RECONNECT-only。
            run_accept_loop(listener, identity, String::new(), store, loop_state, rx, true).await;
        });

        // ① 常驻通道拒绝一次性 token 首配（显式 rejected）。
        let verifier = Arc::new(FpVerifier { expected_fp: fingerprint_of(&workdir) });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
        let tls = connector
            .connect(server_name(), tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap())
            .await
            .unwrap();
        let (mut read_half, mut write_half) = tokio::io::split(tls);
        write_half.write_all(b"MDP2 SOMETOKEN12345678\n").await.unwrap();
        let mut buf = vec![0u8; 128];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), read_half.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains("rejected"));
        assert!(state.status().events.iter().any(|e| e.contains("常驻通道只接受已配对设备")));

        // ② 台账内设备 RECONNECT 免扫码重连成功。
        let paired = reconnect_client(port, &fingerprint_of(&workdir), "RESIDENTTEST0001", &signing, false)
            .await
            .expect("RECONNECT 必须成功");
        let paired: serde_json::Value = serde_json::from_str(&paired).unwrap();
        assert_eq!(paired["type"], "paired_ok");
        assert_eq!(paired["pairing_id"], "RESIDENTTEST0001");

        // ③ 心跳：paired 后发 ping，服务端回 pong。
        let pong = reconnect_client(port, &fingerprint_of(&workdir), "RESIDENTTEST0001", &signing, true)
            .await
            .expect("心跳会话必须成功");
        let pong: serde_json::Value = serde_json::from_str(&pong).unwrap();
        assert_eq!(pong["type"], "pong");

        // ④ 状态如实反映常驻监听。
        assert!(state.status().resident);
        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// 桌面事件下发（M4-3）：broadcast_line 把行送进活跃会话，
    /// 伴侣端（测试客户端）能读到；无活跃会话时返回 false 不报错。
    #[tokio::test]
    async fn broadcast_reaches_the_active_session() {
        use pkcs8::EncodePublicKey as _;

        let workdir = std::env::temp_dir().join(format!("md-broadcast-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());

        let signing = test_device_key();
        let spki = signing.verifying_key().to_public_key_der().unwrap();
        let store = PairedStore::new(&workdir);
        store
            .upsert(PairedCompanion {
                pairing_id: "BROADCASTTEST01".into(),
                model: "Broadcast-Test".into(),
                pubkey_hex: hex_lower(spki.as_bytes()),
                added_at: 1,
                last_seen: 1,
            })
            .unwrap();

        // 无活跃会话时广播是静默 no-op。
        assert!(!state.broadcast_line("{\"type\":\"recording\",\"active\":true}"));

        let (identity, _) = PairingIdentity::load_or_create(&workdir).unwrap();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();
        let loop_state = state.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_accept_loop(listener, identity, String::new(), store, loop_state, rx, true).await;
        });

        // 客户端保持会话，同时读服务端推送。先完整走完 RECONNECT 握手
        // （welcome → challenge → 签名 → paired_ok），下行通道在握手成功后才登记。
        let tls = tls_connect(port, &fingerprint_of(&workdir)).await;
        let (read_half, mut write_half) = tokio::io::split(tls);
        let mut lines = BufReader::new(read_half).lines();
        write_half
            .write_all(b"MDP2 RECONNECT BROADCASTTEST01\n")
            .await
            .unwrap();
        let welcome = lines.next_line().await.unwrap().unwrap();
        assert!(welcome.contains("welcome"), "got {welcome}");
        let challenge: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(challenge["type"], "challenge");
        use p256::ecdsa::signature::{SignatureEncoding, Signer};
        let sig: p256::ecdsa::Signature = signing.sign(hex_decode(challenge["nonce"].as_str().unwrap()).unwrap().as_slice());
        write_half
            .write_all(format!("{{\"type\":\"challenge_response\",\"sig\":\"{}\"}}\n", hex_lower(sig.to_der().to_vec().as_slice())).as_bytes())
            .await
            .unwrap();
        let paired_line = lines.next_line().await.unwrap().unwrap();
        assert!(paired_line.contains("paired_ok"), "got {paired_line}");

        assert!(state.broadcast_line("{\"type\":\"recording\",\"active\":true}"));
        let pushed = match tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await {
            Ok(Ok(Some(line))) => line,
            other => {
                eprintln!("推送读取结果：{other:?}");
                eprintln!("服务端事件流：{:#?}", state.status().events);
                panic!("必须收到服务端推送");
            }
        };
        assert!(pushed.contains("\"recording\""), "got {pushed}");
        let _ = std::fs::remove_dir_all(&workdir);
    }

    /// 通知转发（X10-66 一期）端到端：伴侣端发 `notification` 行，事件钩子收到
    /// 完整 JSON；事件流只出现通用文案——通知标题/内容不得写进事件流（不落盘）。
    #[tokio::test]
    async fn notification_from_companion_reaches_the_event_hook() {
        use pkcs8::EncodePublicKey as _;
        use tokio::io::AsyncWriteExt;

        let workdir = std::env::temp_dir().join(format!("md-notify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&workdir);
        let state = Arc::new(PairingState::default());

        let signing = test_device_key();
        let spki = signing.verifying_key().to_public_key_der().unwrap();
        let store = PairedStore::new(&workdir);
        store
            .upsert(PairedCompanion {
                pairing_id: "NOTIFYTEST0001".into(),
                model: "Notify-Test".into(),
                pubkey_hex: hex_lower(spki.as_bytes()),
                added_at: 1,
                last_seen: 1,
            })
            .unwrap();

        // 钩子接进通道，测试断言收到的通知。
        let (hook_tx, mut hook_rx) = mpsc::unbounded_channel::<serde_json::Value>();
        state.set_event_hook(Arc::new(move |value: &serde_json::Value| {
            let _ = hook_tx.send(value.clone());
        }));

        let (identity, _) = PairingIdentity::load_or_create(&workdir).unwrap();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();
        let loop_state = state.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        loop_state.inner.lock().unwrap().resident = true;
        tokio::spawn(async move {
            run_accept_loop(listener, identity, String::new(), store, loop_state, rx, true).await;
        });

        // RECONNECT 握手（与 broadcast 测试同构），然后上行一条通知。
        let tls = tls_connect(port, &fingerprint_of(&workdir)).await;
        let (read_half, mut write_half) = tokio::io::split(tls);
        let mut lines = BufReader::new(read_half).lines();
        write_half
            .write_all(b"MDP2 RECONNECT NOTIFYTEST0001\n")
            .await
            .unwrap();
        let _welcome = lines.next_line().await.unwrap().unwrap();
        let challenge: serde_json::Value =
            serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
        use p256::ecdsa::signature::{SignatureEncoding, Signer};
        let sig: p256::ecdsa::Signature = signing
            .sign(hex_decode(challenge["nonce"].as_str().unwrap()).unwrap().as_slice());
        write_half
            .write_all(
                format!(
                    "{{\"type\":\"challenge_response\",\"sig\":\"{}\"}}\n",
                    hex_lower(sig.to_der().to_vec().as_slice())
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let paired_line = lines.next_line().await.unwrap().unwrap();
        assert!(paired_line.contains("paired_ok"), "got {paired_line}");

        write_half
            .write_all(
                // 行协议以换行分帧：缺尾部 \n 服务端永远等不到行尾（黏包坑实锤）。
                r#"{"type":"notification","pkg":"com.example.chat","app":"聊天","title":"机密标题","text":"机密内容","posted":1727840000000}"#
                    .as_bytes(),
            )
            .await
            .unwrap();
        // 行协议以换行分帧：缺尾部 \n 服务端永远等不到行尾。
        write_half.write_all(b"\n").await.unwrap();
        // tokio-rustls 的写可能滞留在会话缓冲，不 flush 数据不落 TCP。
        write_half.flush().await.unwrap();

        let received = match tokio::time::timeout(std::time::Duration::from_secs(10), hook_rx.recv())
            .await
        {
            Ok(v) => v.expect("钩子通道未关闭"),
            Err(_) => {
                eprintln!("服务端事件流：{:#?}", state.status().events);
                panic!("钩子必须收到通知");
            }
        };
        assert_eq!(received["type"], "notification");
        assert_eq!(received["title"], "机密标题");
        assert_eq!(received["pkg"], "com.example.chat");

        // 事件流只允许通用文案，标题/内容不得出现（给用户看的一侧，也是落盘面）。
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let events = state.status().events;
        assert!(events.iter().any(|e| e.contains("伴侣端转来一条手机通知")));
        assert!(
            events.iter().all(|e| !e.contains("机密标题") && !e.contains("机密内容")),
            "通知内容不得进入事件流：{events:#?}"
        );
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
