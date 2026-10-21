use crate::imports::*;
use super::*;
use crate::app::*;
use crate::ui::*;
use crate::pane::*;
use crate::i18n::*;
use crate::store;

// ---------- sessions ----------

thread_local! {
    pub(crate) static LAST_SAVED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Writes every tab (name, lock, order, folders, filters, columns) and the window.
/// The startup mode is stored beside that list; it does not gate the write.
pub(crate) unsafe fn save_session() {
    remember_tabs();
    let main = app(|a| a.main);
    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    if !main.0.is_null() {
        _ = GetWindowPlacement(main, &mut wp);
    }
    let text = app(|a| {
        if !main.0.is_null() {
            let r = wp.rcNormalPosition;
            let max = (wp.showCmd == SW_SHOWMAXIMIZED.0 as u32) as i32;
            a.s.win = Some([r.left, r.top, r.right, r.bottom, max]);
        }
        store::sync_text(&a.s)
    });
    let same = LAST_SAVED.with(|l| l.borrow().as_str() == text);
    if same || store::write_settings(&text).is_ok() {
        if !same {
            LAST_SAVED.with(|l| *l.borrow_mut() = text);
        }
        app(|a| a.save_since = None);
    }
}

/// Coalesce tab, folder and window edits into one write, and never wait longer than SAVE_LATEST_MS.
pub(crate) unsafe fn note_save() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        return;
    }
    let flush = app(|a| {
        let now = Instant::now();
        let late = a.save_since.is_some_and(|t| now.duration_since(t).as_millis() >= SAVE_LATEST_MS);
        if late {
            a.save_since = None;
            true
        } else {
            if a.save_since.is_none() {
                a.save_since = Some(now);
            }
            false
        }
    });
    if flush {
        _ = KillTimer(Some(main), TIMER_SAVE);
        save_session();
    } else {
        SetTimer(Some(main), TIMER_SAVE, SAVE_SOON_MS, None);
    }
}

// Save button / Ctrl+S: overwrite the current session file; the first time, ask for a name.
pub(crate) unsafe fn save_current() {
    let file = app(|a| a.s.file.clone());
    if file.is_empty() {
        _ = save_session_as();
    } else {
        write_session(Path::new(&file));
    }
}

pub(crate) unsafe fn write_session(path: &Path) {
    remember_tabs();
    if std::fs::write(path, app(|a| a.s.snapshot())).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    app(|a| {
        a.s.file = path.to_string_lossy().into_owned();
        (a.base, a.dirty) = (Some(a.s.clone()), false);
    });
    refresh_save_button();
}

// The named-session save dialog: naming, overwrite prompts, and the sessions folder.
pub(crate) unsafe fn ask_session_path(title: &str) -> Result<String> {
    let main = app(|a| a.main);
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
    let kind = HSTRING::from(tr("Grids session (*.grids)", "Grids 세션 (*.grids)", "Grids セッション (*.grids)"));
    dlg.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(kind.as_ptr()), pszSpec: w!("*.grids") }])?;
    dlg.SetDefaultExtension(w!("grids"))?;
    dlg.SetTitle(&HSTRING::from(title))?;
    if let Ok(f) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.to_string_lossy().as_ref()), None) {
        dlg.SetFolder(&f)?;
    }
    dlg.Show(Some(main))?; // Err on cancel
    Ok(pwstr(dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?))
}

pub(crate) unsafe fn save_session_as() -> Result<()> {
    let path = ask_session_path(tr("Save session", "세션 저장", "セッションを保存"))?;
    write_session(Path::new(&path));
    Ok(())
}

/// Writes one tab's grid to a named session. The window's own session file stays as it is.
pub(crate) unsafe fn save_tab_as_session(i: usize) {
    remember_tabs();
    let Some(text) = app(|a| tab_session_text(&a.s, i)) else { return };
    let Ok(path) = ask_session_path(tr("Save this tab as session", "이 탭을 세션으로 저장", "このタブをセッションとして保存")) else { return };
    if std::fs::write(Path::new(&path), text).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

pub(crate) unsafe fn load_session_file(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    let other = Session::parse(&text);
    let mut s = app(|a| a.s.clone());
    s.take_layout(&other);
    s.file = path.to_string_lossy().into_owned();
    install_grids(s);
    app(|a| (a.base, a.dirty) = (Some(a.s.clone()), false));
    refresh_save_button();
    relayout();
    sync_filter_edits();
    redraw_all();
    focus_view(active_pane());
}

// Paths from the command line or a second instance: a session file loads, folders fill panes from the active one on.
pub(crate) unsafe fn open_args(args: Vec<String>) {
    if let Some(f) = session_arg(&args) {
        return load_session_file(Path::new(f));
    }
    if app(|x| x.zoom.take()).is_some() {
        relayout();
        redraw_all();
    }
    let (a, n) = app(|x| (x.s.active, x.s.panes()));
    for (k, p) in args.iter().take(n).enumerate() {
        navigate((a + k) % n, p);
    }
    focus_view(a);
}

// Window of a running copy of this same exe. A portable.txt copy keeps its own grids.json.
pub(crate) unsafe fn running_instance() -> Option<HWND> {
    let me = std::env::current_exe().ok()?.to_string_lossy().to_lowercase();
    let mut h = None;
    loop {
        let w = FindWindowExW(None, h, w!("GridsMain"), PCWSTR::null()).ok()?;
        h = Some(w);
        let mut pid = 0;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { continue };
        let (mut buf, mut n) = ([0u16; 1024], 1024u32);
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        _ = CloseHandle(p);
        if ok && String::from_utf16_lossy(&buf[..n as usize]).to_lowercase() == me {
            return Some(w);
        }
    }
}

#[repr(C)]
pub(crate) struct CopyData {
    pub(crate) dw: usize,
    pub(crate) cb: u32,
    pub(crate) lp: *const core::ffi::c_void,
}

