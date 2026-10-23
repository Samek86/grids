use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, IWICBitmapEncoder, IWICBitmapFrameEncode, IWICImagingFactory, WICBitmapEncoderNoCache,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{STREAM_SEEK_END, STREAM_SEEK_SET};
use windows::Win32::System::DataExchange::IsClipboardFormatAvailable;
use windows::Win32::System::Memory::GlobalSize;
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::imports::*;
use crate::app::*;
use crate::ui::*;
use crate::pane::*;
use crate::session::*;
use crate::i18n::*;
use crate::{settings, store};

// ---------- background paste ----------

// Copies running on worker threads; the status line says so and skips its disk calls meanwhile.
pub(crate) static COPYING: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct Busy;

impl Busy {
    pub(crate) fn new() -> Busy {
        COPYING.fetch_add(1, Ordering::Relaxed);
        Busy
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        COPYING.fetch_sub(1, Ordering::Relaxed);
    }
}

thread_local! {
    // joined after the message loop so closing Grids doesn't cut a copy short
    pub(crate) static JOBS: RefCell<Vec<std::thread::JoinHandle<()>>> = const { RefCell::new(Vec::new()) };
}

// Ctrl+V in a file list: a held key's repeats are swallowed; the first press starts a copy, or goes
// to the shell when `start` can't take it. True = handled, don't pass it on.
pub(crate) fn paste_key(repeat: bool, start: impl FnOnce() -> bool) -> bool {
    repeat || start()
}

// Some(true): every source sits in `dst` (copy with "- Copy" names), Some(false): none does,
// None: empty, not a file system path or mixed (left to the shell).
pub(crate) fn paste_mode(srcs: &[String], dst: &str) -> Option<bool> {
    let dst = dst.trim_end_matches('\\').to_lowercase();
    let parent = |p: &String| Some(p.trim_end_matches('\\').rsplit_once('\\')?.0.trim_end_matches('\\').to_lowercase());
    let same: Vec<bool> = srcs.iter().map(|p| if is_fs(p) { parent(p).map(|d| d == dst) } else { None }).collect::<Option<_>>()?;
    match (same.iter().all(|&s| s), same.iter().any(|&s| s)) {
        _ if same.is_empty() => None,
        (true, _) => Some(true),
        (false, false) => Some(false),
        _ => None,
    }
}

// The ⇄ menu's copy / move: like a paste, but a move into the folder the files already sit in is refused.
pub(crate) fn send_mode(srcs: &[String], dst: &str, mv: bool) -> Option<bool> {
    paste_mode(srcs, dst).filter(|&same| !(mv && same))
}

// The ⇄ menu sends a whole selection or nothing: never a partial list from a capped or unreadable read.
pub(crate) const SEND_MAX: u32 = 10_000;

#[derive(Debug, PartialEq)]
pub(crate) enum Pick {
    Empty,
    TooMany,
    Unreadable,
}

pub(crate) fn collect_all(n: u32, mut get: impl FnMut(u32) -> Option<String>) -> std::result::Result<Vec<String>, Pick> {
    match n {
        0 => Err(Pick::Empty),
        n if n > SEND_MAX => Err(Pick::TooMany),
        n => (0..n).map(|k| get(k).ok_or(Pick::Unreadable)).collect(),
    }
}

// Ctrl+Shift+C in a file list: the selected items' paths (or the folder's), quoted, one per line.
// Virtual items without a file path are skipped. The clipboard is only touched once the text is ready.
pub(crate) unsafe fn copy_paths(a: usize) {
    let Some(fv) = view_of::<IFolderView2>(a) else { return };
    let path = |it: IShellItem| it.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p));
    let mut paths: Vec<String> = Vec::new();
    if let Ok(arr) = fv.GetSelection(false) {
        paths = (0..arr.GetCount().unwrap_or(0)).filter_map(|k| path(arr.GetItemAt(k).ok()?)).collect();
    }
    if fv.ItemCount(SVGIO_SELECTION).unwrap_or(0) == 0 {
        paths = fv.GetFolder::<IShellItem>().ok().and_then(path).into_iter().collect();
    }
    if paths.is_empty() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let text: Vec<u16> = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\r\n").encode_utf16().chain([0]).collect();
    let main = app(|x| x.main);
    let mut ok = false;
    if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2) {
        let p = GlobalLock(h) as *mut u16;
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
            _ = GlobalUnlock(h);
            if OpenClipboard(Some(main)).is_ok() {
                ok = EmptyClipboard().is_ok() && SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0))).is_ok();
                _ = CloseClipboard();
            }
        }
        if !ok {
            _ = GlobalFree(Some(h)); // after a successful SetClipboardData the system owns it
        }
    }
    if !ok {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let n = paths.len();
    let note = match (ui(), n) {
        (1, 1) => "경로 복사함".into(),
        (1, n) => format!("경로 {n}개 복사함"),
        (2, 1) => "パスをコピーしました".into(),
        (2, n) => format!("パスを {n} 個コピーしました"),
        (_, 1) => "Copied path".into(),
        (_, n) => format!("Copied {n} paths"),
    };
    app(|x| x.note = Some((note, Instant::now() + std::time::Duration::from_secs(3))));
    update_status();
}

// Plain file copies (Ctrl+C, then Ctrl+V into a folder) run on their own thread, so Grids stays usable.
// Cut, links, virtual items and anything else stay with the shell.
pub(crate) unsafe fn try_paste(a: usize) -> bool {
    let (dst, main) = app(|x| (x.s.paths[a].clone(), x.main));
    if !is_fs(&dst) || OpenClipboard(Some(main)).is_err() {
        return false;
    }
    let mut srcs = Vec::new();
    if let Ok(h) = GetClipboardData(CF_HDROP.0 as u32) {
        let drop = HDROP(h.0);
        for k in 0..DragQueryFileW(drop, u32::MAX, None) {
            let mut buf = vec![0u16; DragQueryFileW(drop, k, None) as usize + 1];
            let n = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
            srcs.push(String::from_utf16_lossy(&buf[..n]));
        }
    }
    let mut effect = None;
    if let Ok(h) = GetClipboardData(RegisterClipboardFormatW(w!("Preferred DropEffect"))) {
        let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
        if !p.is_null() {
            effect = Some(*p);
            _ = GlobalUnlock(HGLOBAL(h.0));
        }
    }
    let has = |f: PCWSTR| IsClipboardFormatAvailable(RegisterClipboardFormatW(f)).is_ok();
    let pic = picture_only(
        !srcs.is_empty(),
        has(w!("Shell IDList Array")),
        has(w!("FileGroupDescriptorW")) || has(w!("FileGroupDescriptor")),
        IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok(),
    )
    .then(|| clip_picture())
    .flatten();
    _ = CloseClipboard();
    if let Some(pic) = pic {
        return start_picture(pic, dst);
    }
    let copy = effect.is_none_or(|e| e & DROPEFFECT_MOVE.0 == 0 && e & DROPEFFECT_COPY.0 != 0);
    let Some(rename) = paste_mode(&srcs, &dst).filter(|_| copy) else { return false };
    start_copy(srcs, dst, rename, false)
}

/// Only a bare picture. Files, virtual files and text stay with the shell paste.
pub(crate) fn picture_only(has_hdrop_items: bool, has_idlist: bool, has_fgd: bool, has_text: bool) -> bool {
    !(has_hdrop_items || has_idlist || has_fgd || has_text)
}

pub(crate) enum Pic {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

/// The clipboard's PNG bytes, else CF_DIB. The clipboard is already open.
unsafe fn clip_picture() -> Option<Pic> {
    let bytes = |fmt: u32| {
        let h = HGLOBAL(GetClipboardData(fmt).ok()?.0);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(p, GlobalSize(h)).to_vec();
        _ = GlobalUnlock(h);
        Some(v)
    };
    if let Some(mut png) = bytes(RegisterClipboardFormatW(w!("PNG"))).filter(|b| b.starts_with(b"\x89PNG\r\n\x1a\n")) {
        if let Some(i) = png.windows(4).rposition(|w| w == b"IEND").filter(|i| i + 8 <= png.len()) {
            png.truncate(i + 8);
        }
        return Some(Pic::Png(png));
    }
    bytes(CF_DIB.0 as u32).map(Pic::Dib)
}

/// A CF_DIB as a .bmp file. None when the header or the palette is cut short.
pub(crate) fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |i: usize| Some(u32::from_le_bytes(dib.get(i..i + 4)?.try_into().ok()?));
    let size = u32_at(0)? as usize;
    let bits = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let (compression, used) = (u32_at(16)?, u32_at(32)? as usize);
    if size < 40 {
        return None;
    }
    let colors = if used > 0 { used } else if bits <= 8 { 1usize << bits } else { 0 };
    let masks = if size == 40 && compression == 3 { 12 } else { 0 };
    let off = colors.checked_mul(4)?.checked_add(size + masks)?;
    if off >= dib.len() {
        return None;
    }
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&u32::try_from(14 + dib.len()).ok()?.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&(14 + off as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// `Clipboard image 2026-10-21 153012.png`, with ` (k)` from the second try on.
pub(crate) fn image_name(lang: u8, t: &SYSTEMTIME, k: u32) -> String {
    let head = match lang {
        1 => "클립보드 이미지",
        2 => "クリップボード画像",
        _ => "Clipboard image",
    };
    let n = if k >= 2 { format!(" ({k})") } else { String::new() };
    format!("{head} {:04}-{:02}-{:02} {:02}{:02}{:02}{n}.png", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// Writes `bytes` under the first free name. Never replaces a file. Returns the name used.
pub(crate) fn save_unique(dir: &Path, name_of: impl Fn(u32) -> String, bytes: &[u8]) -> std::io::Result<String> {
    use std::io::{ErrorKind, Write};
    for k in 1..=999 {
        let name = name_of(k);
        let path = dir.join(&name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                if let Err(e) = f.write_all(bytes) {
                    drop(f);
                    _ = std::fs::remove_file(&path);
                    return Err(e);
                }
                return Ok(name);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(ErrorKind::AlreadyExists.into())
}

/// The status line shows this on its next tick. Set by the worker, taken by `update_status`.
pub(crate) static SAVED: Mutex<Option<String>> = Mutex::new(None);

/// Encodes and saves on a worker. The pane's selection, focus and folder stay as they are.
pub(crate) unsafe fn start_picture(pic: Pic, dst: String) -> bool {
    let lang = ui();
    let job = std::thread::Builder::new().name("grids-image".into()).spawn(move || {
        let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
        let png = match pic {
            Pic::Png(b) => Ok(b),
            Pic::Dib(d) => dib_to_bmp(&d)
                .ok_or_else(|| match lang {
                    1 => "읽을 수 없는 그림".to_string(),
                    2 => "読めない画像".to_string(),
                    _ => "unreadable picture".to_string(),
                })
                .and_then(|b| to_png(&b).map_err(|e| e.message())),
        };
        let now = GetLocalTime();
        match png.and_then(|b| save_unique(Path::new(&dst), |k| image_name(lang, &now, k), &b).map_err(|e| e.to_string())) {
            Ok(name) => {
                let note = match lang {
                    1 => format!("{name} 저장함"),
                    2 => format!("{name} を保存しました"),
                    _ => format!("Saved {name}"),
                };
                *SAVED.lock().unwrap_or_else(|e| e.into_inner()) = Some(note);
            }
            Err(e) => {
                let what = match lang {
                    1 => "클립보드 그림을 저장하지 못했습니다: ",
                    2 => "クリップボードの画像を保存できませんでした: ",
                    _ => "Couldn't save the clipboard picture: ",
                };
                MessageBoxW(None, &HSTRING::from(format!("{what}{e}")), w!("Grids"), MB_ICONWARNING);
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    keep_job(job);
    true
}

#[repr(C)]
struct EncVtbl {
    unk: [usize; 3],
    initialize: usize,
    get_container: usize,
    get_info: usize,
    set_colors: usize,
    set_palette: usize,
    set_thumb: usize,
    set_preview: usize,
    create_frame: unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
}

#[repr(C)]
struct FrameVtbl {
    unk: [usize; 3],
    initialize: unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) -> HRESULT,
}

unsafe fn wic_frame(enc: &IWICBitmapEncoder) -> Result<IWICBitmapFrameEncode> {
    use windows::core::Interface;
    let vt = &*(Interface::vtable(enc) as *const _ as *const EncVtbl);
    let mut raw = std::ptr::null_mut();
    let mut bag = std::ptr::null_mut();
    (vt.create_frame)(Interface::as_raw(enc), &mut raw, &mut bag).ok()?;
    if !bag.is_null() {
        let release = *(bag as *const () as *const *const usize).add(2);
        let release: unsafe extern "system" fn(*mut core::ffi::c_void) -> u32 = std::mem::transmute(release);
        release(bag);
    }
    let frame: IWICBitmapFrameEncode = Interface::from_raw(raw);
    let init = (*(Interface::vtable(&frame) as *const _ as *const FrameVtbl)).initialize;
    init(Interface::as_raw(&frame), std::ptr::null_mut()).ok()?;
    Ok(frame)
}

unsafe fn to_png(bmp: &[u8]) -> Result<Vec<u8>> {
    let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
    let input = SHCreateMemStream(Some(bmp)).ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
    let src = wic.CreateDecoderFromStream(&input, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?.GetFrame(0)?;
    let out = SHCreateMemStream(None).ok_or_else(|| Error::from(E_OUTOFMEMORY))?;
    let enc = wic.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null())?;
    enc.Initialize(&out, WICBitmapEncoderNoCache)?;
    // CreateNewFrame's options bag is IPropertyBag2. Passing null avoids the StructuredStorage
    // feature, which is large enough that a release build of it is killed on a small machine.
    let frame = wic_frame(&enc)?;
    frame.WriteSource(&src, std::ptr::null())?;
    frame.Commit()?;
    enc.Commit()?;
    let mut len = 0u64;
    out.Seek(0, STREAM_SEEK_END, Some(&mut len))?;
    out.Seek(0, STREAM_SEEK_SET, None)?;
    let mut png = vec![0u8; len as usize];
    let mut read = 0u32;
    out.Read(png.as_mut_ptr().cast(), len as u32, Some(&mut read)).ok()?;
    png.truncate(read as usize);
    Ok(png)
}

fn keep_job(job: std::thread::JoinHandle<()>) {
    JOBS.with_borrow_mut(|j| {
        j.retain(|j| !j.is_finished());
        j.push(job);
    });
}

// Copies (or, with `mv`, moves) `srcs` into `dst` on a worker thread. False = the thread didn't start.
pub(crate) unsafe fn start_copy(srcs: Vec<String>, dst: String, rename: bool, mv: bool) -> bool {
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Err(e) = run_copy(&srcs, &dst, rename, mv) {
            if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                let what = if mv { tr("Moving failed: ", "이동하지 못했습니다: ", "移動できませんでした: ") } else { tr("Copying failed: ", "복사하지 못했습니다: ", "コピーできませんでした: ") };
                let text = HSTRING::from(format!("{what}{}", e.message()));
                MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    keep_job(job);
    true
}

// No owner window: the progress and conflict dialogs belong to the worker thread.
pub(crate) unsafe fn run_copy(srcs: &[String], dst: &str, rename: bool, mv: bool) -> Result<()> {
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMMKDIR | FOFX_ADDUNDORECORD;
    op.SetOperationFlags(if rename { flags | FOF_RENAMEONCOLLISION } else { flags })?;
    let to: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(dst), None)?;
    for s in srcs {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(s), None)?;
        if mv {
            op.MoveItem(&item, &to, PCWSTR::null(), None)?;
        } else {
            op.CopyItem(&item, &to, PCWSTR::null(), None)?;
        }
    }
    op.PerformOperations()
}

pub(crate) unsafe fn delete_selection(i: usize, shift: bool) {
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let Ok(paths) = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) }) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if paths.is_empty() {
        return;
    }
    let (ask, permanent) = app(|a| (a.s.confirm, shift || a.s.del == 1));
    if ask {
        let n = paths.len();
        let verb = if permanent {
            tr("Permanently delete", "완전히 삭제", "完全に削除")
        } else {
            tr("Move to the Recycle Bin", "휴지통으로 이동", "ごみ箱へ移動")
        };
        let text = HSTRING::from(format!("{verb} ({n})"));
        let owner = app(|a| a.main);
        if MessageBoxW(Some(owner), &text, w!("Grids"), MB_YESNO | MB_ICONQUESTION) != IDYES {
            return;
        }
    }
    start_delete(paths, permanent);
}

pub(crate) unsafe fn start_delete(srcs: Vec<String>, permanent: bool) -> bool {
    let busy = Busy::new();
    let job = std::thread::Builder::new().spawn(move || {
        let _busy = busy;
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        if let Err(e) = run_delete(&srcs, permanent) {
            if e.code() != COPYENGINE_E_USER_CANCELLED && e.code() != ERROR_CANCELLED.to_hresult() {
                let text = HSTRING::from(format!("{}{}", tr("Delete failed: ", "삭제하지 못했습니다: ", "削除できませんでした: "), e.message()));
                _ = MessageBoxW(None, &text, w!("Grids"), MB_ICONWARNING);
            }
        }
        if com {
            CoUninitialize();
        }
    });
    let Ok(job) = job else { return false };
    keep_job(job);
    true
}

pub(crate) unsafe fn run_delete(srcs: &[String], permanent: bool) -> Result<()> {
    let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
    let mut flags = FOF_NOCONFIRMATION | FOF_NOCONFIRMMKDIR;
    if !permanent {
        flags |= FOF_ALLOWUNDO | FOFX_ADDUNDORECORD;
    }
    op.SetOperationFlags(flags)?;
    for s in srcs {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(s), None)?;
        op.DeleteItem(&item, None)?;
    }
    op.PerformOperations()
}

pub(crate) fn default_new_name() -> String {
    tr("New Text Document.txt", "새 텍스트 문서.txt", "新しいテキスト ドキュメント.txt").into()
}

pub(crate) unsafe fn create_text_file() {
    let i = active_pane();
    let dir = app(|a| a.s.paths[i].clone());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = app(|a| if a.s.new_name.trim().is_empty() { default_new_name() } else { a.s.new_name.clone() });
    let dirp = PathBuf::from(&dir);
    let mut path = dirp.join(&base);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut n = 2u32;
    while path.exists() && n < 1000 {
        let name = if ext.is_empty() { format!("{stem} ({n})") } else { format!("{stem} ({n}).{ext}") };
        path = dirp.join(name);
        n += 1;
    }
    if std::fs::write(&path, b"").is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
}

pub(crate) unsafe fn pick_folder(owner: HWND) -> Option<String> {
    let dlg: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
    let opts = dlg.GetOptions().unwrap_or_default();
    dlg.SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST).ok()?;
    dlg.Show(Some(owner)).ok()?;
    let name = dlg.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    Some(pwstr(name))
}

pub(crate) unsafe fn pick_open(owner: HWND) -> Option<String> {
    file_dialog(owner, false)
}

pub(crate) unsafe fn file_dialog(owner: HWND, save: bool) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let mut buf = [0u16; 520];
    if save {
        for (d, c) in buf.iter_mut().zip("grids.ini".encode_utf16()) {
            *d = c;
        }
    }
    let filter: Vec<u16> = "Settings\0*.ini\0All\0*.*\0\0".encode_utf16().chain(std::iter::once(0)).collect();
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        Flags: if save { OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST } else { OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST },
        ..Default::default()
    };
    let ok = if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) };
    ok.as_bool().then(|| {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    })
}

pub(crate) fn settings_filter() -> Vec<u16> {
    let mut s = String::new();
    for (label, spec) in [(tr("JSON", "JSON", "JSON"), "*.json"), (tr("Settings", "설정", "設定"), "*.json;*.ini"), (tr("All", "모두", "すべて"), "*.*")] {
        s.push_str(label);
        s.push('\0');
        s.push_str(spec);
        s.push('\0');
    }
    s.push('\0');
    s.encode_utf16().collect()
}

/// Export / import picker. Save defaults to grids.json. Open accepts JSON and a legacy ini.
pub(crate) unsafe fn pick_settings(owner: HWND, save: bool) -> Option<String> {
    use windows::Win32::UI::Controls::Dialogs::{GetOpenFileNameW, GetSaveFileNameW, OFN_FILEMUSTEXIST, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    let mut buf = [0u16; 520];
    if save {
        for (d, c) in buf.iter_mut().zip("grids.json".encode_utf16()) {
            *d = c;
        }
    }
    let filter = settings_filter();
    let def = w!("json");
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: owner,
        lpstrFilter: PCWSTR(filter.as_ptr()),
        lpstrFile: PWSTR(buf.as_mut_ptr()),
        nMaxFile: buf.len() as u32,
        Flags: if save { OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST } else { OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST },
        lpstrDefExt: if save { def } else { PCWSTR::null() },
        ..Default::default()
    };
    let ok = if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) };
    ok.as_bool().then(|| {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    })
}

pub(crate) unsafe fn reset_prefs() {
    let fresh = Session::default();
    app(|a| {
        a.s.theme = fresh.theme;
        a.s.font = fresh.font;
        a.s.bar = fresh.bar;
        a.s.stat = fresh.stat;
        a.s.view_new = fresh.view_new;
        a.s.start_new = fresh.start_new;
        a.s.start_path.clear();
        a.s.ext = fresh.ext;
        a.s.tree_on = fresh.tree_on;
        a.s.edge = fresh.edge;
        a.s.confirm = fresh.confirm;
        a.s.del = fresh.del;
        a.s.new_name.clear();
        a.s.dropm = fresh.dropm;
        a.s.boot = fresh.boot;
        a.s.boot_file.clear();
        a.s.autosave = fresh.autosave;
        a.s.dot = fresh.dot;
        a.s.single = fresh.single;
        a.s.one_window = fresh.one_window;
        a.s.dblup = fresh.dblup;
        a.s.pane_tabs_on = fresh.pane_tabs_on;
        a.s.accent = fresh.accent;
        a.s.accent_dark = fresh.accent_dark;
    });
    set_language(Lang::Auto);
    crate::keys::reset_all();
    apply_ext(true);
    sync_pane_tabs(app(|a| a.s.pane_tabs_on));
    relayout();
    save_session();
    settings::refresh();
}

/// JSON replaces the settings. A file that is not JSON is read as a legacy ini.
pub(crate) unsafe fn import_settings(text: &str) -> bool {
    let Ok(mut s) = store::import_text(text) else { return false };
    let (win, lang_now) = app(|a| (a.s.win, a.s.lang));
    s.win = win;
    s.dark = resolve_dark(s.theme);
    let lang = s.lang;
    install_grids(s);
    if lang != lang_now {
        set_language(lang);
    } else {
        apply_theme();
        apply_font();
    }
    apply_ext(app(|a| a.s.ext));
    relayout();
    save_session();
    settings::refresh();
    true
}

pub(crate) fn apply_boot(s: &mut Session) {
    match s.boot {
        2 => {
            s.set_heights(vec![2, 2]);
            s.paths = vec![String::new(); MAXP];
            s.views = vec![(0, 0); MAXP];
            s.cols = vec![String::new(); MAXP];
            s.pane_folders = vec![Vec::new(); MAXP];
            s.pane_cur = vec![0; MAXP];
            s.active = 0;
            s.file.clear();
            s.tabs.clear();
            s.tab = 0;
            s.anchor();
        }
        1 if !s.boot_file.is_empty() => {
            if let Ok(t) = std::fs::read_to_string(&s.boot_file) {
                let file = s.boot_file.clone();
                s.take_layout(&Session::parse(&t));
                s.file = file;
            }
        }
        // 0, the default: every tab loaded from grids.json stays. Only the active one is shown.
        _ => {
            if s.tabs.is_empty() {
                s.anchor();
            }
            s.publish();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dib(size: u32, bits: u16, comp: u32, used: u32, extra: usize) -> Vec<u8> {
        let mut d = vec![0u8; size as usize + extra];
        d[0..4].copy_from_slice(&size.to_le_bytes());
        d[14..16].copy_from_slice(&bits.to_le_bytes());
        d[16..20].copy_from_slice(&comp.to_le_bytes());
        d[32..36].copy_from_slice(&used.to_le_bytes());
        d
    }

    #[test]
    fn paste_picture() {
        let off = |b: Vec<u8>| (u32::from_le_bytes(b[2..6].try_into().unwrap()), u32::from_le_bytes(b[10..14].try_into().unwrap()), b[..2].to_vec());
        assert_eq!(dib_to_bmp(&dib(40, 24, 0, 0, 12)).map(off), Some((66, 54, b"BM".to_vec())));
        assert_eq!(dib_to_bmp(&dib(40, 32, 3, 0, 16)).map(off), Some((70, 66, b"BM".to_vec())));
        assert_eq!(dib_to_bmp(&dib(124, 32, 3, 0, 16)).map(|b| off(b).1), Some(138));
        assert_eq!(dib_to_bmp(&dib(40, 8, 0, 0, 1024 + 4)).map(|b| off(b).1), Some(1078));
        assert_eq!(dib_to_bmp(&dib(40, 8, 0, 2, 8 + 4)).map(|b| off(b).1), Some(62));
        assert_eq!(dib_to_bmp(&dib(40, 8, 0, 0, 1024)), None);
        assert_eq!(dib_to_bmp(&dib(40, 24, 0, 0, 0)), None);
        assert_eq!(dib_to_bmp(&dib(40, 24, 0, 0, 4)[..20]), None);
        assert_eq!(dib_to_bmp(&[]), None);
        assert!(picture_only(false, false, false, false));
        for k in 0..4 {
            assert!(!picture_only(k == 0, k == 1, k == 2, k == 3));
        }
        let t = SYSTEMTIME { wYear: 2026, wMonth: 10, wDay: 21, wHour: 15, wMinute: 30, wSecond: 2, ..Default::default() };
        assert_eq!(image_name(0, &t, 1), "Clipboard image 2026-10-21 153002.png");
        assert_eq!(image_name(1, &t, 2), "클립보드 이미지 2026-10-21 153002 (2).png");
        assert_eq!(image_name(2, &t, 1), "クリップボード画像 2026-10-21 153002.png");
        let dir = std::env::temp_dir().join(format!("grids-picture-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.png"), vec![0u8; 3]).unwrap();
        let name = |k: u32| if k == 1 { "a.png".to_string() } else { format!("a ({k}).png") };
        assert_eq!(save_unique(&dir, name, b"new").unwrap(), "a (2).png");
        assert_eq!((std::fs::read(dir.join("a.png")).unwrap(), std::fs::read(dir.join("a (2).png")).unwrap()), (vec![0; 3], b"new".to_vec()));
        assert_eq!(save_unique(&dir, name, b"x").unwrap(), "a (3).png");
        _ = std::fs::remove_dir_all(&dir);
    }
}

