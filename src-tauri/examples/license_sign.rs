//! Pro 许可证签发工具（仅内部使用，不随应用分发）。
//!
//! 私钥通过环境变量 `MIRRORDOCK_LICENSE_SEED`（64 位十六进制）提供，
//! 种子保存在仓库外的内部文档目录，绝不入 Git、绝不入日志。
//!
//! 用法：
//! ```sh
//! export MIRRORDOCK_LICENSE_SEED=<64 hex chars>
//! cargo run --example license_sign -- --key-id 2026-001 --days 365
//! cargo run --example license_sign -- --key-id 2026-001 --edition pro --days 0   # 0 = 永久
//! ```

use mirrordock_lib::licensing::{encode_license, LicensePayload};

fn main() {
    let mut key_id = String::new();
    let mut edition = "pro".to_string();
    let mut days: u64 = 365;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--key-id" => {
                key_id = args.next().expect("--key-id 需要一个值");
            }
            "--edition" => {
                edition = args.next().expect("--edition 需要一个值");
            }
            "--days" => {
                days = args.next().expect("--days 需要一个值").parse().expect("--days 须为整数");
            }
            other => {
                eprintln!("未知参数: {other}");
                std::process::exit(64);
            }
        }
    }
    if key_id.is_empty() {
        eprintln!("用法: license_sign --key-id <id> [--edition pro] [--days N]  （0 = 永久）");
        std::process::exit(64);
    }

    let seed_hex = std::env::var("MIRRORDOCK_LICENSE_SEED").expect(
        "缺少 MIRRORDOCK_LICENSE_SEED 环境变量（签发私钥在内部文档目录的说明文件里）",
    );
    let mut seed = [0u8; 32];
    for (i, byte) in seed.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&seed_hex[i * 2..i * 2 + 2], 16).expect("种子须为 64 位十六进制");
    }
    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);

    let expires_at = if days == 0 {
        None
    } else {
        Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("系统时间早于 Unix 纪元")
                .as_secs()
                + days * 86_400,
        )
    };

    let payload = LicensePayload {
        product: "mirrordock".to_string(),
        key_id,
        edition,
        expires_at,
    };
    println!("{}", encode_license(&payload, &signing));
}
