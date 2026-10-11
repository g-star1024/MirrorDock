//! 实时镜像 WebSocket 层（X10-115）。
//!
//! 把 scrcpy 4.1 的视频流经 WebSocket 推给浏览器，并把浏览器回传的触控/键鼠
//! 注入回设备。协议底座在 [`crate::scrcpy_proto`]（Rust 原生实现，无 fork scrcpy）。
//!
//! ## 会话模型
//! - 一台设备同一时刻**只允许一条镜像会话**（`MirrorManager.active`）。
//!   与录制互斥是桌面端就定的规矩：同时编码两路 H.264 会把设备编码器打满掉帧。
//! - 多个浏览器标签页连同一台设备时，视频经 `broadcast` 扇出（共同看一路画面），
//!   控制消息由各连接各自回传（共用一个 control socket，消息串行写入）。
//!
//! ## WS 报文（文本帧，JSON）
//! 服务器 → 浏览器：
//! - `{"t":"meta","device":..,"codec":"h264","width":w,"height":h}` 会话就绪
//! - `{"t":"session","width":w,"height":h}` 分辨率变化/旋转
//! - `{"t":"video","config":b,"keyframe":b,"pts":n,"data":"<base64>"}` 一个媒体包
//! - `{"t":"state","running":b}` 会话启停
//! - `{"t":"error","message":..}` 异常（设备掉线等）
//!
//! 浏览器 → 服务器（控制，全部可选字段带默认）：
//! - `{"t":"touch","action":"down|up|move","x":n,"y":n,"screenW":w,"screenH":h,"pressure":f}`
//! - `{"t":"key","action":"down|up","keycode":n,"metastate":n}`
//! - `{"t":"text","text":"..."}`
//! - `{"t":"scroll","x":n,"y":n,"screenW":w,"screenH":h,"hscroll":f,"vscroll":f}`
//! - `{"t":"back"}` / `{"t":"power","on":b}` / `{"t":"rotate"}`

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State as AxumState, WebSocketUpgrade};
use axum::response::IntoResponse;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::{broadcast, Mutex};

use crate::scrcpy_proto::{self, control, VideoEvent};
use crate::State;

/// 每个镜像会话的扇出通道容量。视频帧约 30fps、单帧 KB 级，64 约等于 2 秒缓冲；
/// 更慢的消费者会收到 Lagged 并跳帧，而不是把快消费者一起拖慢。
const VIDEO_FANOUT: usize = 64;

/// 浏览器看的一路视频事件（已转成 WS 可发的 JSON 形态）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum ServerMsg {
    Meta {
        device: String,
        codec: String,
        width: u32,
        height: u32,
    },
    Session {
        width: u32,
        height: u32,
    },
    Video {
        config: bool,
        keyframe: bool,
        pts: u64,
        data: String,
    },
    State {
        running: bool,
    },
    Error {
        message: String,
    },
}

/// 一路镜像会话：向广播通道发视频事件，并持有到设备 control socket 的写句柄。
struct ActiveMirror {
    /// 视频/会话事件的扇出。subscribe() 即得一个浏览器侧的接收端。
    tx: broadcast::Sender<ServerMsg>,
    /// 回传控制消息。串行写入 control socket（scrcpy 控制通道是单条字节流）。
    control: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    /// 设备当前分辨率（控制消息需要按此归一化坐标）。
    screen: Arc<Mutex<control::ScreenSize>>,
}

/// 会话表：serial → 活跃镜像。
#[derive(Default)]
pub struct MirrorManager {
    active: Mutex<HashMap<String, Arc<ActiveMirror>>>,
}

impl MirrorManager {
    /// 取某设备当前会话（若已开启）。
    async fn get(&self, serial: &str) -> Option<Arc<ActiveMirror>> {
        self.active.lock().await.get(serial).cloned()
    }

    /// 开启某设备的镜像会话：push server、adb forward、连 video/control、
    /// 起解复用泵把视频事件灌进广播通道。已存在则直接复用。
    ///
    /// 返回该会话句柄；浏览器随后 `subscribe()` 收视频。
    async fn ensure(
        self: &Arc<Self>,
        serial: &str,
        max_size: u16,
    ) -> io::Result<Arc<ActiveMirror>> {
        // 快路径：已有会话直接复用（多个标签页看同一台）。
        if let Some(m) = self.get(serial).await {
            return Ok(m);
        }
        let mut guard = self.active.lock().await;
        // 双检：等锁期间可能已被别的连接建立。
        if let Some(m) = guard.get(serial) {
            return Ok(m.clone());
        }

        let session =
            scrcpy_proto::start_mirror(serial, mirror_local_port(serial), max_size).await?;
        let (video, control_sock, _scid, _port, server_child) = session.into_parts();
        // server_child 必须活到会话结束：把它连同解复用泵一起 move 进后台任务，
        // 任务结束（设备断开/广播无人收）时 drop 即 kill_on_drop 杀掉设备端 server。

        let (tx, _rx) = broadcast::channel(VIDEO_FANOUT);
        let (control_read, control_write) = control_sock.into_split();
        let active = Arc::new(ActiveMirror {
            tx: tx.clone(),
            control: Arc::new(Mutex::new(control_write)),
            screen: Arc::new(Mutex::new(control::ScreenSize {
                width: 1080,
                height: 2340,
            })),
        });

        // 后台解复用泵：video socket → 广播；同时保活 server_child。
        let screen_ref = active.screen.clone();
        let serial_owned = serial.to_owned();
        let tx_pump = tx.clone();
        tokio::spawn(async move {
            let _server_child = server_child; // 保活：任务存活期间设备端 server 不被杀
            let _control_read = control_read; // 目前不收设备→客户端的控制回包
            pump_video(video, tx_pump, screen_ref, serial_owned).await;
        });

        guard.insert(serial.to_owned(), active.clone());
        Ok(active)
    }

    /// 结束某设备的镜像会话（关掉广播，泵任务随 socket 断开自然退出）。
    pub async fn stop(&self, serial: &str) -> bool {
        self.active.lock().await.remove(serial).is_some()
    }
}

/// 解复用泵：把 video socket 的事件转成 ServerMsg 广播出去。
async fn pump_video(
    video: tokio::net::TcpStream,
    tx: broadcast::Sender<ServerMsg>,
    screen: Arc<Mutex<control::ScreenSize>>,
    serial: String,
) {
    let demux = match scrcpy_proto::VideoDemux::new(video).handshake().await {
        Ok((d, name, cid)) => {
            let codec = match cid {
                c if c == scrcpy_proto::codec::H264 => "h264",
                c if c == scrcpy_proto::codec::H265 => "h265",
                c if c == scrcpy_proto::codec::AV1 => "av1",
                _ => "unknown",
            };
            let _ = tx.send(ServerMsg::Meta {
                device: name,
                codec: codec.into(),
                width: 0,
                height: 0,
            });
            d
        }
        Err(e) => {
            let _ = tx.send(ServerMsg::Error {
                message: format!("镜像握手失败：{e}"),
            });
            return;
        }
    };
    let mut demux = demux;
    loop {
        match demux.next_event().await {
            Ok(VideoEvent::Session { width, height }) => {
                *screen.lock().await = control::ScreenSize {
                    width: width.min(u16::MAX as u32) as u16,
                    height: height.min(u16::MAX as u32) as u16,
                };
                let _ = tx.send(ServerMsg::Session { width, height });
            }
            Ok(VideoEvent::Media {
                config,
                keyframe,
                pts,
                data,
            }) => {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
                // 没有订阅者时 send 返回 Err，但镜像仍在跑——不算故障，继续。
                let _ = tx.send(ServerMsg::Video {
                    config,
                    keyframe,
                    pts,
                    data: b64,
                });
            }
            Err(e) => {
                let _ = tx.send(ServerMsg::Error {
                    message: format!("设备 {serial} 视频流中断：{e}"),
                });
                let _ = tx.send(ServerMsg::State { running: false });
                return;
            }
        }
    }
}

/// 单设备固定本地转发端口（NAS 单设备场景够用；多设备时按 serial 哈希错开）。
fn mirror_local_port(_serial: &str) -> u16 {
    27183
}

// ===========================================================================
// 控制消息（浏览器 → 服务器，JSON）
// ===========================================================================

#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
enum ClientMsg {
    Touch {
        action: String,
        x: u32,
        y: u32,
        #[serde(rename = "screenW")]
        screen_w: u32,
        #[serde(rename = "screenH")]
        screen_h: u32,
        #[serde(default = "default_pressure")]
        pressure: f32,
    },
    Key {
        action: String,
        keycode: u32,
        #[serde(default)]
        metastate: u32,
    },
    Text {
        text: String,
    },
    Scroll {
        x: u32,
        y: u32,
        #[serde(rename = "screenW")]
        screen_w: u32,
        #[serde(rename = "screenH")]
        screen_h: u32,
        hscroll: f32,
        vscroll: f32,
    },
    Back,
    Power {
        on: bool,
    },
    Rotate,
}

fn default_pressure() -> f32 {
    1.0
}

impl ClientMsg {
    /// 转成 scrcpy 控制消息。触控/滚轮用浏览器上报的分辨率（与画面一致）。
    fn to_control(&self) -> Option<control::ControlMessage> {
        use control::{ControlMessage as C, KeyAction, ScreenSize, TouchAction};
        let screen = |w: u32, h: u32| ScreenSize {
            width: w.min(u16::MAX as u32) as u16,
            height: h.min(u16::MAX as u32) as u16,
        };
        match self {
            ClientMsg::Touch {
                action,
                x,
                y,
                screen_w,
                screen_h,
                pressure,
            } => {
                let act = match action.as_str() {
                    "down" => TouchAction::Down,
                    "up" => TouchAction::Up,
                    "move" => TouchAction::Move,
                    _ => return None,
                };
                Some(C::InjectTouch {
                    action: act,
                    pointer_id: u64::MAX, // 鼠标指针
                    x: *x,
                    y: *y,
                    screen: screen(*screen_w, *screen_h),
                    pressure: *pressure,
                    action_button: 0,
                    buttons: 0,
                })
            }
            ClientMsg::Key {
                action,
                keycode,
                metastate,
            } => {
                let act = match action.as_str() {
                    "down" => KeyAction::Down,
                    "up" => KeyAction::Up,
                    _ => return None,
                };
                Some(C::InjectKeycode {
                    action: act,
                    keycode: *keycode,
                    repeat: 0,
                    metastate: *metastate,
                })
            }
            ClientMsg::Text { text } => Some(C::InjectText(text.clone())),
            ClientMsg::Scroll {
                x,
                y,
                screen_w,
                screen_h,
                hscroll,
                vscroll,
            } => Some(C::InjectScroll {
                x: *x,
                y: *y,
                screen: screen(*screen_w, *screen_h),
                hscroll: *hscroll,
                vscroll: *vscroll,
                buttons: 0,
            }),
            ClientMsg::Back => Some(C::BackOrScreenOn(KeyAction::Down)),
            ClientMsg::Power { on } => Some(C::SetDisplayPower(*on)),
            ClientMsg::Rotate => Some(C::RotateDevice),
        }
    }
}

// ===========================================================================
// WebSocket 端点
// ===========================================================================

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    pub serial: String,
    #[serde(default)]
    pub max_size: Option<u16>,
}

/// `GET /api/stream/ws?serial=...` —— 升级为 WebSocket。
pub async fn stream_ws(
    ws: WebSocketUpgrade,
    Query(q): Query<StreamQuery>,
    AxumState(state): AxumState<Arc<State>>,
) -> impl IntoResponse {
    let serial = q.serial.clone();
    // 未显式指定分辨率时跟随 Web 设置的画质预设（而非硬编码 1920）。
    let max_size = match q.max_size {
        Some(v) => v,
        None => state.settings.quality().await.max_size() as u16,
    };
    ws.on_upgrade(move |sock| handle_ws(sock, state, serial, max_size))
}

async fn handle_ws(sock: WebSocket, state: Arc<State>, serial: String, max_size: u16) {
    // 确保会话已建立（多标签页复用同一路）。
    let mirror = match state.mirrors.ensure(&serial, max_size).await {
        Ok(m) => m,
        Err(e) => {
            // 会话起不来（设备不在线/未授权/server 缺失）：发一条错误就关。
            let text = serde_json::to_string(&ServerMsg::Error {
                message: format!("无法开启镜像：{e}"),
            })
            .unwrap_or_default();
            let (mut tx, _rx) = sock.split();
            let _ = tx.send(Message::Text(text.into())).await;
            return;
        }
    };
    let _ = mirror.tx.send(ServerMsg::State { running: true });
    let mut rx = mirror.tx.subscribe();
    let (mut ws_tx, mut ws_rx) = sock.split();
    let control = mirror.control.clone();

    // 出站：广播通道 → 本连接的 WS sink。
    let mut out_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    let text = serde_json::to_string(&msg).unwrap_or_default();
                    if ws_tx.send(Message::Text(text.into())).await.is_err() {
                        return; // 连接已关
                    }
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    // 慢消费者跳帧：告知但不致命。
                    let note = serde_json::to_string(&ServerMsg::Error {
                        message: format!("已跳过 {n} 帧（网络或解码偏慢）"),
                    })
                    .unwrap_or_default();
                    if ws_tx.send(Message::Text(note.into())).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    });

    // 入站：本连接 WS → 设备 control socket。
    let mut in_task = tokio::spawn(async move {
        while let Some(Ok(frame)) = ws_rx.next().await {
            let Message::Text(text) = frame else { continue };
            let Ok(client) = serde_json::from_str::<ClientMsg>(&text) else {
                continue;
            };
            let Some(ctrl) = client.to_control() else {
                continue;
            };
            let mut buf = Vec::new();
            control::serialize(&ctrl, &mut buf);
            let mut guard = control.lock().await;
            use tokio::io::AsyncWriteExt;
            if guard.write_all(&buf).await.is_err() {
                return; // control socket 断了
            }
            let _ = guard.flush().await;
        }
    });

    // 任一侧结束（浏览器断开 / 设备断开）就收尾本连接；会话本体留给其他订阅者。
    tokio::select! {
        _ = &mut out_task => in_task.abort(),
        _ = &mut in_task => out_task.abort(),
    }
}

// ===========================================================================
// 测试
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[test]
    fn client_touch_maps_to_control() {
        let msg: ClientMsg = serde_json::from_str(
            r#"{"t":"touch","action":"down","x":100,"y":200,"screenW":1080,"screenH":2340}"#,
        )
        .unwrap();
        match msg.to_control() {
            Some(control::ControlMessage::InjectTouch { x, y, .. }) => {
                assert_eq!((x, y), (100, 200));
            }
            other => panic!("期望 InjectTouch，得到 {other:?}"),
        }
    }

    #[test]
    fn client_key_maps_to_control() {
        let msg: ClientMsg =
            serde_json::from_str(r#"{"t":"key","action":"down","keycode":4}"#).unwrap();
        match msg.to_control() {
            Some(control::ControlMessage::InjectKeycode { keycode, .. }) => assert_eq!(keycode, 4),
            other => panic!("期望 InjectKeycode，得到 {other:?}"),
        }
    }

    #[test]
    fn client_back_and_rotate() {
        let back: ClientMsg = serde_json::from_str(r#"{"t":"back"}"#).unwrap();
        assert!(matches!(
            back.to_control(),
            Some(control::ControlMessage::BackOrScreenOn(_))
        ));
        let rot: ClientMsg = serde_json::from_str(r#"{"t":"rotate"}"#).unwrap();
        assert!(matches!(
            rot.to_control(),
            Some(control::ControlMessage::RotateDevice)
        ));
    }

    #[test]
    fn unknown_action_dropped() {
        let msg: ClientMsg = serde_json::from_str(
            r#"{"t":"touch","action":"pinch","x":0,"y":0,"screenW":1,"screenH":1}"#,
        )
        .unwrap();
        assert!(msg.to_control().is_none());
    }

    /// 合成流集成测试：fake scrcpy video socket → 泵 → 广播，浏览器侧收到
    /// Meta/Session/Video 序列。验证「视频事件能扇出给订阅者」这条主链路。
    #[tokio::test]
    async fn pump_fans_out_video_events() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            s.write_all(&[0x00]).await.unwrap(); // dummy
            s.write_all(&3u32.to_be_bytes()).await.unwrap();
            s.write_all(b"Dev").await.unwrap(); // device meta
            s.write_all(&scrcpy_proto::codec::H264.to_be_bytes())
                .await
                .unwrap();
            let mut sh = [0u8; 12];
            sh[0] = 0x80;
            sh[4..8].copy_from_slice(&1080u32.to_be_bytes());
            sh[8..12].copy_from_slice(&2340u32.to_be_bytes());
            s.write_all(&sh).await.unwrap(); // session
            let mut mh = [0u8; 12];
            let raw = 0x6000_0000_0000_0000u64; // config+keyframe, pts=0
            mh[0..8].copy_from_slice(&raw.to_be_bytes());
            mh[8..12].copy_from_slice(&4u32.to_be_bytes());
            s.write_all(&mh).await.unwrap();
            s.write_all(&[0xDE, 0xAD, 0xBE, 0xEF]).await.unwrap();
            s.flush().await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        });

        let video = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let (tx, _rx) = broadcast::channel(VIDEO_FANOUT);
        let screen = Arc::new(Mutex::new(control::ScreenSize {
            width: 0,
            height: 0,
        }));
        let mut rx = tx.subscribe();
        let pump = tokio::spawn(pump_video(video, tx, screen.clone(), "test".to_owned()));

        // 期望序列：Meta → Session → Video
        let m1 = rx.recv().await.unwrap();
        assert!(matches!(m1, ServerMsg::Meta { .. }));
        let m2 = rx.recv().await.unwrap();
        match m2 {
            ServerMsg::Session { width, height } => assert_eq!((width, height), (1080, 2340)),
            other => panic!("期望 Session，得到 {other:?}"),
        }
        let m3 = rx.recv().await.unwrap();
        match m3 {
            ServerMsg::Video {
                config, keyframe, ..
            } => assert!(config && keyframe),
            other => panic!("期望 Video，得到 {other:?}"),
        }
        // screen 已随 Session 更新
        let s = screen.lock().await;
        assert_eq!((s.width, s.height), (1080, 2340));
        drop(s);
        pump.abort();
    }
}
