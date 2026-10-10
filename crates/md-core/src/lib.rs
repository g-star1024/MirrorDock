//! MirrorDock 共享核心库（X10-113）。
//!
//! 桌面端（Tauri）与 NAS 端（Docker/HTTP）共用这里的实现，让两处跑同一份判据。
//! 抽取原则：**只放与界面、与平台都无关的纯逻辑**。凡是依赖窗口、托盘、菜单栏、
//! macOS 输入法（如 `input_source`）的模块一律留在桌面端，不为了“抽得更多”而
//! 把平台相关的东西塞进跨平台 crate。

pub mod base32;
pub mod devices;
pub mod error;

// 方便消费方一次性引入核心类型。
pub use base32::{base32_decode, base32_encode};
pub use error::{adb_command_error, adb_missing_error, AppError};
