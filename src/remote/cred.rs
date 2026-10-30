//! Generic credentials in Windows Credential Manager. The blob is UTF-16LE without a trailing null.

use windows::Win32::Security::Credentials::{CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC};

use crate::imports::*;

pub(crate) fn write_secret(target: &str, secret: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    let mut name = wide_z(target);
    let mut blob = utf16_bytes(secret);
    let cred = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(name.as_mut_ptr()),
        CredentialBlobSize: blob.len() as u32,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    let ok = unsafe { CredWriteW(&cred, 0).is_ok() };
    blob.fill(0);
    name.fill(0);
    ok
}

pub(crate) fn read_secret(target: &str) -> Option<String> {
    if target.is_empty() {
        return None;
    }
    let name = HSTRING::from(target);
    let mut cred = std::ptr::null_mut();
    let ok = unsafe { CredReadW(&name, CRED_TYPE_GENERIC, Some(0), &mut cred).is_ok() };
    if !ok || cred.is_null() {
        return None;
    }
    let blob = unsafe {
        let c = &*cred;
        let n = c.CredentialBlobSize as usize;
        if c.CredentialBlob.is_null() || n == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, n).to_vec()
        }
    };
    unsafe { CredFree(cred as *const _) };
    let text = utf16_text(&blob);
    let mut blob = blob;
    blob.fill(0);
    Some(text)
}

pub(crate) fn delete_secret(target: &str) -> bool {
    if target.is_empty() {
        return false;
    }
    unsafe { CredDeleteW(PCWSTR(wide_z(target).as_ptr()), CRED_TYPE_GENERIC, Some(0)).is_ok() }
}

fn wide_z(s: &str) -> Vec<u16> {
    let mut v: Vec<u16> = s.encode_utf16().collect();
    v.push(0);
    v
}

fn utf16_bytes(s: &str) -> Vec<u8> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut bytes = Vec::with_capacity(units.len() * 2);
    for u in units {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    bytes
}

fn utf16_text(bytes: &[u8]) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks(2) {
        if chunk.len() < 2 {
            break;
        }
        units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_round_trip_does_not_keep_the_secret() {
        let target = format!("Panegrid:test:{}:{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        let secret = "s3cret-pl4in-pgtest";
        assert!(write_secret(&target, secret), "CredWrite");
        assert_eq!(read_secret(&target).as_deref(), Some(secret));
        assert!(delete_secret(&target));
        assert!(read_secret(&target).is_none());
    }
}
