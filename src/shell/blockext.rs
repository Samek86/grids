//! Stop a named shell extension from loading inside Maguro.
//!
//! COM looks up a registered class object before it loads `InprocServer32`. A stub
//! factory whose `CreateInstance` fails makes that CLSID unavailable in this process
//! only. Nothing is written to the registry, and Explorer is unchanged. DLLs under
//! System32, SysWOW64, or WinSxS, and the shell DLLs themselves, are never stubbed.
//! A DLL the shell loads by path, with no CLSID, cannot be blocked this way.

use std::collections::HashSet;

use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
    RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ,
};

use crate::crash::{self, file_name};
use crate::imports::*;

const CAP: usize = 2_000;
const THUMB: &str = "{e357fccd-a995-4576-b01f-234630154e96}";
const PREVIEW: &str = "{8895b1c6-b41f-4c1c-a562-0d564250836f}";
const CONTEXT: &str = "{000214E4-0000-0000-C000-000000000046}";
const EXTS: &[&str] = &["jpg", "jpeg", "jpe", "jfif", "png", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "psd"];

const DENY: &[&str] = &[
    "shell32.dll",
    "explorerframe.dll",
    "windows.storage.dll",
    "windowscodecs.dll",
    "propsys.dll",
    "ntshrui.dll",
    "twinui.dll",
    "twinui.pcshell.dll",
    "comctl32.dll",
    "ole32.dll",
    "oleaut32.dll",
    "kernel32.dll",
    "kernelbase.dll",
    "ntdll.dll",
    "user32.dll",
];

#[implement(IClassFactory)]
struct BlockedFactory;

impl IClassFactory_Impl for BlockedFactory_Impl {
    fn CreateInstance(&self, punkouter: Ref<IUnknown>, _riid: *const GUID, ppvobject: *mut *mut core::ffi::c_void) -> Result<()> {
        if !ppvobject.is_null() {
            unsafe { *ppvobject = std::ptr::null_mut() };
        }
        if !punkouter.is_null() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        Err(CLASS_E_CLASSNOTAVAILABLE.into())
    }

    fn LockServer(&self, _flock: BOOL) -> Result<()> {
        Ok(())
    }
}

/// File names of in-process shell servers, lowercased. Read-only. Empty on any failure.
pub(crate) fn registered_names() -> Vec<String> {
    let mut names = Vec::new();
    unsafe { collect(|_clsid, path| push_name(&mut names, path)) };
    names.sort();
    names.dedup();
    names
}

/// Register stubs for CLSIDs whose server file name is in `want`. No-op when `want` is empty.
pub(crate) fn apply(want: &[String]) {
    if want.is_empty() {
        return;
    }
    let mut seen = HashSet::new();
    unsafe {
        collect(|clsid, path| {
            if !names_match(want, path) || !blockable(path) {
                return;
            }
            let Some(guid) = parse_guid(clsid) else { return };
            if !seen.insert(clsid.to_ascii_lowercase()) {
                return;
            }
            stub(guid);
        });
    }
}

fn push_name(names: &mut Vec<String>, path: &str) {
    let file = file_name(path).to_ascii_lowercase();
    if file.is_empty() || names.iter().any(|e| e == &file) {
        return;
    }
    names.push(file);
}

pub(crate) fn names_match(want: &[String], path: &str) -> bool {
    let file = file_name(path).to_ascii_lowercase();
    if file.is_empty() {
        return false;
    }
    let stem = file.strip_suffix(".dll").unwrap_or(&file);
    want.iter().any(|w| {
        let w = w.trim();
        let w = w.rsplit(['\\', '/']).next().unwrap_or(w).to_ascii_lowercase();
        !w.is_empty() && (w == file || w == stem)
    })
}

pub(crate) fn blockable(path: &str) -> bool {
    let lower = path.replace('/', "\\").to_ascii_lowercase();
    if crash::is_system_module(&lower) {
        return false;
    }
    let file = file_name(&lower);
    !DENY.iter().any(|d| file == *d)
}

fn parse_guid(s: &str) -> Option<GUID> {
    let t = s.trim().trim_matches(|c| c == '{' || c == '}' || c == ' ');
    GUID::try_from(t).ok()
}

unsafe fn stub(clsid: GUID) {
    let factory: IClassFactory = BlockedFactory.into();
    if CoRegisterClassObject(&clsid, &factory, CLSCTX_INPROC_SERVER, REGCLS_MULTIPLEUSE).is_ok() {
        std::mem::forget(factory);
    }
}

unsafe fn collect(mut each: impl FnMut(&str, &str)) {
    let mut n = 0usize;
    let mut seen = HashSet::new();
    let mut one = |clsid: &str| {
        if n >= CAP {
            return;
        }
        let key = clsid.trim().trim_matches(|c| c == '{' || c == '}').to_ascii_lowercase();
        if key.is_empty() || !seen.insert(key.clone()) {
            return;
        }
        let path = server_of(&key);
        if path.is_empty() {
            return;
        }
        n += 1;
        each(&key, &path);
    };
    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        enum_value_names(root, r"Software\Microsoft\Windows\CurrentVersion\Shell Extensions\Approved", &mut one);
        enum_sub_defaults(root, r"Software\Microsoft\Windows\CurrentVersion\Explorer\ShellIconOverlayIdentifiers", &mut one);
    }
    enum_sub_defaults(
        HKEY_LOCAL_MACHINE,
        r"Software\Microsoft\Windows\CurrentVersion\PropertySystem\PropertyHandlers",
        &mut one,
    );
    for ext in EXTS {
        for handler in [THUMB, PREVIEW, CONTEXT] {
            let rel = format!(r".{ext}\ShellEx\{handler}");
            let v = key_default(HKEY_CLASSES_ROOT, &rel);
            if !v.is_empty() {
                one(&v);
            }
        }
    }
    for handler in [THUMB, PREVIEW] {
        let rel = format!(r"SystemFileAssociations\image\ShellEx\{handler}");
        let v = key_default(HKEY_CLASSES_ROOT, &rel);
        if !v.is_empty() {
            one(&v);
        }
    }
}

unsafe fn enum_value_names(root: HKEY, path: &str, one: &mut impl FnMut(&str)) {
    let Some(k) = open_at(root, path) else { return };
    for i in 0..CAP as u32 {
        let mut name = [0u16; 128];
        let mut n = name.len() as u32;
        let err = RegEnumValueW(k, i, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None);
        if err != ERROR_SUCCESS {
            break;
        }
        if n == 0 {
            continue;
        }
        let s = String::from_utf16_lossy(&name[..n as usize]);
        one(&s);
    }
    let _ = RegCloseKey(k);
}

unsafe fn enum_sub_defaults(root: HKEY, path: &str, one: &mut impl FnMut(&str)) {
    let Some(k) = open_at(root, path) else { return };
    for i in 0..CAP as u32 {
        let mut name = [0u16; 256];
        let mut n = name.len() as u32;
        let err = RegEnumKeyExW(k, i, Some(PWSTR(name.as_mut_ptr())), &mut n, None, None, None, None);
        if err != ERROR_SUCCESS {
            break;
        }
        if n == 0 {
            continue;
        }
        let sub = String::from_utf16_lossy(&name[..n as usize]);
        let child = if path.is_empty() { sub } else { format!(r"{path}\{sub}") };
        let v = key_default(root, &child);
        if !v.is_empty() {
            one(&v);
        }
    }
    let _ = RegCloseKey(k);
}

unsafe fn server_of(clsid: &str) -> String {
    let rels = [
        format!(r"CLSID\{{{clsid}}}\InprocServer32"),
        format!(r"Software\Classes\CLSID\{{{clsid}}}\InprocServer32"),
    ];
    for root in [HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        for rel in &rels {
            let s = key_default(root, rel);
            if !s.is_empty() {
                return s;
            }
        }
    }
    String::new()
}

unsafe fn key_default(root: HKEY, path: &str) -> String {
    let Some(k) = open_at(root, path) else { return String::new() };
    let s = key_sz(k);
    let _ = RegCloseKey(k);
    s
}

unsafe fn open_at(root: HKEY, path: &str) -> Option<HKEY> {
    if path.is_empty() {
        return None;
    }
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let mut key = HKEY::default();
    let err = RegOpenKeyExW(root, PCWSTR(wide.as_ptr()), None, KEY_READ, &mut key);
    (err == ERROR_SUCCESS && !key.is_invalid()).then_some(key)
}

unsafe fn key_sz(key: HKEY) -> String {
    let mut buf = [0u16; 1024];
    let mut n = (buf.len() * 2) as u32;
    let err = RegGetValueW(key, PCWSTR::null(), PCWSTR::null(), RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ, None, Some(buf.as_mut_ptr() as _), Some(&mut n));
    if err != ERROR_SUCCESS || n < 2 {
        return String::new();
    }
    let chars = (n as usize / 2).saturating_sub(1).min(buf.len());
    String::from_utf16_lossy(&buf[..chars]).trim_end_matches('\0').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_the_file_not_a_substring() {
        assert!(names_match(&["codec.dll".into()], r"C:\Program Files\Vendor\Codec.dll"));
        assert!(names_match(&["codec".into()], r"C:\Program Files\Vendor\Codec.dll"));
        assert!(!names_match(&["jpeg".into()], r"C:\a\jpegxr.dll"));
        assert!(names_match(&["jpeg".into()], r"C:\a\jpeg.dll"));
        assert!(!names_match(&["jpeg".into()], r"C:\a\notjpeg.dll"));
        assert!(!names_match(&["".into()], r"C:\a\jpeg.dll"));
    }

    #[test]
    fn system_dlls_stay_loaded() {
        assert!(!blockable(r"C:\Windows\System32\shell32.dll"));
        assert!(!blockable(r"C:\Windows\SysWOW64\explorerframe.dll"));
        assert!(!blockable(r"C:\Windows\System32\vendor.dll"));
        assert!(!blockable(r"D:\tools\propsys.dll"));
        assert!(blockable(r"C:\Program Files\Codec\codec.dll"));
    }
}
