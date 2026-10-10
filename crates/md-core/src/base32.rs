//! Base32（RFC 4648，无填充）——自桌面端 licensing 模块抽离，X10-113。
//!
//! 先前深埋在 mod licensing 内，NAS 端要用就得整体依赖授权模块，
//! 故抽为独立模块供两端共用。

    // ---- Base32（RFC 4648，无填充）。自实现以避免引入额外依赖。 ----

    const B32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

    pub fn base32_encode(data: &[u8]) -> String {
        let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
        for chunk in data.chunks(5) {
            let mut buf = [0u8; 5];
            buf[..chunk.len()].copy_from_slice(chunk);
            let bits = u64::from_be_bytes([
                0, buf[0], buf[1], buf[2], buf[3], buf[4], 0, 0,
            ]) >> 16;
            let chars = match chunk.len() {
                5 => 8,
                4 => 7,
                3 => 5,
                2 => 4,
                _ => 2,
            };
            for i in 0..chars {
                let idx = ((bits >> (35 - i * 5)) & 0x1f) as usize;
                out.push(B32_ALPHABET[idx] as char);
            }
        }
        out
    }

    pub fn base32_decode(text: &str) -> Option<Vec<u8>> {
        let mut bytes = Vec::new();
        let mut bits: u32 = 0;
        let mut acc: u32 = 0;
        for ch in text.chars() {
            let v = B32_ALPHABET
                .iter()
                .position(|&c| c as char == ch.to_ascii_uppercase())? as u32;
            acc = (acc << 5) | v;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                bytes.push(((acc >> bits) & 0xff) as u8);
            }
        }
        Some(bytes)
    }
