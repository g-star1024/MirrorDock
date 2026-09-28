//! C4-01 伴侣 App 同网加密会话 POC（桌面侧）。
//!
//! 设计要点（与威胁模型 TM-005 对齐）：
//! - 出带校验：一次性配对二维码携带 `token` 与服务器证书 SPKI 的 SHA-256 指纹，
//!   伴侣 App 用指纹锁定服务器身份，同网中间人无法伪造。
//! - 一次性证书：每次配对现场生成自签证书（不落盘、不复用），私钥只在内存。
//! - 一次性 token：10 字节随机 → base32 16 字符；配对结束即失效。
//! - 会话内容只有握手与统计 JSON 行，不传屏幕帧（屏幕帧不入日志/不落盘铁律）。
//! - 状态机 idle → listening → connected → idle，事件环形缓冲供前端展示。

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;

/// 事件环形缓冲上限，与诊断日志口径一致。
const EVENT_LIMIT: usize = 200;

/// 握手协议版本前缀。
pub const PROTOCOL_TAG: &str = "MDP1";

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
    /// 服务器证书 SPKI 的 SHA-256（hex 小写）。
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
}

impl PairingState {
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

/// 生成一次性 token（10 字节熵 → base32 16 字符）。
pub fn generate_pairing_token() -> String {
    let mut bytes = [0u8; 10];
    getrandom::getrandom(&mut bytes).expect("系统熵源不可用");
    crate::licensing::base32_encode(&bytes)
}

/// 生成现场自签证书，返回 (cert_der, key_der, spki_sha256_hex)。
pub fn generate_ephemeral_cert() -> Result<(Vec<u8>, Vec<u8>, String), String> {
    let mut params = rcgen::CertificateParams::new(vec!["mirrordock-companion.local".into()])
        .map_err(|e| e.to_string())?;
    params.not_before = rcgen::date_time_ymd(2026, 1, 1);
    params.not_after = rcgen::date_time_ymd(2027, 1, 1);
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "MirrorDock Companion Pairing");
    let key_pair = rcgen::KeyPair::generate().map_err(|e| e.to_string())?;
    let cert = params.self_signed(&key_pair).map_err(|e| e.to_string())?;
    let cert_der = cert.der().as_ref().to_vec();
    let key_der = key_pair.serialize_der();
    // SPKI（SubjectPublicKeyInfo）DER——指纹锁定对象，与证书内公钥一致。
    let spki_der = key_pair.public_key_der();
    let mut hasher = Sha256::new();
    hasher.update(&spki_der);
    Ok((cert_der, key_der, hex_lower(&hasher.finalize())))
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
pub fn begin_pairing(state: &Arc<PairingState>) -> Result<PairingOffer, crate::AppError> {
    end_pairing(state);

    let (cert_der, key_der, fingerprint) =
        generate_ephemeral_cert().map_err(|e| crate::AppError {
            code: "pairing_cert_failed",
            message: format!("配对证书生成失败：{e}"),
            recovery: "重试一次；持续失败请通过帮助与诊断反馈。".into(),
        })?;
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
        run_accept_loop(listener, cert_der, key_der, token, shared, shutdown_rx).await;
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
    cert_der: Vec<u8>,
    key_der: Vec<u8>,
    token_expected: String,
    state: Arc<PairingState>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let certs = vec![CertificateDer::from(cert_der)];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der));
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
                            Ok(tls) => serve_session(tls, &token_expected, &state).await,
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
async fn serve_session(
    tls: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    token_expected: &str,
    state: &PairingState,
) {
    let (reader, mut writer) = tokio::io::split(tls);
    let mut lines = BufReader::new(reader).lines();

    // 第一行必须是 "MDP1 <token>"。
    let hello = match tokio::time::timeout(std::time::Duration::from_secs(10), lines.next_line()).await {
        Ok(Ok(Some(line))) => line,
        _ => {
            state.push_event("握手超时或对端提前断开");
            return;
        }
    };
    let mut parts = hello.split_whitespace();
    let (proto, token) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if proto != PROTOCOL_TAG || token != token_expected {
        let _ = writer.write_all(b"{\"type\":\"rejected\"}\n").await;
        state.push_event("配对码校验失败，连接已拒绝");
        return;
    }
    let _ = writer
        .write_all(b"{\"type\":\"welcome\",\"protocol\":\"MDP1\"}\n")
        .await;
    state.push_event("配对成功，加密会话已建立");

    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(300), lines.next_line()).await {
            Ok(Ok(Some(line))) => {
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
                        let model = value.get("model").and_then(|m| m.as_str()).unwrap_or("unknown");
                        state.push_event(format!("设备报到：{model}"));
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
            _ => {
                state.push_event("会话结束（超时或断开）");
                return;
            }
        }
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
    fn ephemeral_cert_has_spki_fingerprint() {
        let (cert_der, key_der, fp) = generate_ephemeral_cert().unwrap();
        assert!(!cert_der.is_empty() && !key_der.is_empty());
        assert_eq!(fp.len(), 64);
        // 每次现场生成新密钥 → 指纹必然不同。
        let (_, _, fp2) = generate_ephemeral_cert().unwrap();
        assert_ne!(fp, fp2);
    }

    #[test]
    fn ipv4_candidates_nonempty() {
        assert!(!local_ipv4_candidates().is_empty());
    }

    /// 端到端：真实 TLS 监听 + 客户端 SPKI 指纹锁定 + token 握手 + 统计流。
    /// 直接驱动 run_accept_loop（不经 tauri runtime，tokio 测试线程内完成）。
    #[tokio::test]
    async fn pairing_session_end_to_end() {
        let state = Arc::new(PairingState::default());
        let (cert_der, key_der, fingerprint) = generate_ephemeral_cert().unwrap();
        let token = generate_pairing_token();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();

        let loop_state = state.clone();
        let loop_token = token.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_accept_loop(listener, cert_der, key_der, loop_token, loop_state, rx).await;
        });

        // 客户端：SPKI 指纹锁定（与伴侣 App 相同的出带校验逻辑）。
        let verifier = Arc::new(FpVerifier {
            expected_fp: fingerprint.clone(),
        });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

        let tcp = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        let tls = connector
            .connect(server_name(), tcp)
            .await
            .expect("TLS 握手（指纹校验通过）");

        let (read_half, mut write_half) = tokio::io::split(tls);
        write_half
            .write_all(format!("{PROTOCOL_TAG} {token}\n").as_bytes())
            .await
            .unwrap();
        let mut lines = BufReader::new(read_half).lines();
        let welcome = lines.next_line().await.unwrap().unwrap();
        assert!(welcome.contains("\"welcome\""), "got {welcome}");

        write_half
            .write_all(b"{\"type\":\"device_hello\",\"model\":\"POC-Test\"}\n")
            .await
            .unwrap();
        let mut stats_line = br#"{"type":"capture_stats","frames":42,"audio_supported":true,"sample_jpeg_bytes":18000}"#.to_vec();
        stats_line.push(b'\n');
        write_half.write_all(&stats_line).await.unwrap();
        write_half.write_all(b"{\"type\":\"bye\"}\n").await.unwrap();
        // 留出服务端处理时间。
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        let status = state.status();
        let joined = status.events.join("\n");
        assert!(joined.contains("设备报到：POC-Test"), "{joined}");
        assert!(joined.contains("捕获统计：42 帧"), "{joined}");
        assert!(joined.contains("伴侣端正常结束会话"), "{joined}");
    }

    /// 错误 token 必须被拒绝。
    #[tokio::test]
    async fn pairing_rejects_wrong_token() {
        let state = Arc::new(PairingState::default());
        let (cert_der, key_der, fingerprint) = generate_ephemeral_cert().unwrap();
        let std_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = std_listener.local_addr().unwrap().port();
        let listener = set_nonblocking_and_convert(std_listener).unwrap();

        let loop_state = state.clone();
        let (_tx, rx) = oneshot::channel::<()>();
        tokio::spawn(async move {
            run_accept_loop(listener, cert_der, key_der, "CORRECTTOKEN1234".into(), loop_state, rx).await;
        });

        let verifier = Arc::new(FpVerifier { expected_fp: fingerprint });
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(config));

        let tcp = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}")).await.unwrap();
        let tls = connector.connect(server_name(), tcp).await.unwrap();
        let (mut read_half, mut write_half) = tokio::io::split(tls);
        write_half.write_all(b"MDP1 WRONGTOKEN00000\n").await.unwrap();
        let mut buf = vec![0u8; 128];
        let n = tokio::time::timeout(std::time::Duration::from_secs(5), read_half.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).contains("rejected"));
        assert!(state.status().events.iter().any(|e| e.contains("配对码校验失败")));
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
