//! 一次性许可证密钥对生成工具（仅内部使用，不随应用分发）。
//!
//! 从 /dev/urandom 读取 32 字节种子：私钥直接写入指定文件（绝不打印、
//! 绝不入 Git），公钥打印到标准输出供编译进 `LICENSE_VERIFYING_KEY`。
//!
//! 用法：`cargo run --example license_keygen -- <私钥输出文件路径>`

use std::io::Write;

use ed25519_dalek::Signer;

fn main() {
    let out_path = std::env::args().nth(1).expect("用法: license_keygen <私钥输出文件>");
    let mut seed = [0u8; 32];
    let mut urandom = std::fs::File::open("/dev/urandom").expect("无法打开 /dev/urandom");
    std::io::Read::read_exact(&mut urandom, &mut seed).expect("读取随机数失败");

    let signing = ed25519_dalek::SigningKey::from_bytes(&seed);
    let pubkey_hex: String = signing
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    // 用签名证明密钥对自洽：任意消息的签名必须能被同源公钥验证。
    let message = b"mirrordock license keygen self-check";
    use ed25519_dalek::Verifier;
    signing
        .verifying_key()
        .verify(message, &signing.sign(message))
        .expect("生成的密钥对自检失败");

    let mut out = std::fs::File::create(&out_path).expect("无法创建私钥输出文件");
    writeln!(
        out,
        "# MirrorDock 许可证签发私钥（绝不入 Git 仓库、绝不在对话/日志中回显）\n\
         # 用途：cargo run --example license_sign 读取 MIRRORDOCK_LICENSE_SEED 签发 Pro 许可证\n\
         # 对应公钥已编译进 src-tauri/src/lib.rs 的 LICENSE_VERIFYING_KEY\n\
         # 生成时间：{}（license_keygen，/dev/urandom）\n\
         MIRRORDOCK_LICENSE_SEED={}",
        chrono_like_now(),
        seed.iter().map(|b| format!("{b:02x}")).collect::<String>()
    )
    .expect("写入私钥失败");

    println!("PUBKEY_HEX={pubkey_hex}");
    println!("seed written (not echoed) to: {out_path}");
}

fn chrono_like_now() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map(|s| format!("unix:{} (UTC)", s))
        .unwrap_or_else(|_| "unknown".to_owned())
}
