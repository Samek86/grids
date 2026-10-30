//! rclone's `obscure` / `reveal` (AES-256-CTR, public key, raw URL-safe base64).
//! The IV rides in the token, so a password never has to be passed to `rclone obscure`.

use aes::Aes256;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use ctr::cipher::{KeyIvInit, StreamCipher};
use ctr::Ctr128BE;

const CRYPT_KEY: [u8; 32] = [
    0x9c, 0x93, 0x5b, 0x48, 0x73, 0x0a, 0x55, 0x4d, 0x6b, 0xfd, 0x7c, 0x63, 0xc8, 0x86, 0xa9, 0x2b, 0xd3, 0x90, 0x19, 0x8e, 0xb8, 0x12, 0x8a, 0xfb, 0xf4, 0xde, 0x16, 0x2b, 0x8b, 0x95, 0xf6, 0x38,
];

type Aes256Ctr = Ctr128BE<Aes256>;

fn xor(data: &mut [u8], iv: &[u8; 16]) {
    let mut stream = Aes256Ctr::new(&CRYPT_KEY.into(), iv.into());
    stream.apply_keystream(data);
}

/// Obscure `plain` with a caller-supplied IV. Tests pin the IV; callers use [`obscure`].
pub(crate) fn obscure_iv(plain: &str, iv: &[u8; 16]) -> String {
    let mut body = plain.as_bytes().to_vec();
    xor(&mut body, iv);
    let mut raw = Vec::with_capacity(16 + body.len());
    raw.extend_from_slice(iv);
    raw.extend_from_slice(&body);
    URL_SAFE_NO_PAD.encode(raw)
}

pub(crate) fn obscure(plain: &str) -> String {
    obscure_iv(plain, &fresh_iv())
}

pub(crate) fn reveal(token: &str) -> Option<String> {
    let raw = URL_SAFE_NO_PAD.decode(token.trim()).ok()?;
    if raw.len() < 16 {
        return None;
    }
    let mut iv = [0u8; 16];
    iv.copy_from_slice(&raw[..16]);
    let mut body = raw[16..].to_vec();
    xor(&mut body, &iv);
    String::from_utf8(body).ok()
}

fn fresh_iv() -> [u8; 16] {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let mut iv = [0u8; 16];
    iv[..8].copy_from_slice(&t.to_le_bytes());
    iv[8..].copy_from_slice(&n.to_le_bytes());
    iv
}

/// Replace any copy of a secret (plain or obscured) so a mount error can be shown.
pub(crate) fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= 3 {
            out = out.replace(secret, "••••");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn obscure_round_trips_and_hides_the_password() {
        let iv = [7u8; 16];
        let token = obscure_iv("s3cret-pl4in", &iv);
        assert!(!token.contains("s3cret-pl4in"));
        assert_eq!(reveal(&token).as_deref(), Some("s3cret-pl4in"));
        assert_eq!(reveal(&obscure("다른암호")).as_deref(), Some("다른암호"));
        assert!(reveal("@@@").is_none());
    }

    #[test]
    fn matches_rclone_reveal_when_installed() {
        let Some(exe) = super::super::tools::find_rclone() else { return };
        let token = obscure_iv("pgtest-secret", &[3u8; 16]);
        let out = std::process::Command::new(exe).args(["reveal", &token]).output().expect("rclone reveal");
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "pgtest-secret");
    }

    #[test]
    fn redact_strips_plain_and_obscured() {
        let token = obscure_iv("s3cret-pl4in", &[9u8; 16]);
        let line = format!("pass={token} pw=s3cret-pl4in");
        let clean = redact(&line, &["s3cret-pl4in".into(), token.clone()]);
        assert!(!clean.contains("s3cret-pl4in"));
        assert!(!clean.contains(&token));
    }
}
