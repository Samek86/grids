use std::cell::{Cell, RefCell};

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::i18n::*;

// ---------- session list window: every saved session by name, load with double-click or Enter ----------

const ID_LIST: i32 = 10;
const ID_OK: i32 = 1;
const ID_CANCEL: i32 = 2;
const ID_SAVE: i32 = 3;
const LBN_DBLCLK: u32 = 2;
const LB_ADDSTRING: u32 = 0x0180;
const LB_GETCURSEL: u32 = 0x0188;
const LB_SETCURSEL: u32 = 0x0186;

thread_local! {
    static FILES: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
    static OUT: RefCell<Option<Option<PathBuf>>> = const { RefCell::new(None) };
    static SAVE_ASKED: Cell<bool> = const { Cell::new(false) };
    static LIST: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
}

/// Session files in `dir` as `(name, path)`, most recently changed first. The name is the file stem.
pub(crate) fn session_list(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(std::time::SystemTime, String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_session_file(p))
        .filter_map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Some((t, p.file_stem()?.to_string_lossy().into_owned(), p))
        })
        .collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
    v.into_iter().map(|(_, n, p)| (n, p)).collect()
}

unsafe fn finish(h: HWND, pick: bool) {
    let path = if pick {
        let k = SendMessageW(LIST.with(|l| l.get()), LB_GETCURSEL, None, None).0;
        if k < 0 {
            return;
        }
        FILES.with(|f| f.borrow().get(k as usize).cloned())
    } else {
        None
    };
    if pick && path.is_none() {
        return;
    }
    OUT.with(|o| *o.borrow_mut() = Some(path));
    let _ = DestroyWindow(h);
}

unsafe fn layout(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let s = |v: i32| v * dpi / 96;
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let (pad, bh, bw) = (s(10), s(28), s(84));
    let by = rc.bottom - pad - bh;
    let _ = MoveWindow(LIST.with(|l| l.get()), pad, pad, rc.right - 2 * pad, by - 2 * pad + s(4), true);
    if let Ok(c) = GetDlgItem(Some(h), ID_CANCEL) {
        let _ = MoveWindow(c, rc.right - pad - bw, by, bw, bh, true);
    }
    if let Ok(o) = GetDlgItem(Some(h), ID_OK) {
        let _ = MoveWindow(o, rc.right - pad - 2 * bw - s(8), by, bw, bh, true);
    }
    if let Ok(v) = GetDlgItem(Some(h), ID_SAVE) {
        let _ = MoveWindow(v, pad, by, s(110), bh, true);
    }
}

unsafe extern "system" fn pick_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COMMAND => {
            let (id, code) = ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32);
            match (id, code) {
                (ID_LIST, LBN_DBLCLK) | (ID_OK, _) => finish(h, true),
                (ID_CANCEL, _) => finish(h, false),
                (ID_SAVE, _) => {
                    SAVE_ASKED.with(|c| c.set(true));
                    finish(h, false);
                }
                _ => {}
            }
            return LRESULT(0);
        }
        WM_SIZE => layout(h),
        WM_CLOSE => {
            finish(h, false);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// Small modal window listing the sessions in the sessions folder. None means cancel or no sessions.
pub(crate) unsafe fn pick_session(owner: HWND, title: &str) -> Option<PathBuf> {
    pick_session_ex(owner, title, false)
}

/// `with_save` adds a "Save current as…" button; pressing it closes the list and sets SAVE_ASKED.
unsafe fn pick_session_ex(owner: HWND, title: &str, with_save: bool) -> Option<PathBuf> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(pick_proc),
            hInstance: m.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: w!("PanegridSessionList"),
            ..Default::default()
        });
    });
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    let items = session_list(&dir);
    SAVE_ASKED.with(|c| c.set(false));
    if items.is_empty() && !with_save {
        let text = HSTRING::from(format!(
            "{}\n{}",
            tr("No saved sessions yet.", "저장된 세션이 아직 없어요.", "保存したセッションはまだありません。"),
            dir.display()
        ));
        MessageBoxW(Some(owner), &text, &HSTRING::from(title), MB_OK | MB_ICONINFORMATION);
        return None;
    }
    OUT.with(|o| *o.borrow_mut() = None);
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = (360 * dpi / 96, 320 * dpi / 96);
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let (x, y) = (orc.left + (orc.right - orc.left - ww) / 2, orc.top + (orc.bottom - orc.top - hh) / 2);
    let style = WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("PanegridSessionList"), &HSTRING::from(title), style, x, y, ww, hh, Some(owner), None, None, None).ok()?;
    let font = app(|a| a.font);
    let tab = 0x0001_0000u32;
    // LBS_NOTIFY | WS_VSCROLL | WS_BORDER | LBS_NOINTEGRALHEIGHT
    let list = crate::child(h, w!("LISTBOX"), w!(""), 0x0001 | 0x0020_0000 | 0x0080_0000 | 0x0100 | tab, WINDOW_EX_STYLE(0), ID_LIST, font);
    LIST.with(|l| l.set(list));
    for (name, _) in &items {
        let t = HSTRING::from(name.as_str());
        SendMessageW(list, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    if items.is_empty() {
        let t = HSTRING::from(tr("(No saved sessions yet)", "(저장된 세션이 아직 없어요)", "(保存したセッションはまだありません)"));
        SendMessageW(list, LB_ADDSTRING, None, Some(LPARAM(t.as_ptr() as isize)));
    }
    SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), None);
    FILES.with(|f| *f.borrow_mut() = items.into_iter().map(|(_, p)| p).collect());
    let load = HSTRING::from(tr("Load", "불러오기", "読み込む"));
    let cancel = HSTRING::from(tr("Cancel", "취소", "キャンセル"));
    crate::child(h, w!("BUTTON"), PCWSTR(load.as_ptr()), 0x0001 | tab, WINDOW_EX_STYLE(0), ID_OK, font); // BS_DEFPUSHBUTTON
    crate::child(h, w!("BUTTON"), PCWSTR(cancel.as_ptr()), tab, WINDOW_EX_STYLE(0), ID_CANCEL, font);
    if with_save {
        let t = HSTRING::from(tr("Save current as…", "현재 상태 저장…", "現在の状態を保存…"));
        crate::child(h, w!("BUTTON"), PCWSTR(t.as_ptr()), tab, WINDOW_EX_STYLE(0), ID_SAVE, font);
    }
    layout(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(list));
    let mut msg = MSG::default();
    while OUT.with(|o| o.borrow().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_KEYDOWN && mine {
            if msg.wParam.0 == VK_RETURN.0 as usize {
                finish(h, true);
                continue;
            }
            if msg.wParam.0 == VK_ESCAPE.0 as usize {
                finish(h, false);
                continue;
            }
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    crate::keys::set_modal(false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, true);
    let _ = SetForegroundWindow(owner);
    OUT.with(|o| o.borrow_mut().take().flatten())
}

/// Toolbar Load: the list window, then load the picked session.
pub(crate) unsafe fn open_session_list() {
    let main = app(|a| a.main);
    if let Some(p) = pick_session_ex(main, tr("Load session", "세션 불러오기", "セッションを読み込む"), true) {
        load_session_file(&p);
    } else if SAVE_ASKED.with(|c| c.replace(false)) {
        _ = save_session_as();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_list_names_and_order() {
        let d = std::env::temp_dir().join(format!("pg-list-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("old.qdr"), "x").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        std::fs::write(d.join("work.grids"), "x").unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        std::fs::create_dir_all(d.join("dir.grids")).unwrap();
        let names: Vec<String> = session_list(&d).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, vec!["work".to_string(), "old".to_string()]);
        assert!(session_list(&d.join("missing")).is_empty());
        _ = std::fs::remove_dir_all(&d);
    }
}
