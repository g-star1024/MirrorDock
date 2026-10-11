//! scrcpy 4.1 客户端协议（Rust 原生实现）。
//!
//! 逐字节对照 scrcpy v4.1 权威来源，不凭记忆猜：
//! - 协议总览：`doc/develop.md`（Connection / Video and audio / Controls 三节）
//! - 控制消息布局：`app/src/control_msg.c`（`sc_control_msg_serialize`）
//! - 设备→客户端消息：`app/src/device_msg.c`（`sc_device_msg_deserialize`）
//!
//! # 连接（tunnel_forward=true，本模块固定走 forward）
//! 1. `adb forward tcp:<local> localabstract:scrcpy_<scid>`（scid 为 31 位随机数）
//! 2. `adb shell CLASSPATH=... app_process / com.genymobile.scrcpy.Server 4.1 \
//!      scid=<scid> tunnel_forward=true audio=false control=true cleanup=false \
//!      max_size=<n> video_codec=h264`
//! 3. 按序连 socket：video（首），audio（关），control（末）。audio=false 时只 video+control。
//!
//! 每个 socket（forward 模式）：首 socket 先收 1 字节 dummy，再收 device meta（设备名）。
//! 视频 socket 线格式见 [`video`]；控制 socket 直接收发控制消息（见 [`control`]）。
//!
//! # 为什么不用 fork 的 scrcpy
//! 官方客户端不原生出 WebSocket/裸 H.264，ws-scrcpy 之流靠 fork scrcpy 注入 WS 服务。
//! 我们只在服务端实现客户端协议，复用 vanilla scrcpy-server，容器零新增依赖。

// 本模块为实时镜像（X10-115）的协议底座，在 WS 端点接入前部分 API 暂无调用方。
#![allow(dead_code)]

use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Command as ProcCommand;

// ===========================================================================
// 视频解复用
// ===========================================================================

/// 视频 socket 上解析出的事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoEvent {
    /// 旋转/会话重置：新分辨率。首帧前也必发一次。
    Session { width: u32, height: u32 },
    /// 一个媒体包（已含原始 H.264 NAL 字节，可直接喂浏览器解码器）。
    /// `config`=含 SPS/PPS；`keyframe`=关键帧（IDR）。
    Media {
        config: bool,
        keyframe: bool,
        pts: u64,
        data: Vec<u8>,
    },
}

/// 视频 codec id（u32 大端，即 4 字节 ASCII）。
pub mod codec {
    pub const H264: u32 = 0x6832_3634; // "h264"
    pub const H265: u32 = 0x6832_3635; // "h265"
    pub const AV1: u32 = 0x0061_7631; // "av1"
}

async fn read_dyn_str<R: AsyncRead + Unpin>(r: &mut R) -> io::Result<String> {
    let mut len = [0u8; 4];
    r.read_exact(&mut len).await?;
    let n = u32::from_be_bytes(len) as usize;
    let mut buf = vec![0u8; n];
    r.read_exact(&mut buf).await?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// 视频流解复用器：从视频 socket 逐事件读取。
///
/// 构造后必须先调 [`VideoDemux::handshake`] 吃掉 dummy byte + device meta + codec id，
/// 之后循环 [`next_event`]。
pub struct VideoDemux<R> {
    reader: R,
}

impl<R: AsyncRead + Unpin> VideoDemux<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    /// 吃掉 forward 模式的 dummy byte（1B）+ device meta（设备名）+ codec id（u32），
    /// 返回 (解复用器, 设备名, codec id)。
    pub async fn handshake(mut self) -> io::Result<(Self, String, u32)> {
        let mut dummy = [0u8; 1];
        self.reader.read_exact(&mut dummy).await?; // forward 模式设备先发 1 字节 dummy
        let name = read_dyn_str(&mut self.reader).await?; // device meta（设备名）
        let mut cid = [0u8; 4];
        self.reader.read_exact(&mut cid).await?; // codec id
        Ok((self, name, u32::from_be_bytes(cid)))
    }

    /// 读下一个视频事件。连接关闭时返回 `UnexpectedEof`。
    pub async fn next_event(&mut self) -> io::Result<VideoEvent> {
        let mut h = [0u8; 12];
        self.reader.read_exact(&mut h).await?;

        // 首 bit（MSB）= 1 → session packet（旋转/分辨率变化）
        if h[0] & 0x80 != 0 {
            let width = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
            let height = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
            return Ok(VideoEvent::Session { width, height });
        }

        // 否则 media packet：byte0 的 bit7=media flag(0)，bit6=config，bit5=keyframe；
        // 低 61 位 = PTS；byte8..12 = payload size。
        let raw = u64::from_be_bytes(h[0..8].try_into().unwrap());
        let config = raw & 0x4000_0000_0000_0000 != 0;
        let keyframe = raw & 0x2000_0000_0000_0000 != 0;
        let pts = raw & 0x1FFF_FFFF_FFFF_FFFF;
        let size = u32::from_be_bytes([h[8], h[9], h[10], h[11]]) as usize;

        let mut data = vec![0u8; size];
        self.reader.read_exact(&mut data).await?;
        Ok(VideoEvent::Media {
            config,
            keyframe,
            pts,
            data,
        })
    }
}

// ===========================================================================
// 控制消息（客户端 → 设备）
// ===========================================================================

pub mod control {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum KeyAction {
        Down = 0,
        Up = 1,
        Multi = 2,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum TouchAction {
        Down = 0,
        Up = 1,
        Move = 2,
        Cancel = 3,
        Outside = 4,
        PointerDown = 5,
        PointerUp = 6,
        HoverMove = 7,
        Scroll = 8,
        HoverEnter = 9,
        HoverExit = 10,
        BtnPress = 11,
        BtnRelease = 12,
    }

    /// 设备屏幕尺寸参考（用于把指针坐标归一化到 0..screen 比例，scrcpy 要求）。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ScreenSize {
        pub width: u16,
        pub height: u16,
    }

    #[derive(Debug, Clone)]
    pub enum ControlMessage {
        /// 注入按键（action/keycode/repeat/metastate）。keycode 用 Android KEYCODE_*。
        InjectKeycode {
            action: KeyAction,
            keycode: u32,
            repeat: u32,
            metastate: u32,
        },
        /// 注入文本（特殊字符走这个，不走 keycode）。
        InjectText(String),
        /// 注入触控事件。pointer_id=`u64::MAX` 表示鼠标，`u64::MAX-1` 表示通用手指。
        InjectTouch {
            action: TouchAction,
            pointer_id: u64,
            x: u32,
            y: u32,
            screen: ScreenSize,
            pressure: f32, // 0..1
            action_button: u32,
            buttons: u32,
        },
        /// 注入滚轮。hscroll/vscroll 范围 [-16,16]，归一化后转 i16 定点。
        InjectScroll {
            x: u32,
            y: u32,
            screen: ScreenSize,
            hscroll: f32,
            vscroll: f32,
            buttons: u32,
        },
        /// BACK 或点亮屏幕（action 仅用 Down）。
        BackOrScreenOn(KeyAction),
        /// 开关屏幕。
        SetDisplayPower(bool),
        /// 旋转设备。
        RotateDevice,
    }

    /// f32(0..1) → u16 定点（scrcpy 的 sc_float_to_u16fp）。
    fn float_to_u16fp(v: f32) -> u16 {
        let v = v.clamp(0.0, 1.0) * 65535.0;
        v as u16
    }

    /// i16 定点：值范围 [-16,16] 归一化到 [-1,1] 再转 i16（scrcpy sc_float_to_i16fp）。
    fn scroll_to_i16fp(v: f32) -> i16 {
        let n = (v / 16.0).clamp(-1.0, 1.0) * 32767.0;
        n as i16
    }

    /// 把控制消息序列化进 `out`（大端，逐字节对齐 control_msg.c）。
    pub fn serialize(msg: &ControlMessage, out: &mut Vec<u8>) {
        let start = out.len();
        match msg {
            ControlMessage::InjectKeycode {
                action,
                keycode,
                repeat,
                metastate,
            } => {
                out.resize(start + 14, 0);
                out[start] = 0; // SC_CONTROL_MSG_TYPE_INJECT_KEYCODE
                out[start + 1] = *action as u8;
                out[start + 2..start + 6].copy_from_slice(&keycode.to_be_bytes());
                out[start + 6..start + 10].copy_from_slice(&repeat.to_be_bytes());
                out[start + 10..start + 14].copy_from_slice(&metastate.to_be_bytes());
            }
            ControlMessage::InjectText(text) => {
                let bytes = text.as_bytes();
                // type(1) + len(4) + payload
                out.resize(start + 5 + bytes.len(), 0);
                out[start] = 1; // INJECT_TEXT
                out[start + 1..start + 5].copy_from_slice(&(bytes.len() as u32).to_be_bytes());
                out[start + 5..].copy_from_slice(bytes);
            }
            ControlMessage::InjectTouch {
                action,
                pointer_id,
                x,
                y,
                screen,
                pressure,
                action_button,
                buttons,
            } => {
                out.resize(start + 32, 0);
                out[start] = 2; // INJECT_TOUCH_EVENT
                out[start + 1] = *action as u8;
                out[start + 2..start + 10].copy_from_slice(&pointer_id.to_be_bytes());
                out[start + 10..start + 14].copy_from_slice(&x.to_be_bytes());
                out[start + 14..start + 18].copy_from_slice(&y.to_be_bytes());
                out[start + 18..start + 20].copy_from_slice(&screen.width.to_be_bytes());
                out[start + 20..start + 22].copy_from_slice(&screen.height.to_be_bytes());
                out[start + 22..start + 24]
                    .copy_from_slice(&float_to_u16fp(*pressure).to_be_bytes());
                out[start + 24..start + 28].copy_from_slice(&action_button.to_be_bytes());
                out[start + 28..start + 32].copy_from_slice(&buttons.to_be_bytes());
            }
            ControlMessage::InjectScroll {
                x,
                y,
                screen,
                hscroll,
                vscroll,
                buttons,
            } => {
                out.resize(start + 21, 0);
                out[start] = 3; // INJECT_SCROLL_EVENT
                out[start + 1..start + 5].copy_from_slice(&x.to_be_bytes());
                out[start + 5..start + 9].copy_from_slice(&y.to_be_bytes());
                out[start + 9..start + 11].copy_from_slice(&screen.width.to_be_bytes());
                out[start + 11..start + 13].copy_from_slice(&screen.height.to_be_bytes());
                out[start + 13..start + 15]
                    .copy_from_slice(&scroll_to_i16fp(*hscroll).to_be_bytes());
                out[start + 15..start + 17]
                    .copy_from_slice(&scroll_to_i16fp(*vscroll).to_be_bytes());
                out[start + 17..start + 21].copy_from_slice(&buttons.to_be_bytes());
            }
            ControlMessage::BackOrScreenOn(action) => {
                out.resize(start + 2, 0);
                out[start] = 4; // BACK_OR_SCREEN_ON
                out[start + 1] = *action as u8;
            }
            ControlMessage::SetDisplayPower(on) => {
                out.resize(start + 2, 0);
                out[start] = 9; // SET_DISPLAY_POWER
                out[start + 1] = *on as u8;
            }
            ControlMessage::RotateDevice => {
                out.resize(start + 1, 0);
                out[start] = 11; // ROTATE_DEVICE
            }
        }
    }
}

// ===========================================================================
// 会话管理：启动 scrcpy-server + adb forward + 连接 video/control socket
// ===========================================================================

/// 一个镜像会话：持有两个 socket 与 adb shell 子进程（stop 时杀掉即结束）。
pub struct MirrorSession {
    pub video: TcpStream,
    pub control: TcpStream,
    pub scid: u32,
    pub local_port: u16,
    server_child: tokio::process::Child,
}

impl MirrorSession {
    /// 拆出部件，供 WS 层把子进程 move 进保活任务。
    pub fn into_parts(self) -> (TcpStream, TcpStream, u32, u16, tokio::process::Child) {
        (
            self.video,
            self.control,
            self.scid,
            self.local_port,
            self.server_child,
        )
    }
}

fn scid() -> u32 {
    // 31 位随机数（scrcpy 约定），用时间纳秒低位即可（非密码学用途）。
    (SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("系统时钟早于 epoch")
        .as_nanos() as u32)
        & 0x7FFF_FFFF
}

fn server_path() -> String {
    std::env::var("SCRCPY_SERVER_PATH").unwrap_or_else(|_| "/opt/scrcpy/scrcpy-server".to_owned())
}

async fn run_adb(args: &[&str]) -> io::Result<()> {
    let status = ProcCommand::new(crate::adb::adb_binary())
        .args(args)
        .status()
        .await?;
    if !status.success() {
        return Err(io::Error::other(format!("adb {:?} 失败：{status}", args)));
    }
    Ok(())
}

/// 启动一次镜像会话（USB 直连场景：serial 指定设备）。
///
/// `local_port` 为宿主机转发端口（NAS 单设备可用固定值，如 27183）。
/// `max_size` 限制视频宽高最大值（0 = 不限制）。
pub async fn start_mirror(
    serial: &str,
    local_port: u16,
    max_size: u16,
) -> io::Result<MirrorSession> {
    let scid = scid();
    let srv = server_path();

    // 1. 推送 server（幂等；每次重推确保与客户端版本一致）
    run_adb(&[
        "-s",
        serial,
        "push",
        &srv,
        "/data/local/tmp/scrcpy-server.jar",
    ])
    .await?;

    // 2. adb forward
    run_adb(&[
        "-s",
        serial,
        "forward",
        &format!("tcp:{local_port}"),
        &format!("localabstract:scrcpy_{scid}"),
    ])
    .await?;

    // 3. 启动设备端 server（前台子进程：杀掉即结束会话；cleanup=false 不自动清理）
    let max_size_arg = if max_size == 0 {
        String::new()
    } else {
        format!(" max_size={max_size}")
    };
    let server_child = ProcCommand::new(crate::adb::adb_binary())
        .args([
            "-s",
            serial,
            "shell",
            "CLASSPATH=/data/local/tmp/scrcpy-server.jar",
            "app_process",
            "/",
            "com.genymobile.scrcpy.Server",
            "4.1",
            &format!("scid={scid}"),
            "tunnel_forward=true",
            "audio=false",
            "control=true",
            "cleanup=false",
            "video_codec=h264",
        ])
        .args(max_size_arg.split_whitespace().filter(|s| !s.is_empty()))
        .kill_on_drop(true)
        .spawn()?;

    // 4. 连 video（首 socket，含 dummy+meta+codec）与 control（次 socket）
    //    设备端 server 启动有延迟，连不上就重试。
    let video = connect_retry(local_port).await?;
    let control = connect_retry(local_port).await?;

    Ok(MirrorSession {
        video,
        control,
        scid,
        local_port,
        server_child,
    })
}

async fn connect_retry(port: u16) -> io::Result<TcpStream> {
    let mut last = None;
    for _ in 0..50 {
        match TcpStream::connect(("127.0.0.1", port)).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                last = Some(e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "连接 scrcpy-server 超时")))
}

impl MirrorSession {
    /// 发送一条控制消息到设备。
    pub async fn send_control(&mut self, msg: &control::ControlMessage) -> io::Result<()> {
        let mut buf = Vec::new();
        control::serialize(msg, &mut buf);
        self.control.write_all(&buf).await?;
        self.control.flush().await?;
        Ok(())
    }

    /// 结束会话：杀掉设备端 server 子进程并移除 adb forward。
    pub async fn stop(mut self) {
        let _ = self.server_child.start_kill();
        let _ = run_adb(&["forward", "--remove", &format!("tcp:{}", self.local_port)]).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[test]
    fn control_keycode_layout() {
        // INJECT_KEYCODE: type(0) action(1) keycode(u32) repeat(u32) metastate(u32)
        let msg = control::ControlMessage::InjectKeycode {
            action: control::KeyAction::Down,
            keycode: 0x1234_5678,
            repeat: 0,
            metastate: 0x00AB_CDEF,
        };
        let mut buf = Vec::new();
        control::serialize(&msg, &mut buf);
        assert_eq!(buf.len(), 14);
        assert_eq!(buf[0], 0);
        assert_eq!(buf[1], 0);
        assert_eq!(&buf[2..6], &0x1234_5678u32.to_be_bytes());
        assert_eq!(&buf[6..10], &0u32.to_be_bytes());
        assert_eq!(&buf[10..14], &0x00AB_CDEFu32.to_be_bytes());
    }

    #[test]
    fn control_touch_layout() {
        // INJECT_TOUCH_EVENT: 32 bytes
        let msg = control::ControlMessage::InjectTouch {
            action: control::TouchAction::Down,
            pointer_id: u64::MAX, // 鼠标
            x: 100,
            y: 200,
            screen: control::ScreenSize {
                width: 1080,
                height: 2340,
            },
            pressure: 1.0,
            action_button: 0,
            buttons: 0,
        };
        let mut buf = Vec::new();
        control::serialize(&msg, &mut buf);
        assert_eq!(buf.len(), 32);
        assert_eq!(buf[0], 2);
        assert_eq!(buf[1], 0);
        assert_eq!(&buf[2..10], &u64::MAX.to_be_bytes());
        assert_eq!(&buf[10..14], &100u32.to_be_bytes());
        assert_eq!(&buf[14..18], &200u32.to_be_bytes());
        assert_eq!(&buf[18..20], &1080u16.to_be_bytes());
        assert_eq!(&buf[20..22], &2340u16.to_be_bytes());
        assert_eq!(&buf[22..24], &0xFFFFu16.to_be_bytes()); // pressure 1.0 → 65535
    }

    #[test]
    fn control_scroll_layout() {
        let msg = control::ControlMessage::InjectScroll {
            x: 10,
            y: 20,
            screen: control::ScreenSize {
                width: 100,
                height: 200,
            },
            hscroll: 0.0,
            vscroll: 16.0, // 满格 → +32767
            buttons: 0,
        };
        let mut buf = Vec::new();
        control::serialize(&msg, &mut buf);
        assert_eq!(buf.len(), 21);
        assert_eq!(buf[0], 3);
        assert_eq!(&buf[1..5], &10u32.to_be_bytes());
        assert_eq!(&buf[5..9], &20u32.to_be_bytes());
        assert_eq!(&buf[13..15], &0i16.to_be_bytes());
        assert_eq!(&buf[15..17], &32767i16.to_be_bytes());
    }

    #[test]
    fn parse_session_packet() {
        // MSB=1 → session；width/height 在 byte4..12
        let mut h = [0u8; 12];
        h[0] = 0x80;
        h[4..8].copy_from_slice(&1920u32.to_be_bytes());
        h[8..12].copy_from_slice(&1080u32.to_be_bytes());
        let raw = u64::from_be_bytes(h[0..8].try_into().unwrap());
        assert!(raw & 0x8000_0000_0000_0000 != 0);
        let width = u32::from_be_bytes([h[4], h[5], h[6], h[7]]);
        let height = u32::from_be_bytes([h[8], h[9], h[10], h[11]]);
        assert_eq!((width, height), (1920, 1080));
    }

    #[tokio::test]
    async fn demux_over_fake_scrcpy() {
        // 起一个 fake scrcpy-server：发 dummy + device meta + codec(h264)
        // + 一个 session + 一个 config/keyframe 媒体包 + 一个普通媒体包。
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        let srv_task = tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            // dummy byte
            s.write_all(&[0x00]).await.unwrap();
            // device meta: len(4)=5 + "Pixel"
            s.write_all(&5u32.to_be_bytes()).await.unwrap();
            s.write_all(b"Pixel").await.unwrap();
            // codec id = h264
            s.write_all(&codec::H264.to_be_bytes()).await.unwrap();
            // session packet: MSB=1, width=1080, height=2340
            let mut sh = [0u8; 12];
            sh[0] = 0x80;
            sh[4..8].copy_from_slice(&1080u32.to_be_bytes());
            sh[8..12].copy_from_slice(&2340u32.to_be_bytes());
            s.write_all(&sh).await.unwrap();
            // media packet 1: config + keyframe, pts=0, payload=IDRish bytes
            let mut mh = [0u8; 12];
            let raw = 0x6000_0000_0000_0000u64; // bit6 config + bit5 keyframe, PTS=0
            mh[0..8].copy_from_slice(&raw.to_be_bytes());
            mh[8..12].copy_from_slice(&4u32.to_be_bytes());
            s.write_all(&mh).await.unwrap();
            s.write_all(&[0xAA, 0xBB, 0xCC, 0xDD]).await.unwrap();
            // media packet 2: 普通帧, pts=1000, payload
            let mut mh2 = [0u8; 12];
            let raw2 = 1000u64; // 无 config/keyframe
            mh2[0..8].copy_from_slice(&raw2.to_be_bytes());
            mh2[8..12].copy_from_slice(&2u32.to_be_bytes());
            s.write_all(&mh2).await.unwrap();
            s.write_all(&[0x01, 0x02]).await.unwrap();
            s.flush().await.unwrap();
            // 保持连接让客户端读完
            tokio::time::sleep(Duration::from_millis(200)).await;
        });

        let client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let (mut demux, name, cid) = VideoDemux::new(client).handshake().await.unwrap();
        assert_eq!(name, "Pixel");
        assert_eq!(cid, codec::H264);

        let e1 = demux.next_event().await.unwrap();
        match e1 {
            VideoEvent::Session { width, height } => assert_eq!((width, height), (1080, 2340)),
            other => panic!("期望 session，得到 {other:?}"),
        }
        let e2 = demux.next_event().await.unwrap();
        match e2 {
            VideoEvent::Media {
                config,
                keyframe,
                pts,
                data,
            } => {
                assert!(config && keyframe);
                assert_eq!(pts, 0);
                assert_eq!(data, vec![0xAA, 0xBB, 0xCC, 0xDD]);
            }
            other => panic!("期望 media，得到 {other:?}"),
        }
        let e3 = demux.next_event().await.unwrap();
        match e3 {
            VideoEvent::Media {
                config,
                keyframe,
                pts,
                data,
            } => {
                assert!(!config && !keyframe);
                assert_eq!(pts, 1000);
                assert_eq!(data, vec![0x01, 0x02]);
            }
            other => panic!("期望 media，得到 {other:?}"),
        }
        srv_task.abort();
    }
}
