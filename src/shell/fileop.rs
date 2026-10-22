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
    _ = CloseClipboard();
    let copy = effect.is_none_or(|e| e & DROPEFFECT_MOVE.0 == 0 && e & DROPEFFECT_COPY.0 != 0);
    let Some(rename) = paste_mode(&srcs, &dst).filter(|_| copy) else { return false };
    start_copy(srcs, dst, rename, false)
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
    JOBS.with_borrow_mut(|j| {
        j.retain(|j| !j.is_finished());
        j.push(job);
    });
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
    JOBS.with_borrow_mut(|j| {
        j.retain(|j| !j.is_finished());
        j.push(job);
    });
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
        a.s.dblup = fresh.dblup;
        a.s.accent = fresh.accent;
        a.s.accent_dark = fresh.accent_dark;
    });
    set_language(Lang::Auto);
    crate::keys::reset_all();
    apply_ext(true);
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

