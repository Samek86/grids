//! Picture bytes are decoded in a child process.
//!
//! A property handler or codec that faults while the hosted Details view reads a JPEG
//! takes down this process; that path is Windows. Our own preview must not do the same.
//! The child is killed if it runs too long. Its death does not kill Maguro.

use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::jpeg::{self, JpegFault};
use super::peek::Pix;
use super::preview::{self, ext_of};
use crate::i18n::tr;
use crate::winpath::win_long_path;

const FILE_CAP: u64 = 80 * 1024 * 1024;
const CHILD_MS: u64 = 8_000;

static SEQ: AtomicU64 = AtomicU64::new(1);

pub(crate) fn image_main(args: &[String]) -> i32 {
    let Some(w) = args.get(1).and_then(|s| s.parse::<i32>().ok()) else { return 1 };
    let Some(h) = args.get(2).and_then(|s| s.parse::<i32>().ok()) else { return 1 };
    let Some(flags) = args.get(3).and_then(|s| s.parse::<u32>().ok()) else { return 1 };
    let Some(out) = args.get(4) else { return 1 };
    let Some(path) = args.get(5) else { return 1 };
    let w = w.clamp(1, 4096);
    let h = h.clamp(1, 4096);
    let path = win_long_path(path);
    let Ok(meta) = std::fs::metadata(&path) else { return 1 };
    if meta.len() == 0 || meta.len() > FILE_CAP {
        return 2;
    }
    let Ok(bytes) = std::fs::read(&path) else { return 1 };
    if preview_ext(&path) {
        match jpeg::inspect(&bytes) {
            Err(JpegFault::Huge) => return 3,
            Err(JpegFault::ZeroSize) => return 2,
            Err(JpegFault::Empty) => return 2,
            _ => {}
        }
    }
    unsafe {
        let com = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED).is_ok();
        let pix = preview::decode_pic(&bytes, w, h).map(|pix| apply_orient(&bytes, pix)).or_else(|| preview::shell_pic(&path, w, h, shell_flags(flags)));
        if com {
            windows::Win32::System::Com::CoUninitialize();
        }
        match pix {
            Some(pix) if write_pix(Path::new(out), &pix) => 0,
            Some(_) => 1,
            None => 4,
        }
    }
}

fn preview_ext(path: &str) -> bool {
    matches!(ext_of(path).as_str(), "jpg" | "jpeg" | "jpe" | "jfif")
}

fn apply_orient(bytes: &[u8], mut pix: Pix) -> Pix {
    let Ok(meta) = jpeg::inspect(bytes) else { return pix };
    if let Some((w, h, bgra)) = jpeg::orient_bgra(pix.w, pix.h, &pix.bgra, meta.orientation) {
        pix = Pix::new(w, h, bgra);
    }
    pix
}

fn shell_flags(bits: u32) -> windows::Win32::UI::Shell::SIIGBF {
    windows::Win32::UI::Shell::SIIGBF(bits as i32)
}

fn write_pix(path: &Path, pix: &Pix) -> bool {
    let Some(n) = pix_len(pix.w, pix.h) else { return false };
    if pix.bgra.len() != n {
        return false;
    }
    let Ok(mut f) = std::fs::File::create(path) else { return false };
    let ok = f.write_all(b"PGIM").is_ok()
        && f.write_all(&(pix.w as u32).to_le_bytes()).is_ok()
        && f.write_all(&(pix.h as u32).to_le_bytes()).is_ok()
        && f.write_all(&1u16.to_le_bytes()).is_ok()
        && f.write_all(&pix.bgra).is_ok();
    ok
}

fn pix_len(w: i32, h: i32) -> Option<usize> {
    if !(1..=jpeg::MAX_EDGE as i32).contains(&w) || !(1..=jpeg::MAX_EDGE as i32).contains(&h) {
        return None;
    }
    (w as usize).checked_mul(h as usize)?.checked_mul(4)
}

fn read_pix(path: &Path) -> Option<Pix> {
    let b = std::fs::read(path).ok()?;
    if b.len() < 14 || &b[..4] != b"PGIM" {
        return None;
    }
    let w = u32::from_le_bytes(b[4..8].try_into().ok()?) as i32;
    let h = u32::from_le_bytes(b[8..12].try_into().ok()?) as i32;
    let n = pix_len(w, h)?;
    let data = b.get(14..14 + n)?.to_vec();
    Some(Pix::new(w, h, data))
}

/// Decode `path` off this process. `Err` is an in-pane message, not a crash.
pub(crate) fn isolated_pic(path: &str, w: i32, h: i32, flags: u32) -> Result<Pix, String> {
    if let Err(msg) = reject_header(path) {
        return Err(msg);
    }
    let w = w.clamp(1, 4096);
    let h = h.clamp(1, 4096);
    let Ok(exe) = std::env::current_exe() else {
        return Err(msg_spawn());
    };
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let out = std::env::temp_dir().join(format!("maguro-img-{}-{id}.pgim", std::process::id()));
    let _tmp = Tmp(out.clone());
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--maguro-image").arg(w.to_string()).arg(h.to_string()).arg(flags.to_string()).arg(&out).arg(path);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|_| msg_spawn())?;
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) if start.elapsed() >= std::time::Duration::from_millis(CHILD_MS) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(msg_slow());
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(30)),
            Err(_) => return Err(msg_stop("?")),
        }
    };
    if !status.success() {
        return Err(msg_stop(&code_text(&status)));
    }
    read_pix(&out).ok_or_else(|| msg_stop(&code_text(&status)))
}

struct Tmp(std::path::PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn code_text(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(c) if c < 0 => format!("0x{:08X}", c as u32),
        Some(c) => c.to_string(),
        None => "?".into(),
    }
}

fn reject_header(path: &str) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| msg_open())?;
    if meta.len() == 0 {
        return Err(msg_empty());
    }
    if meta.len() > FILE_CAP {
        return Err(msg_big());
    }
    let ext = ext_of(path);
    let head = read_prefix(path, 1024 * 1024).unwrap_or_default();
    if matches!(ext.as_str(), "jpg" | "jpeg" | "jpe" | "jfif") {
        return match jpeg::inspect(&head) {
            Ok(_) => Ok(()),
            Err(JpegFault::Truncated) if (head.len() as u64) < meta.len() => Ok(()),
            Err(JpegFault::Huge) => Err(msg_huge()),
            Err(JpegFault::Empty) => Err(msg_empty()),
            Err(JpegFault::ZeroSize) | Err(JpegFault::NotJpeg) | Err(JpegFault::Truncated) => Err(msg_bad()),
        };
    }
    if let Some(Err(JpegFault::Huge)) = jpeg::probe_edges(&head) {
        return Err(msg_huge());
    }
    if let Some(Err(JpegFault::ZeroSize)) = jpeg::probe_edges(&head) {
        return Err(msg_bad());
    }
    Ok(())
}

fn read_prefix(path: &str, max: usize) -> Option<Vec<u8>> {
    use std::io::Read;
    let f = std::fs::File::open(path).ok()?;
    let mut b = Vec::new();
    f.take(max as u64).read_to_end(&mut b).ok()?;
    Some(b)
}

fn msg_empty() -> String {
    tr(crate::Key::FileIsEmpty).into()
}
fn msg_huge() -> String {
    tr(crate::Key::PictureIsTooLarge).into()
}
fn msg_big() -> String {
    tr(crate::Key::SkippedLargerThan80Mb).into()
}
fn msg_bad() -> String {
    tr(crate::Key::PictureCouldNotBeRead).into()
}
fn msg_slow() -> String {
    tr(crate::Key::PictureTookTooLong).into()
}
fn msg_open() -> String {
    tr(crate::Key::FileCouldNotBeOpened).into()
}
fn msg_spawn() -> String {
    tr(crate::Key::CouldNotStartPictureDecoder).into()
}
fn msg_stop(code: &str) -> String {
    format!("{} ({code})", tr(crate::Key::DecoderStopped))
}
