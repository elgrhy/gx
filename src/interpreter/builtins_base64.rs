//! Base64 encode/decode — pure Rust, no external dependency.

pub(super) fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = if chunk.len() > 1 {
            chunk[1] as usize
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            chunk[2] as usize
        } else {
            0
        };
        result.push(CHARS[b0 >> 2] as char);
        result.push(CHARS[((b0 & 3) << 4) | (b1 >> 4)] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[b2 & 0x3f] as char);
        } else {
            result.push('=');
        }
    }
    result
}

pub(super) fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    let s = s.trim_end_matches('=');
    let decode_char = |c: char| -> Result<u8, String> {
        match c {
            'A'..='Z' => Ok(c as u8 - b'A'),
            'a'..='z' => Ok(c as u8 - b'a' + 26),
            '0'..='9' => Ok(c as u8 - b'0' + 52),
            '+' => Ok(62),
            '/' => Ok(63),
            _ => Err(format!("Invalid base64 character: {}", c)),
        }
    };
    let chars: Vec<char> = s.chars().collect();
    let mut result = Vec::new();
    for chunk in chars.chunks(4) {
        let b0 = decode_char(chunk[0])?;
        let b1 = if chunk.len() > 1 {
            decode_char(chunk[1])?
        } else {
            0
        };
        let b2 = if chunk.len() > 2 {
            decode_char(chunk[2])?
        } else {
            0
        };
        let b3 = if chunk.len() > 3 {
            decode_char(chunk[3])?
        } else {
            0
        };
        result.push((b0 << 2) | (b1 >> 4));
        if chunk.len() > 2 {
            result.push(((b1 & 0xf) << 4) | (b2 >> 2));
        }
        if chunk.len() > 3 {
            result.push(((b2 & 3) << 6) | b3);
        }
    }
    Ok(result)
}
