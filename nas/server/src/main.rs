//! MirrorDock NAS 版服务端入口。
//!
//! 立项场景：NAS 常开机 + USB 直连 + 24h 录制归档 + 远程 Web 访问。
//!
//! 刻意保持「薄」：这里是 HTTP 编排层，判据逻辑全部在 md-core，与桌面端同源。
//! 不做指标导出、高可用、集群这类 NAS 世界里常见的横向扩张——先把「手机长期
//! 挂着、能看能用能录」这一件事做扎实。

mod adb;
mod api;
mod record;

use axum::routing::{delete, get, post};
use axum::Router;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

pub struct State {
    data_dir: PathBuf,
    /// 同一时刻只允许一条录制。NAS 常开机的价值在于长时间稳定录像，
    /// 并发多录制会互相抢编码资源导致掉帧。
    recording: Mutex<Option<record::Recording>>,
}

fn data_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("MIRRORDOCK_DATA").unwrap_or_else(|_| "/var/lib/mirrordock".to_owned()),
    )
}

#[tokio::main]
async fn main() {
    let data = data_dir();
    // 录像/截图目录要在首次请求前就备好，否则第一次录制会因竞态失败。
    let _ = tokio::fs::create_dir_all(data.join("recordings")).await;
    let _ = tokio::fs::create_dir_all(data.join("screenshots")).await;

    let state = Arc::new(State {
        data_dir: data.clone(),
        recording: Mutex::new(None),
    });

    // 开机自检：scrcpy 缺 server 时，设备识别与截图（走 adb 通道）全都正常，
    // 只有录制会秒退——NAS 常开机的场景下人不一定在场，必须在启动日志里先喊出来。
    let server = record::server_path();
    if server.is_file() {
        println!("scrcpy-server 就绪：{}", server.display());
    } else {
        eprintln!(
            "[警告] 找不到 scrcpy-server（{}）：录制将不可用，设备识别与截图不受影响。 \
             请检查镜像是否完整（是否覆盖了 SCRCPY_SERVER_PATH）。",
            server.display()
        );
    }

    // 静态站：MIRRORDOCK_WEB 可指向自定义前端目录，缺省随包 ./web。
    let web = std::env::var("MIRRORDOCK_WEB").unwrap_or_else(|_| "/opt/mirrordock/web".to_owned());

    let api = Router::new()
        .route("/api/status", get(api::status))
        .route("/api/devices", get(api::devices))
        .route("/api/screenshot", get(api::screenshot))
        .route("/api/record/start", post(api::start_record))
        .route("/api/record/stop", post(api::stop_record))
        .route("/api/recordings", get(api::recordings))
        .route("/api/recordings/{name}", delete(api::delete_recording))
        .route("/api/files", get(api::list_files));

    // 录像/截图下载。用现成的静态服务而不是自写下载接口：它天然支持 Range 请求，
    // 浏览器能直接拖动 MP4 进度条播放，这是「24h 录制归档」场景里用户真正要的
    // 用法——录完不用下载几十 GB 文件就能回看。
    let files = ServeDir::new(&data);

    let app = Router::new()
        .merge(api)
        .nest_service("/files", files)
        // 兜底静态服务必须放在 merge 之后：顺序颠倒会吞掉 /api 前缀，表现为
        // 接口返回 index.html——在浏览器里很难看出这是路由顺序问题。
        .fallback_service(ServeDir::new(&web))
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_owned());
    let addr = format!("0.0.0.0:{port}");
    println!("MirrorDock NAS 端启动：http://{addr}");
    println!("  数据目录：{}", data.display());
    println!("  静态站目录：{web}");
    println!("  adb：{}", adb::adb_binary());
    println!("  scrcpy：{}", adb::scrcpy_binary());

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("端口绑定失败");
    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}
