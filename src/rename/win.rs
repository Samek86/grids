// Separate window for the selected items. Painted like Settings (theme, accent, DPI);
// edits and buttons are real child controls on that paint.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Once;

use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWINDOWATTRIBUTE};
use windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
use windows::Win32::UI::HiDpi::GetDpiForWindow;

use crate::imports::*;
use crate::tr;

use super::engine::{self, BatchItem, CaseMode, DatePos, DateSrc, Issue, NumPos, PreviewRow, Rule, TrimEdge};
use super::exec;

const ID_ADD: i32 = 1;
const ID_CLOSE: i32 = 2;
const ID_APPLY: i32 = 3;
const ID_UNDO: i32 = 4;
const ID_BASE: i32 = 100;
const STRIDE: i32 = 16;
const MAX_RULES: usize = 24;

const F_KIND: i32 = 0;
const F_A: i32 = 1;
const F_B: i32 = 2;
const F_CASE: i32 = 3;
const F_EXT: i32 = 4;
const F_UP: i32 = 5;
const F_DOWN: i32 = 6;
const F_DEL: i32 = 7;
const F_MODE: i32 = 8;
const F_POS: i32 = 9;
const F_C: i32 = 10;
const F_SRC: i32 = 11;
const F_AT: i32 = 12;

const CB_RESETCONTENT: u32 = 0x014B;
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const CBN_SELCHANGE: u32 = 1;
const EN_CHANGE: u32 = 0x0300;
const BN_CLICKED: u32 = 0;
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const EM_SETCUEBANNER: u32 = 0x1501;
const EM_SETMARGINS: u32 = 0x00D3;

fn sc(dpi: i32, v: i32) -> i32 {
    v * dpi.max(96) / 96
}

fn s(en: &'static str, ko: &'static str, ja: &'static str) -> [&'static str; 3] {
    [en, ko, ja]
}

fn tx(t: [&'static str; 3]) -> &'static str {
    tr(t[0], t[1], t[2])
}

struct St {
    hwnd: HWND,
    host: HWND,
    dpi: i32,
    font: HFONT,
    small: HFONT,
    rules: Vec<Rule>,
    items: Vec<BatchItem>,
    existing: HashSet<(String, String)>,
    rows: Vec<PreviewRow>,
    rule_scroll: i32,
    preview_scroll: i32,
    filling: bool,
    busy: bool,
    note: String,
    /// Names to write onto `items` when the worker reports success. Parallel to `items`.
    pending: Vec<String>,
    pending_note: String,
    clear_rules_on_ok: bool,
}

impl Default for St {
    fn default() -> Self {
        Self {
            hwnd: HWND::default(),
            host: HWND::default(),
            dpi: 96,
            font: HFONT::default(),
            small: HFONT::default(),
            rules: vec![Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false }],
            items: Vec::new(),
            existing: HashSet::new(),
            rows: Vec::new(),
            rule_scroll: 0,
            preview_scroll: 0,
            filling: false,
            busy: false,
            note: String::new(),
            pending: Vec::new(),
            pending_note: String::new(),
            clear_rules_on_ok: false,
        }
    }
}

thread_local! {
    static ST: RefCell<St> = RefCell::new(St::default());
}

fn hwnd() -> HWND {
    ST.with(|s| s.borrow().hwnd)
}

pub(crate) unsafe fn open_active() {
    let i = crate::active_pane();
    open_items(collect_pane(i));
}

pub(crate) unsafe fn open_paths(_owner: HWND, items: &[(String, bool)]) {
    let batch = items.iter().filter_map(|(p, file)| item_from_path(p, !file)).collect();
    open_items(batch);
}

pub(crate) fn menu_label() -> String {
    let name = tr("Batch rename…", "일괄 이름 바꾸기…", "一括リネーム…");
    let chord = crate::keys::index("file.rename").map(crate::keys::chord_text).unwrap_or_default();
    if chord.is_empty() {
        name.to_string()
    } else {
        format!("{name}\t{chord}")
    }
}

unsafe fn open_items(items: Vec<BatchItem>) {
    if items.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    ensure_class();
    let existing = load_existing(&items);
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.items = items;
            st.existing = existing;
            st.preview_scroll = 0;
            st.note.clear();
        });
        recompute();
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(h);
        return;
    }
    let owner = crate::app(|a| a.main);
    let dpi = if owner.0.is_null() { 96 } else { GetDpiForWindow(owner) as i32 };
    let (ww, hh) = (sc(dpi.max(96), 980), sc(dpi.max(96), 700));
    let mut orc = RECT::default();
    if !owner.0.is_null() {
        let _ = GetWindowRect(owner, &mut orc);
    }
    let x = orc.left + ((orc.right - orc.left - ww) / 2).max(0);
    let y = orc.top + ((orc.bottom - orc.top - hh) / 2).max(0);
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE;
    let h = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("PanegridRename"),
        &HSTRING::from(tx(s("Batch rename", "일괄 이름 바꾸기", "一括リネーム"))),
        style,
        x,
        y,
        ww,
        hh,
        (!owner.0.is_null()).then_some(owner),
        None,
        None,
        None,
    )
    .unwrap_or_default();
    if h.0.is_null() {
        return;
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.items = items;
        st.existing = existing;
    });
    recompute();
}

pub(crate) unsafe fn close() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub(crate) unsafe fn captures(h: HWND) -> bool {
    let w = hwnd();
    !w.0.is_null() && IsWindow(Some(w)).as_bool() && (h == w || IsChild(w, h).as_bool())
}

pub(crate) unsafe fn refresh() {
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    style_window(h);
    rebuild_fonts();
    let font = ST.with(|s| s.borrow().font);
    apply_font(h, font);
    let host = ST.with(|s| s.borrow().host);
    if !host.0.is_null() {
        apply_font(host, font);
    }
    let _ = SetWindowTextW(h, &HSTRING::from(tx(s("Batch rename", "일괄 이름 바꾸기", "一括リネーム"))));
    set_btn(h, ID_ADD, tx(s("Add rule", "규칙 추가", "ルールを追加")));
    set_btn(h, ID_CLOSE, tx(s("Close", "닫기", "閉じる")));
    set_btn(h, ID_APPLY, tx(s("Rename", "이름 바꾸기", "名前の変更")));
    set_btn(h, ID_UNDO, tx(s("Undo", "되돌리기", "元に戻す")));
    let n = ST.with(|s| s.borrow().rules.len());
    ST.with(|s| s.borrow_mut().filling = true);
    for i in 0..n {
        refill_kind(i);
        refill_mode(i);
        let rule = ST.with(|s| s.borrow().rules.get(i).cloned());
        if let Some(rule) = rule {
            write_rule(i, &rule);
        }
    }
    ST.with(|s| s.borrow_mut().filling = false);
    layout(h);
    sync_buttons();
    let _ = InvalidateRect(Some(h), None, true);
    if !host.0.is_null() {
        let _ = InvalidateRect(Some(host), None, true);
    }
}

/// Esc closes. Ctrl+Enter renames when the preview is clean. Anything else is typing.
pub(crate) unsafe fn on_key(vk: u16, ctrl: bool, alt: bool, _shift: bool) -> bool {
    if !ctrl && !alt && VIRTUAL_KEY(vk) == VK_ESCAPE {
        close();
        return true;
    }
    if ctrl && !alt && VIRTUAL_KEY(vk) == VK_RETURN {
        apply();
        return true;
    }
    false
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(rename_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: w!("PanegridRename"),
            ..Default::default()
        });
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(host_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("PanegridRenameRules"),
            ..Default::default()
        });
    });
}

unsafe extern "system" fn rename_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let dpi = GetDpiForWindow(h) as i32;
            ST.with(|s| {
                let mut st = s.borrow_mut();
                let items = std::mem::take(&mut st.items);
                let existing = std::mem::take(&mut st.existing);
                *st = St::default();
                st.hwnd = h;
                st.dpi = dpi.max(96);
                st.items = items;
                st.existing = existing;
            });
            rebuild_fonts();
            build_shell(h);
            rebuild_rule_controls();
            style_window(h);
            layout(h);
            return LRESULT(0);
        }
        WM_SIZE => {
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
        WM_DPICHANGED => {
            let dpi = (wp.0 & 0xffff) as i32;
            ST.with(|s| s.borrow_mut().dpi = dpi.max(96));
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            refresh();
        }
        WM_GETMINMAXINFO => {
            let dpi = ST.with(|s| s.borrow().dpi.max(96));
            let m = &mut *(lp.0 as *mut MINMAXINFO);
            m.ptMinTrackSize.x = sc(dpi, 760);
            m.ptMinTrackSize.y = sc(dpi, 520);
            return LRESULT(0);
        }
        WM_PAINT => paint(h),
        WM_ERASEBKGND => return LRESULT(1),
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
        WM_MOUSEWHEEL => on_wheel(((wp.0 >> 16) as i16 as i32) / 120),
        exec::WM_RENAME_DONE => on_done(h),
        WM_DESTROY => {
            drop_fonts();
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.hwnd = HWND::default();
                st.host = HWND::default();
            });
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

unsafe extern "system" fn host_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint_host(h),
        WM_ERASEBKGND => return LRESULT(1),
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => {
            on_command(hwnd(), (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32);
            return LRESULT(0);
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN => return ctl_color(wp),
        WM_MOUSEWHEEL => {
            on_wheel(((wp.0 >> 16) as i16 as i32) / 120);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

unsafe fn ctl_color(wp: WPARAM) -> LRESULT {
    let dc = HDC(wp.0 as _);
    SetBkMode(dc, TRANSPARENT);
    SetBkColor(dc, crate::cref(crate::pal().card));
    SetTextColor(dc, crate::cref(crate::pal().text));
    SetDCBrushColor(dc, crate::cref(crate::pal().card));
    LRESULT(GetStockObject(DC_BRUSH).0 as isize)
}

unsafe fn build_shell(h: HWND) {
    let font = ST.with(|s| s.borrow().font);
    let tab = 0x00010000u32;
    let btn = |id, label| {
        let c = child(h, w!("BUTTON"), label, 0xB | tab, id, font);
        theme_btn(c);
        c
    };
    btn(ID_ADD, tx(s("Add rule", "규칙 추가", "ルールを追加")));
    btn(ID_CLOSE, tx(s("Close", "닫기", "閉じる")));
    btn(ID_APPLY, tx(s("Rename", "이름 바꾸기", "名前の変更")));
    btn(ID_UNDO, tx(s("Undo", "되돌리기", "元に戻す")));
    let host = child(h, w!("PanegridRenameRules"), "", 0x02000000 | 0x04000000, 50, font);
    ST.with(|s| s.borrow_mut().host = host);
}

unsafe fn rebuild_rule_controls() {
    let host = ST.with(|s| s.borrow().host);
    if host.0.is_null() {
        return;
    }
    for i in 0..MAX_RULES {
        for f in 0..STRIDE {
            let c = ctl(host, id(i, f));
            if !c.0.is_null() {
                let _ = DestroyWindow(c);
            }
        }
    }
    let n = ST.with(|s| s.borrow().rules.len());
    let font = ST.with(|s| s.borrow().font);
    ST.with(|s| s.borrow_mut().filling = true);
    for i in 0..n {
        make_rule_controls(host, i, font);
        refill_kind(i);
        refill_mode(i);
        let rule = ST.with(|s| s.borrow().rules.get(i).cloned());
        if let Some(rule) = rule {
            write_rule(i, &rule);
        }
    }
    ST.with(|s| s.borrow_mut().filling = false);
}

unsafe fn make_rule_controls(host: HWND, i: usize, font: HFONT) {
    let tab = 0x00010000u32;
    let combo = |f| {
        let c = child(host, w!("COMBOBOX"), "", 3 | 0x200 | 0x00200000 | tab, id(i, f), font);
        theme_ctl(c);
        c
    };
    let edit = |f| {
        let c = child(host, w!("EDIT"), "", 0x80 | tab, id(i, f), font);
        theme_ctl(c);
        let m = sc(ST.with(|s| s.borrow().dpi), 6) as isize;
        let _ = SendMessageW(c, EM_SETMARGINS, Some(WPARAM(3)), Some(LPARAM(m | (m << 16))));
        c
    };
    let check = |f, label: &str| {
        let c = child(host, w!("BUTTON"), label, 0x3 | tab, id(i, f), font);
        theme_btn(c);
        c
    };
    let mini = |f, label: &str| {
        let c = child(host, w!("BUTTON"), label, 0xB | tab, id(i, f), font);
        theme_btn(c);
        c
    };
    combo(F_KIND);
    edit(F_A);
    edit(F_B);
    edit(F_C);
    edit(F_AT);
    check(F_CASE, tx(s("Match case", "대소문자 구분", "大文字と小文字を区別")));
    check(F_EXT, tx(s("Include extension", "확장자 포함", "拡張子を含める")));
    combo(F_MODE);
    combo(F_POS);
    combo(F_SRC);
    mini(F_UP, "↑");
    mini(F_DOWN, "↓");
    mini(F_DEL, "×");
}

unsafe fn refill_kind(i: usize) {
    let host = ST.with(|s| s.borrow().host);
    let c = ctl(host, id(i, F_KIND));
    if c.0.is_null() {
        return;
    }
    let cur = SendMessageW(c, CB_GETCURSEL, None, None).0;
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in [
        tx(s("Replace text", "텍스트 바꾸기", "テキストを置換")),
        tx(s("Add prefix", "접두사 추가", "接頭辞を追加")),
        tx(s("Add suffix", "접미사 추가", "接尾辞を追加")),
        tx(s("Change case", "대소문자 변경", "大文字と小文字")),
        tx(s("Change extension", "확장자 변경", "拡張子を変更")),
        tx(s("Number", "번호", "連番")),
        tx(s("Date", "날짜", "日付")),
        tx(s("Insert", "삽입", "挿入")),
        tx(s("Delete characters", "글자 삭제", "文字を削除")),
        tx(s("Trim", "다듬기", "トリム")),
    ] {
        let hs = HSTRING::from(label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    let rule = ST.with(|s| s.borrow().rules.get(i).cloned());
    let sel = rule.as_ref().map(kind_index).unwrap_or(cur.max(0) as usize) as isize;
    let _ = SendMessageW(c, CB_SETCURSEL, Some(WPARAM(sel as usize)), None);
}

unsafe fn refill_mode(i: usize) {
    let host = ST.with(|s| s.borrow().host);
    let c = ctl(host, id(i, F_MODE));
    if c.0.is_null() {
        return;
    }
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in [
        tx(s("lowercase", "소문자", "小文字")),
        tx(s("UPPERCASE", "대문자", "大文字")),
        tx(s("Title Case", "단어 첫 글자", "単語の先頭")),
        tx(s("Sentence", "문장", "文頭のみ")),
    ] {
        let hs = HSTRING::from(label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
}

unsafe fn fill_combo(host: HWND, i: usize, field: i32, labels: &[&str]) {
    let c = ctl(host, id(i, field));
    if c.0.is_null() {
        return;
    }
    let _ = SendMessageW(c, CB_RESETCONTENT, None, None);
    for label in labels {
        let hs = HSTRING::from(*label);
        let _ = SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
}

unsafe fn select_combo(host: HWND, i: usize, field: i32, sel: usize) {
    let _ = SendMessageW(ctl(host, id(i, field)), CB_SETCURSEL, Some(WPARAM(sel)), None);
}

unsafe fn write_rule(i: usize, rule: &Rule) {
    let host = ST.with(|s| s.borrow().host);
    let set_edit = |f, text: &str| {
        let c = ctl(host, id(i, f));
        let _ = SetWindowTextW(c, &HSTRING::from(text));
    };
    let set_check = |f, on: bool| {
        let _ = SendMessageW(ctl(host, id(i, f)), BM_SETCHECK, Some(WPARAM(usize::from(on))), None);
    };
    select_combo(host, i, F_KIND, kind_index(rule));
    match rule {
        Rule::Replace { find, replace, case_sensitive, include_ext } => {
            set_edit(F_A, find);
            set_edit(F_B, replace);
            set_check(F_CASE, *case_sensitive);
            set_check(F_EXT, *include_ext);
            cue(ctl(host, id(i, F_A)), tx(s("Find", "찾을 내용", "検索")));
            cue(ctl(host, id(i, F_B)), tx(s("Replace with", "바꿀 내용", "置換")));
        }
        Rule::Prefix { text } => {
            set_edit(F_A, text);
            cue(ctl(host, id(i, F_A)), tx(s("Prefix", "접두사", "接頭辞")));
        }
        Rule::Suffix { text } => {
            set_edit(F_A, text);
            cue(ctl(host, id(i, F_A)), tx(s("Suffix", "접미사", "接尾辞")));
        }
        Rule::Case { mode, include_ext } => {
            select_combo(host, i, F_MODE, mode_index(*mode));
            set_check(F_EXT, *include_ext);
        }
        Rule::Extension { ext } => {
            set_edit(F_A, ext);
            cue(ctl(host, id(i, F_A)), tx(s("New extension", "새 확장자", "新しい拡張子")));
        }
        Rule::Number { start, step, pad, pos } => {
            set_edit(F_A, &start.to_string());
            set_edit(F_B, &step.to_string());
            set_edit(F_C, &pad.to_string());
            cue(ctl(host, id(i, F_A)), tx(s("Start", "시작", "開始")));
            cue(ctl(host, id(i, F_B)), tx(s("Step", "간격", "間隔")));
            cue(ctl(host, id(i, F_C)), tx(s("Digits", "자릿수", "桁数")));
            fill_combo(host, i, F_POS, &[
                tx(s("Before name", "이름 앞", "名前の前")),
                tx(s("After name", "이름 뒤", "名前の後")),
                tx(s("At position", "위치 지정", "位置を指定")),
            ]);
            let (sel, at) = match pos {
                NumPos::Prefix => (0, String::new()),
                NumPos::Suffix => (1, String::new()),
                NumPos::At(n) => (2, n.to_string()),
            };
            select_combo(host, i, F_POS, sel);
            set_edit(F_AT, &at);
            cue(ctl(host, id(i, F_AT)), tx(s("0 = start, -1 = end", "0 = 앞, -1 = 끝", "0 = 先頭、-1 = 末尾")));
        }
        Rule::Date { src, pattern, pos } => {
            fill_combo(host, i, F_SRC, &[
                tx(s("Modified", "수정한 날짜", "更新日時")),
                tx(s("Created", "만든 날짜", "作成日時")),
                tx(s("Accessed", "연 날짜", "アクセス日時")),
            ]);
            select_combo(host, i, F_SRC, match src {
                DateSrc::Modified => 0,
                DateSrc::Created => 1,
                DateSrc::Accessed => 2,
            });
            set_edit(F_A, pattern);
            cue(ctl(host, id(i, F_A)), "YYYY-MM-DD hh:mm:ss");
            fill_combo(host, i, F_POS, &[
                tx(s("Before name", "이름 앞", "名前の前")),
                tx(s("After name", "이름 뒤", "名前の後")),
                tx(s("Replace name", "이름 대신", "名前を置換")),
            ]);
            select_combo(host, i, F_POS, match pos {
                DatePos::Prefix => 0,
                DatePos::Suffix => 1,
                DatePos::Replace => 2,
            });
        }
        Rule::Insert { text, at } => {
            set_edit(F_A, text);
            set_edit(F_AT, &at.to_string());
            cue(ctl(host, id(i, F_A)), tx(s("Text", "텍스트", "テキスト")));
            cue(ctl(host, id(i, F_AT)), tx(s("0 = start, -1 = end", "0 = 앞, -1 = 끝", "0 = 先頭、-1 = 末尾")));
        }
        Rule::Delete { at, len } => {
            set_edit(F_AT, &at.to_string());
            set_edit(F_C, &len.to_string());
            cue(ctl(host, id(i, F_AT)), tx(s("0 = start, -1 = end", "0 = 앞, -1 = 끝", "0 = 先頭、-1 = 末尾")));
            cue(ctl(host, id(i, F_C)), tx(s("Length", "길이", "長さ")));
        }
        Rule::Trim { chars, edge } => {
            set_edit(F_A, chars);
            cue(ctl(host, id(i, F_A)), tx(s("Characters (empty = spaces)", "지울 문자 (비우면 공백)", "文字（空なら空白）")));
            fill_combo(host, i, F_POS, &[
                tx(s("Both ends", "양쪽", "両端")),
                tx(s("Left", "왼쪽", "左")),
                tx(s("Right", "오른쪽", "右")),
            ]);
            select_combo(host, i, F_POS, match edge {
                TrimEdge::Both => 0,
                TrimEdge::Left => 1,
                TrimEdge::Right => 2,
            });
        }
    }
}

unsafe fn read_rule(i: usize) -> Option<Rule> {
    let host = ST.with(|s| s.borrow().host);
    let kind = SendMessageW(ctl(host, id(i, F_KIND)), CB_GETCURSEL, None, None).0;
    if kind < 0 {
        return ST.with(|s| s.borrow().rules.get(i).cloned());
    }
    let text = |f| crate::text_of(ctl(host, id(i, f)));
    let on = |f| SendMessageW(ctl(host, id(i, f)), BM_GETCHECK, None, None).0 == 1;
    let sel = |f| SendMessageW(ctl(host, id(i, f)), CB_GETCURSEL, None, None).0;
    Some(match kind {
        1 => Rule::Prefix { text: text(F_A) },
        2 => Rule::Suffix { text: text(F_A) },
        3 => Rule::Case { mode: mode_from(sel(F_MODE)), include_ext: on(F_EXT) },
        4 => Rule::Extension { ext: text(F_A) },
        5 => {
            let pos = match sel(F_POS) {
                1 => NumPos::Suffix,
                2 => NumPos::At(parse_i32(&text(F_AT), 0)),
                _ => NumPos::Prefix,
            };
            Rule::Number { start: parse_i64(&text(F_A), 1), step: parse_i64(&text(F_B), 1), pad: parse_pad(&text(F_C)), pos }
        }
        6 => {
            let src = match sel(F_SRC) {
                1 => DateSrc::Created,
                2 => DateSrc::Accessed,
                _ => DateSrc::Modified,
            };
            let pos = match sel(F_POS) {
                1 => DatePos::Suffix,
                2 => DatePos::Replace,
                _ => DatePos::Prefix,
            };
            Rule::Date { src, pattern: text(F_A), pos }
        }
        7 => Rule::Insert { text: text(F_A), at: parse_i32(&text(F_AT), 0) },
        8 => Rule::Delete { at: parse_i32(&text(F_AT), 0), len: parse_usize(&text(F_C), 1) },
        9 => {
            let edge = match sel(F_POS) {
                1 => TrimEdge::Left,
                2 => TrimEdge::Right,
                _ => TrimEdge::Both,
            };
            Rule::Trim { chars: text(F_A), edge }
        }
        _ => Rule::Replace { find: text(F_A), replace: text(F_B), case_sensitive: on(F_CASE), include_ext: on(F_EXT) },
    })
}

fn kind_index(rule: &Rule) -> usize {
    match rule {
        Rule::Replace { .. } => 0,
        Rule::Prefix { .. } => 1,
        Rule::Suffix { .. } => 2,
        Rule::Case { .. } => 3,
        Rule::Extension { .. } => 4,
        Rule::Number { .. } => 5,
        Rule::Date { .. } => 6,
        Rule::Insert { .. } => 7,
        Rule::Delete { .. } => 8,
        Rule::Trim { .. } => 9,
    }
}

fn default_rule(kind: isize) -> Rule {
    match kind {
        1 => Rule::Prefix { text: String::new() },
        2 => Rule::Suffix { text: String::new() },
        3 => Rule::Case { mode: CaseMode::Lower, include_ext: false },
        4 => Rule::Extension { ext: String::new() },
        5 => Rule::Number { start: 1, step: 1, pad: 0, pos: NumPos::Suffix },
        6 => Rule::Date { src: DateSrc::Modified, pattern: "YYYY-MM-DD".into(), pos: DatePos::Prefix },
        7 => Rule::Insert { text: String::new(), at: 0 },
        8 => Rule::Delete { at: 0, len: 1 },
        9 => Rule::Trim { chars: String::new(), edge: TrimEdge::Both },
        _ => Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false },
    }
}

/// Empty text and a lone sign stay at the default so a minus can be typed.
fn parse_i64(text: &str, default: i64) -> i64 {
    let t = text.trim();
    if t.is_empty() || t == "-" || t == "+" {
        return default;
    }
    t.parse().unwrap_or(default)
}

fn parse_i32(text: &str, default: i32) -> i32 {
    parse_i64(text, default as i64).clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

fn parse_pad(text: &str) -> u8 {
    parse_i64(text, 0).clamp(0, 16) as u8
}

fn parse_usize(text: &str, default: i64) -> usize {
    parse_i64(text, default).clamp(0, 10_000) as usize
}

fn mode_index(mode: CaseMode) -> usize {
    match mode {
        CaseMode::Lower => 0,
        CaseMode::Upper => 1,
        CaseMode::Title => 2,
        CaseMode::Sentence => 3,
    }
}

fn mode_from(i: isize) -> CaseMode {
    match i {
        1 => CaseMode::Upper,
        2 => CaseMode::Title,
        3 => CaseMode::Sentence,
        _ => CaseMode::Lower,
    }
}

unsafe fn on_command(_h: HWND, id: i32, code: u32) {
    if ST.with(|s| s.borrow().filling) {
        return;
    }
    match id {
        ID_ADD => add_rule(),
        ID_CLOSE => close(),
        ID_APPLY => apply(),
        ID_UNDO => undo(),
        _ if id >= ID_BASE => {
            let i = ((id - ID_BASE) / STRIDE) as usize;
            let f = (id - ID_BASE) % STRIDE;
            if i >= ST.with(|s| s.borrow().rules.len()) {
                return;
            }
            match f {
                F_UP if code == BN_CLICKED => move_rule(i, -1),
                F_DOWN if code == BN_CLICKED => move_rule(i, 1),
                F_DEL if code == BN_CLICKED => remove_rule(i),
                F_KIND if code == CBN_SELCHANGE => change_kind(i),
                F_MODE if code == CBN_SELCHANGE => sync_rule(i),
                F_POS | F_SRC if code == CBN_SELCHANGE => {
                    sync_rule(i);
                    layout(hwnd());
                }
                F_A | F_B | F_C | F_AT if code == EN_CHANGE => sync_rule(i),
                F_CASE | F_EXT if code == BN_CLICKED => sync_rule(i),
                _ => {}
            }
        }
        _ => {}
    }
}

unsafe fn sync_rule(i: usize) {
    let Some(rule) = read_rule(i) else { return };
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(slot) = st.rules.get_mut(i) {
            *slot = rule;
        }
        st.note.clear();
    });
    recompute();
}

unsafe fn change_kind(i: usize) {
    let host = ST.with(|s| s.borrow().host);
    let kind = SendMessageW(ctl(host, id(i, F_KIND)), CB_GETCURSEL, None, None).0;
    let rule = default_rule(kind);
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if let Some(slot) = st.rules.get_mut(i) {
            *slot = rule.clone();
        }
        st.filling = true;
        st.note.clear();
    });
    write_rule(i, &rule);
    ST.with(|s| s.borrow_mut().filling = false);
    layout(hwnd());
    recompute();
}

unsafe fn add_rule() {
    let n = ST.with(|s| s.borrow().rules.len());
    if n >= MAX_RULES {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.rules.push(Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false });
        st.note.clear();
    });
    rebuild_rule_controls();
    let dpi = ST.with(|s| s.borrow().dpi);
    let rules = ST.with(|s| s.borrow().rules.clone());
    let content = rules_span(&rules, dpi);
    let view = host_height();
    let max = (content - view).max(0);
    ST.with(|s| s.borrow_mut().rule_scroll = max);
    layout(hwnd());
    recompute();
    let host = ST.with(|s| s.borrow().host);
    let focus_at = ST.with(|s| s.borrow().rules.len()).saturating_sub(1);
    let _ = SetFocus(Some(ctl(host, id(focus_at, F_A))));
}

unsafe fn remove_rule(i: usize) {
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if i < st.rules.len() {
            st.rules.remove(i);
        }
        st.note.clear();
    });
    rebuild_rule_controls();
    layout(hwnd());
    recompute();
}

unsafe fn move_rule(i: usize, dir: isize) {
    let j = i as isize + dir;
    let n = ST.with(|s| s.borrow().rules.len()) as isize;
    if j < 0 || j >= n {
        return;
    }
    // Controls hold the latest text; pull it in before the swap rebuilds them.
    if let Some(rule) = read_rule(i) {
        ST.with(|s| {
            if let Some(slot) = s.borrow_mut().rules.get_mut(i) {
                *slot = rule;
            }
        });
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.rules.swap(i, j as usize);
        st.note.clear();
    });
    rebuild_rule_controls();
    layout(hwnd());
    recompute();
}

unsafe fn apply() {
    if ST.with(|s| s.borrow().busy) {
        return;
    }
    let (pairs, pending, note) = ST.with(|s| {
        let st = s.borrow();
        let mut pairs = Vec::new();
        let mut pending = Vec::new();
        for (it, row) in st.items.iter().zip(st.rows.iter()) {
            pending.push(if row.issues.is_empty() { row.new_name.clone() } else { row.old_name.clone() });
            if row.issues.is_empty() && row.old_name != row.new_name {
                pairs.push((it.dir.clone(), row.old_name.clone(), row.new_name.clone()));
            }
        }
        let note = renamed_note(pairs.len());
        (pairs, pending, note)
    });
    if pairs.is_empty() || ST.with(|s| engine::error_count(&s.borrow().rows) > 0) {
        return;
    }
    launch(pairs, pending, note, true);
}

unsafe fn undo() {
    if ST.with(|s| s.borrow().busy) {
        return;
    }
    let Some(pairs) = exec::last_undo() else { return };
    let pending = ST.with(|s| {
        s.borrow()
            .items
            .iter()
            .map(|it| {
                pairs
                    .iter()
                    .find(|(d, from, _)| engine::dir_key(d) == engine::dir_key(&it.dir) && from == &it.name)
                    .map(|(_, _, to)| to.clone())
                    .unwrap_or_else(|| it.name.clone())
            })
            .collect::<Vec<_>>()
    });
    let note = undone_note(pairs.len());
    launch(pairs, pending, note, false);
}

unsafe fn launch(pairs: Vec<(String, String, String)>, pending: Vec<String>, note: String, record: bool) {
    let avoid = ST.with(|s| {
        let st = s.borrow();
        let mut occ = st.existing.clone();
        for (dir, from, _) in &pairs {
            occ.remove(&(engine::dir_key(dir), engine::name_key(from)));
        }
        occ
    });
    let waves = engine::plan(&pairs, &avoid);
    if waves.is_empty() {
        return;
    }
    let undo: Vec<(String, String, String)> = pairs.iter().map(|(d, from, to)| (d.clone(), to.clone(), from.clone())).collect();
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.busy = true;
        st.pending = pending;
        st.pending_note = note;
        st.clear_rules_on_ok = record;
    });
    sync_buttons();
    exec::start(hwnd(), waves, undo, record);
}

unsafe fn on_done(h: HWND) {
    let finished = exec::take_done();
    let (pending, note, clear_rules) = ST.with(|s| {
        let mut st = s.borrow_mut();
        st.busy = false;
        (std::mem::take(&mut st.pending), st.pending_note.clone(), st.clear_rules_on_ok)
    });
    let ok = finished.as_ref().is_some_and(|f| f.ok);
    if ok {
        ST.with(|s| {
            let mut st = s.borrow_mut();
            for (it, name) in st.items.iter_mut().zip(pending.iter()) {
                it.name.clone_from(name);
            }
            if clear_rules {
                st.rules = vec![Rule::Replace { find: String::new(), replace: String::new(), case_sensitive: false, include_ext: false }];
            }
            st.note.clone_from(&note);
        });
        if clear_rules {
            rebuild_rule_controls();
        }
    } else {
        reconcile(&pending);
        let msg = if finished.as_ref().is_some_and(|f| f.cancel) {
            tr("Cancelled. Finished steps were put back.", "취소했습니다. 끝난 단계는 되돌렸습니다.", "キャンセルしました。済んだ手順は戻しました。").to_string()
        } else {
            let err = finished.map(|f| f.err).unwrap_or_default();
            format!("{}{err}", tr("Rename failed: ", "이름을 바꾸지 못했습니다: ", "名前を変更できませんでした: "))
        };
        ST.with(|s| s.borrow_mut().note.clone_from(&msg));
        let cancelled = msg.starts_with(tr("Cancelled", "취소", "キャンセル"));
        if !cancelled {
            let _ = MessageBoxW(Some(h), &HSTRING::from(&msg), w!("Panegrid"), MB_ICONWARNING);
        }
    }
    let items = ST.with(|s| s.borrow().items.clone());
    let existing = load_existing(&items);
    ST.with(|s| s.borrow_mut().existing = existing);
    refresh_views();
    layout(h);
    recompute_keep_note();
    let _ = InvalidateRect(Some(h), None, true);
}

unsafe fn reconcile(pending: &[String]) {
    let items = ST.with(|s| s.borrow().items.clone());
    let mut names = Vec::new();
    for (it, new) in items.iter().zip(pending.iter().chain(std::iter::repeat(&String::new()))) {
        let found = actual_name(&it.dir, &it.name).or_else(|| actual_name(&it.dir, new));
        names.push(found.unwrap_or_else(|| it.name.clone()));
    }
    ST.with(|s| {
        for (it, name) in s.borrow_mut().items.iter_mut().zip(names) {
            it.name = name;
        }
    });
}

fn actual_name(dir: &str, name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let path = if dir.ends_with(':') { format!("{dir}\\") } else { dir.to_string() };
    let want = engine::name_key(name);
    std::fs::read_dir(path).ok()?.flatten().find_map(|ent| {
        let n = ent.file_name().to_string_lossy().into_owned();
        (engine::name_key(&n) == want).then_some(n)
    })
}

unsafe fn refresh_views() {
    let n = crate::app(|a| a.s.panes());
    for i in 0..n {
        if let Some(sv) = crate::view_of::<IShellView>(i) {
            let _ = sv.Refresh();
        }
    }
}

fn tz_offset_secs() -> i32 {
    unsafe {
        let mut info = TIME_ZONE_INFORMATION::default();
        let id = GetTimeZoneInformation(&mut info);
        if id == u32::MAX {
            return 0;
        }
        // Bias is the minutes added to local time to reach UTC. DaylightBias applies during DST.
        let extra = if id == 2 { info.DaylightBias } else { info.StandardBias };
        info.Bias.saturating_add(extra).saturating_mul(-60)
    }
}

fn recompute() {
    ST.with(|s| s.borrow_mut().note.clear());
    recompute_keep_note();
}

fn recompute_keep_note() {
    let (items, rules, existing) = ST.with(|s| {
        let st = s.borrow();
        (st.items.clone(), st.rules.clone(), st.existing.clone())
    });
    let rows = engine::preview(&items, &rules, &existing, tz_offset_secs());
    let max = preview_max(rows.len());
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.rows = rows;
        if st.preview_scroll > max {
            st.preview_scroll = max;
        }
    });
    unsafe { sync_buttons() };
    unsafe {
        let h = hwnd();
        if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
            let _ = InvalidateRect(Some(h), None, false);
            let host = ST.with(|s| s.borrow().host);
            if !host.0.is_null() {
                let _ = InvalidateRect(Some(host), None, false);
            }
        }
    }
}

fn preview_max(n: usize) -> i32 {
    let dpi = ST.with(|s| s.borrow().dpi);
    let g = geo_now();
    let head = sc(dpi, 28);
    let row_h = sc(dpi, 36).max(1);
    let visible = ((g.list.bottom - g.list.top - head) / row_h).max(1);
    (n as i32 - visible).max(0)
}

unsafe fn sync_buttons() {
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    let (errs, changes, busy) = ST.with(|s| {
        let st = s.borrow();
        (engine::error_count(&st.rows), engine::change_count(&st.rows), st.busy)
    });
    let undo = exec::last_undo().is_some();
    let _ = EnableWindow(ctl(h, ID_APPLY), (errs == 0 && changes > 0 && !busy).into());
    let _ = EnableWindow(ctl(h, ID_UNDO), (undo && !busy).into());
    let _ = InvalidateRect(Some(ctl(h, ID_APPLY)), None, false);
    let _ = InvalidateRect(Some(ctl(h, ID_UNDO)), None, false);
}

fn host_height() -> i32 {
    let h = hwnd();
    if h.0.is_null() {
        return 0;
    }
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    geo(rc, ST.with(|s| s.borrow().dpi)).host.bottom - geo(rc, ST.with(|s| s.borrow().dpi)).host.top
}

struct Geo {
    rules_lbl: RECT,
    add: RECT,
    host: RECT,
    prev_lbl: RECT,
    summary: RECT,
    list: RECT,
    undo: RECT,
    close: RECT,
    apply: RECT,
}

fn geo_now() -> Geo {
    let h = hwnd();
    let mut rc = RECT::default();
    if !h.0.is_null() {
        unsafe { let _ = GetClientRect(h, &mut rc); }
    }
    geo(rc, ST.with(|s| s.borrow().dpi))
}

fn geo(rc: RECT, dpi: i32) -> Geo {
    let dpi = dpi.max(96);
    let pad = sc(dpi, 16);
    let top = pad;
    let lbl_h = sc(dpi, 28);
    let add_w = sc(dpi, 120);
    let rules_lbl = RECT { left: rc.left + pad, top, right: rc.right - pad - add_w - sc(dpi, 8), bottom: top + lbl_h };
    let add = RECT { left: rc.right - pad - add_w, top: top + sc(dpi, 0), right: rc.right - pad, bottom: top + lbl_h };
    let foot_h = sc(dpi, 56);
    let foot_top = rc.bottom - foot_h;
    let host_top = rules_lbl.bottom + sc(dpi, 8);
    let body = (foot_top - host_top - sc(dpi, 36)).max(sc(dpi, 220));
    let host_h = (body * 42 / 100).max(sc(dpi, 110));
    let host = RECT { left: rc.left + pad, top: host_top, right: rc.right - pad, bottom: host_top + host_h };
    let prev_top = host.bottom + sc(dpi, 12);
    let prev_lbl = RECT { left: rc.left + pad, top: prev_top, right: rc.left + pad + sc(dpi, 200), bottom: prev_top + lbl_h };
    let summary = RECT { left: prev_lbl.right, top: prev_top, right: rc.right - pad, bottom: prev_top + lbl_h };
    let list = RECT { left: rc.left + pad, top: prev_lbl.bottom + sc(dpi, 4), right: rc.right - pad, bottom: foot_top - sc(dpi, 8) };
    let btn_h = sc(dpi, 32);
    let btn_y = foot_top + (foot_h - btn_h) / 2;
    let apply = RECT { left: rc.right - pad - sc(dpi, 128), top: btn_y, right: rc.right - pad, bottom: btn_y + btn_h };
    let close = RECT { left: apply.left - sc(dpi, 8) - sc(dpi, 96), top: btn_y, right: apply.left - sc(dpi, 8), bottom: btn_y + btn_h };
    let undo = RECT { left: rc.left + pad, top: btn_y, right: rc.left + pad + sc(dpi, 112), bottom: btn_y + btn_h };
    Geo { rules_lbl, add, host, prev_lbl, summary, list, undo, close, apply }
}

unsafe fn layout(h: HWND) {
    if h.0.is_null() {
        return;
    }
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let dpi = ST.with(|s| s.borrow().dpi);
    let g = geo(rc, dpi);
    move_to(ctl(h, ID_ADD), g.add);
    move_to(ctl(h, ID_UNDO), g.undo);
    move_to(ctl(h, ID_CLOSE), g.close);
    move_to(ctl(h, ID_APPLY), g.apply);
    let host = ST.with(|s| s.borrow().host);
    move_to(host, g.host);
    place_rules();
}

fn card_dip(rule: &Rule) -> i32 {
    match rule {
        Rule::Number { .. } | Rule::Date { .. } => 128,
        _ => 92,
    }
}

fn rule_stride(rule: &Rule, dpi: i32) -> i32 {
    sc(dpi.max(96), card_dip(rule))
}

fn rules_span(rules: &[Rule], dpi: i32) -> i32 {
    rules.iter().map(|r| rule_stride(r, dpi)).sum()
}

unsafe fn place_rules() {
    let host = ST.with(|s| s.borrow().host);
    if host.0.is_null() {
        return;
    }
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let mut rc = RECT::default();
    let _ = GetClientRect(host, &mut rc);
    let width = (rc.right - rc.left).max(1);
    let rules = ST.with(|s| s.borrow().rules.clone());
    let content = rules_span(&rules, dpi);
    let view = (rc.bottom - rc.top).max(0);
    let max = (content - view).max(0);
    ST.with(|s| {
        let mut st = s.borrow_mut();
        if st.rule_scroll > max {
            st.rule_scroll = max;
        }
        if st.rule_scroll < 0 {
            st.rule_scroll = 0;
        }
    });
    let scroll = ST.with(|s| s.borrow().rule_scroll);
    let mut y = -scroll;
    for (i, rule) in rules.iter().enumerate() {
        let stride = rule_stride(rule, dpi);
        let card = RECT { left: sc(dpi, 2), top: y + sc(dpi, 4), right: width - sc(dpi, 2), bottom: y + stride - sc(dpi, 4) };
        place_card(host, i, card, dpi, rule);
        y += stride;
    }
    for i in rules.len()..MAX_RULES {
        for f in 0..STRIDE {
            let c = ctl(host, id(i, f));
            if !c.0.is_null() {
                let _ = ShowWindow(c, SW_HIDE);
            }
        }
    }
}

unsafe fn place_card(host: HWND, i: usize, card: RECT, dpi: i32, rule: &Rule) {
    let row1 = card.top + sc(dpi, 10);
    let row2 = card.top + sc(dpi, 46);
    let row3 = card.top + sc(dpi, 82);
    let h = sc(dpi, 26);
    let btn = sc(dpi, 28);
    let gap = sc(dpi, 6);
    let right = card.right - sc(dpi, 10);
    let del = RECT { left: right - btn, top: row1, right, bottom: row1 + h };
    let down = RECT { left: del.left - gap - btn, top: row1, right: del.left - gap, bottom: row1 + h };
    let up = RECT { left: down.left - gap - btn, top: row1, right: down.left - gap, bottom: row1 + h };
    let kind = RECT { left: card.left + sc(dpi, 10), top: row1, right: (card.left + sc(dpi, 180)).min(up.left - gap), bottom: row1 + h };
    put(host, id(i, F_KIND), kind, true);
    put(host, id(i, F_UP), up, true);
    put(host, id(i, F_DOWN), down, true);
    put(host, id(i, F_DEL), del, true);
    let _ = EnableWindow(ctl(host, id(i, F_UP)), (i > 0).into());
    let last = i + 1 == ST.with(|s| s.borrow().rules.len());
    let _ = EnableWindow(ctl(host, id(i, F_DOWN)), (!last).into());
    for f in [F_A, F_B, F_C, F_AT, F_CASE, F_EXT, F_MODE, F_POS, F_SRC] {
        put(host, id(i, f), RECT::default(), false);
    }

    let x = card.left + sc(dpi, 10);
    let end = card.right - sc(dpi, 10);
    let edit_at = |field, left, top, right| {
        unsafe { put(host, id(i, field), RECT { left, top, right, bottom: top + h }, true); }
    };
    match rule {
        Rule::Replace { .. } => {
            let case_w = sc(dpi, 168);
            let ext_w = sc(dpi, 176);
            let mut edge = end;
            edit_at(F_EXT, edge - ext_w, row2, edge);
            edge -= ext_w + gap;
            edit_at(F_CASE, edge - case_w, row2, edge);
            edge -= case_w + gap;
            let span = (edge - x - gap).max(sc(dpi, 40));
            let w = span / 2;
            edit_at(F_A, x, row2, x + w);
            edit_at(F_B, x + w + gap, row2, x + w + gap + w);
        }
        Rule::Prefix { .. } | Rule::Suffix { .. } | Rule::Extension { .. } => {
            edit_at(F_A, x, row2, (x + sc(dpi, 280)).min(end));
        }
        Rule::Case { .. } => {
            let ext_w = sc(dpi, 176);
            edit_at(F_EXT, end - ext_w, row2, end);
            let mode_r = (end - ext_w - gap).max(x + sc(dpi, 40));
            edit_at(F_MODE, x, row2, mode_r.min(x + sc(dpi, 220)));
        }
        Rule::Number { pos, .. } => {
            let span = (end - x - gap * 2).max(sc(dpi, 90));
            let w = span / 3;
            edit_at(F_A, x, row2, x + w);
            edit_at(F_B, x + w + gap, row2, x + w + gap + w);
            edit_at(F_C, x + (w + gap) * 2, row2, (x + (w + gap) * 2 + w).min(end));
            let pos_w = sc(dpi, 180);
            edit_at(F_POS, x, row3, (x + pos_w).min(end));
            if matches!(pos, NumPos::At(_)) {
                let at_l = x + pos_w + gap;
                if at_l < end {
                    edit_at(F_AT, at_l, row3, (at_l + sc(dpi, 220)).min(end));
                }
            }
        }
        Rule::Date { .. } => {
            let src_w = sc(dpi, 180);
            edit_at(F_SRC, x, row2, (x + src_w).min(end));
            let pat_l = x + src_w + gap;
            if pat_l < end {
                edit_at(F_A, pat_l, row2, end);
            }
            edit_at(F_POS, x, row3, (x + sc(dpi, 200)).min(end));
        }
        Rule::Insert { .. } => {
            let at_w = sc(dpi, 200);
            edit_at(F_A, x, row2, (end - at_w - gap).max(x + sc(dpi, 40)));
            edit_at(F_AT, end - at_w, row2, end);
        }
        Rule::Delete { .. } => {
            let w = sc(dpi, 180);
            edit_at(F_AT, x, row2, x + w);
            edit_at(F_C, x + w + gap, row2, (x + w + gap + w).min(end));
        }
        Rule::Trim { .. } => {
            let edge_w = sc(dpi, 160);
            edit_at(F_A, x, row2, (end - edge_w - gap).max(x + sc(dpi, 40)));
            edit_at(F_POS, end - edge_w, row2, end);
        }
    }
}

unsafe fn put(parent: HWND, id: i32, r: RECT, show: bool) {
    let c = ctl(parent, id);
    if c.0.is_null() {
        return;
    }
    if !show || r.right <= r.left || r.bottom <= r.top {
        let _ = ShowWindow(c, SW_HIDE);
        return;
    }
    move_to(c, r);
    let _ = ShowWindow(c, SW_SHOW);
}

unsafe fn move_to(h: HWND, r: RECT) {
    if h.0.is_null() {
        return;
    }
    let _ = MoveWindow(h, r.left, r.top, (r.right - r.left).max(0), (r.bottom - r.top).max(0), true);
}

unsafe fn on_wheel(delta: i32) {
    if delta == 0 {
        return;
    }
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    let h = hwnd();
    if h.0.is_null() {
        return;
    }
    let _ = ScreenToClient(h, &mut pt);
    let g = geo_now();
    let over_rules = pt.y >= g.host.top && pt.y < g.host.bottom && pt.x >= g.host.left && pt.x < g.host.right;
    if over_rules {
        let dpi = ST.with(|s| s.borrow().dpi);
        let step = sc(dpi, 48);
        let next = ST.with(|s| s.borrow().rule_scroll) - delta * step;
        ST.with(|s| s.borrow_mut().rule_scroll = next);
        place_rules();
        let host = ST.with(|s| s.borrow().host);
        let _ = InvalidateRect(Some(host), None, true);
    } else {
        let n = ST.with(|s| s.borrow().rows.len());
        let max = preview_max(n);
        let next = (ST.with(|s| s.borrow().preview_scroll) - delta * 3).clamp(0, max);
        ST.with(|s| s.borrow_mut().preview_scroll = next);
        let _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn paint(h: HWND) {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = ST.with(|s| s.borrow().font);
    let small = ST.with(|s| s.borrow().small);
    crate::buffered(h, |dc, rc| {
        let g = geo(rc, dpi);
        crate::fill(dc, rc, crate::pal().bg);
        crate::text(dc, g.rules_lbl, tx(s("Rules", "규칙", "ルール")), font, crate::pal().text, DT_LEFT);
        let hint = tx(s("Top to bottom", "위에서 아래 순서로", "上から順に"));
        crate::text(dc, RECT { left: g.rules_lbl.left + sc(dpi, 70), ..g.rules_lbl }, hint, small, crate::pal().sub, DT_LEFT);
        crate::text(dc, g.prev_lbl, tx(s("Preview", "미리보기", "プレビュー")), font, crate::pal().text, DT_LEFT);
        let note = ST.with(|s| s.borrow().note.clone());
        let rows = ST.with(|s| s.borrow().rows.clone());
        let summary = if note.is_empty() {
            summary_text(rows.len(), engine::change_count(&rows), engine::error_count(&rows))
        } else {
            note
        };
        let sum_color = if engine::error_count(&rows) > 0 && ST.with(|s| s.borrow().note.is_empty()) { err_color() } else { crate::pal().sub };
        crate::text(dc, g.summary, &summary, small, sum_color, DT_RIGHT);
        paint_list(dc, g.list, dpi, font, small, &rows);
        crate::fill(dc, RECT { left: rc.left, top: g.undo.top - sc(dpi, 12), right: rc.right, bottom: g.undo.top - sc(dpi, 11), ..rc }, crate::pal().line);
    });
}

unsafe fn paint_list(dc: HDC, list: RECT, dpi: i32, font: HFONT, small: HFONT, rows: &[PreviewRow]) {
    if list.bottom <= list.top {
        return;
    }
    rfill(dc, list, sc(dpi, 8), crate::pal().card, Some(crate::pal().line));
    let head = sc(dpi, 28);
    let split = list.left + (list.right - list.left) * 42 / 100;
    let new_left = split + sc(dpi, 28);
    let err_w = sc(dpi, 168);
    let head_r = RECT { bottom: list.top + head, ..list };
    crate::text(dc, RECT { left: list.left + sc(dpi, 12), right: split, ..head_r }, tx(s("Current", "현재 이름", "現在の名前")), small, crate::pal().sub, DT_LEFT);
    crate::text(dc, RECT { left: new_left, right: list.right - err_w, ..head_r }, tx(s("New name", "새 이름", "新しい名前")), small, crate::pal().sub, DT_LEFT);
    crate::fill(dc, RECT { top: head_r.bottom - 1, bottom: head_r.bottom, ..list }, crate::pal().line);
    let _ = SaveDC(dc);
    let _ = IntersectClipRect(dc, list.left + 1, head_r.bottom, list.right - 1, list.bottom - 1);
    let row_h = sc(dpi, 36).max(1);
    let scroll = ST.with(|s| s.borrow().preview_scroll);
    let multi = ST.with(|s| {
        let st = s.borrow();
        let first = st.items.first().map(|it| engine::dir_key(&it.dir));
        first.is_some() && st.items.iter().any(|it| Some(engine::dir_key(&it.dir)) != first)
    });
    let items = ST.with(|s| s.borrow().items.clone());
    for (i, row) in rows.iter().enumerate().skip(scroll.max(0) as usize) {
        let y = head_r.bottom + (i as i32 - scroll) * row_h;
        if y > list.bottom {
            break;
        }
        let r = RECT { left: list.left, top: y, right: list.right, bottom: y + row_h };
        if i > 0 {
            crate::fill(dc, RECT { bottom: r.top + 1, top: r.top, ..r }, crate::pal().line);
        }
        let old_r = RECT { left: r.left + sc(dpi, 12), right: split - sc(dpi, 8), ..r };
        let arrow = RECT { left: split, right: new_left, ..r };
        let new_r = RECT { left: new_left, right: r.right - err_w, ..r };
        let err_r = RECT { left: r.right - err_w, right: r.right - sc(dpi, 10), ..r };
        let accent = chrome();
        let wash = crate::blend(crate::pal().card, accent, 0.28);
        if row.issues.is_empty() {
            paint_marked(dc, old_r, &row.old_name, row.old_span, font, crate::pal().text, wash, accent);
            paint_marked(dc, new_r, &row.new_name, row.new_span, font, crate::pal().text, wash, accent);
        } else {
            crate::text(dc, old_r, &row.old_name, font, crate::pal().text, DT_LEFT);
            crate::text(dc, new_r, &row.new_name, font, err_color(), DT_LEFT);
            if let Some(issue) = row.issues.first() {
                crate::text(dc, err_r, issue_text(*issue), small, err_color(), DT_RIGHT);
            }
        }
        crate::text(dc, arrow, "→", font, crate::pal().sub, DT_CENTER);
        if multi {
            if let Some(folder) = items.get(i).map(|it| it.dir.rsplit(['\\', '/']).next().unwrap_or("").to_string()) {
                if !folder.is_empty() && row.issues.is_empty() {
                    // Folder tag stays out of the highlighted name; the error column is free.
                    crate::text(dc, err_r, &folder, small, crate::pal().sub, DT_RIGHT);
                }
            }
        }
    }
    let _ = RestoreDC(dc, -1);
    let max = preview_max(rows.len());
    if max > 0 {
        paint_thumb(dc, list, ST.with(|s| s.borrow().preview_scroll), max, dpi);
    }
}

unsafe fn paint_host(h: HWND) {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = ST.with(|s| s.borrow().small);
    crate::buffered(h, |dc, rc| {
        crate::fill(dc, rc, crate::pal().bg);
        let rules = ST.with(|s| s.borrow().rules.clone());
        if rules.is_empty() {
            let r = RECT { left: sc(dpi, 8), top: sc(dpi, 12), right: rc.right - sc(dpi, 8), bottom: sc(dpi, 48) };
            crate::text(dc, r, tx(s("Add a rule. Names stay as they are until you do.", "규칙을 추가하세요. 그 전까지 이름은 그대로입니다.", "ルールを追加してください。それまでは名前はそのままです。")), font, crate::pal().sub, DT_LEFT);
            return;
        }
        let scroll = ST.with(|s| s.borrow().rule_scroll);
        let width = rc.right - rc.left;
        let mut y = -scroll;
        for (i, rule) in rules.iter().enumerate() {
            let stride = rule_stride(rule, dpi);
            let card = RECT { left: sc(dpi, 2), top: y + sc(dpi, 4), right: width - sc(dpi, 2), bottom: y + stride - sc(dpi, 4) };
            y += stride;
            if card.bottom < 0 || card.top > rc.bottom {
                continue;
            }
            rfill(dc, card, sc(dpi, 8), crate::pal().card, Some(crate::pal().line));
            // Frames behind the borderless edits, slightly outside the control.
            for f in [F_A, F_B, F_C, F_AT] {
                let c = ctl(h, id(i, f));
                if c.0.is_null() || !IsWindowVisible(c).as_bool() {
                    continue;
                }
                let mut wr = RECT::default();
                let _ = GetWindowRect(c, &mut wr);
                let mut pts = [POINT { x: wr.left, y: wr.top }, POINT { x: wr.right, y: wr.bottom }];
                let _ = MapWindowPoints(None, Some(h), &mut pts);
                wr = RECT { left: pts[0].x, top: pts[0].y, right: pts[1].x, bottom: pts[1].y };
                let inf = sc(dpi, 3);
                rfill(dc, RECT { left: wr.left - inf, top: wr.top - inf, right: wr.right + inf, bottom: wr.bottom + inf }, sc(dpi, 4), crate::pal().card, Some(crate::pal().line));
            }
        }
        let content = rules_span(&rules, dpi);
        let view = rc.bottom - rc.top;
        let max = (content - view).max(0);
        if max > 0 {
            paint_thumb(dc, rc, scroll, max, dpi);
        }
    });
}

unsafe fn paint_thumb(dc: HDC, r: RECT, scroll: i32, max: i32, dpi: i32) {
    let track = RECT { left: r.right - sc(dpi, 8), top: r.top + sc(dpi, 6), right: r.right - sc(dpi, 4), bottom: r.bottom - sc(dpi, 6) };
    rfill(dc, track, sc(dpi, 2), crate::blend(crate::pal().bg, crate::pal().text, 0.08), None);
    let span = (track.bottom - track.top).max(1);
    let thumb_h = (span * span / (span + max)).clamp(sc(dpi, 18), span);
    let travel = (span - thumb_h).max(1);
    let y = track.top + scroll.clamp(0, max) * travel / max.max(1);
    rfill(dc, RECT { left: track.left, top: y, right: track.right, bottom: y + thumb_h }, sc(dpi, 2), crate::blend(crate::pal().bg, crate::pal().text, 0.35), None);
}

unsafe fn paint_marked(dc: HDC, rect: RECT, text: &str, span: Option<(usize, usize)>, font: HFONT, normal: u32, wash: u32, mark: u32) {
    let Some((a, b)) = span else {
        crate::text(dc, rect, text, font, normal, DT_LEFT);
        return;
    };
    let chars: Vec<char> = text.chars().collect();
    let pre: String = chars.get(..a).unwrap_or(&[]).iter().collect();
    let mid: String = chars.get(a..b).unwrap_or(&[]).iter().collect();
    let post: String = chars.get(b..).unwrap_or(&[]).iter().collect();
    let total = text_w(dc, font, text);
    if total > (rect.right - rect.left).max(0) {
        crate::text(dc, rect, text, font, mark, DT_LEFT);
        return;
    }
    let mut x = rect.left;
    let draw = |dc: HDC, x: i32, s: &str, fg: u32, bg: Option<u32>| -> i32 {
        if s.is_empty() {
            return x;
        }
        let w = text_w(dc, font, s).max(0);
        let r = RECT { left: x, top: rect.top, right: x + w, bottom: rect.bottom };
        if let Some(bg) = bg {
            let pad = RECT { top: r.top + 6, bottom: r.bottom - 6, ..r };
            if pad.bottom > pad.top {
                crate::fill(dc, pad, bg);
            }
        }
        crate::text(dc, r, s, font, fg, DT_LEFT);
        x + w
    };
    x = draw(dc, x, &pre, normal, None);
    x = draw(dc, x, &mid, mark, Some(wash));
    let _ = draw(dc, x, &post, normal, None);
}

unsafe fn text_w(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() || font.is_invalid() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    let _ = DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right - r.left
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    let dis = d.itemState.0 & 4 != 0;
    let pressed = d.itemState.0 & 1 != 0;
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = if id >= ID_BASE { ST.with(|s| s.borrow().small) } else { ST.with(|s| s.borrow().font) };
    let accent = chrome();
    let in_card = id >= ID_BASE;
    let ground = if in_card { crate::pal().card } else { crate::pal().bg };
    crate::fill(d.hDC, d.rcItem, ground);
    let accent_btn = id == ID_APPLY && !dis;
    let (fill, border, ink) = if accent_btn {
        (accent, accent, crate::on_color(accent))
    } else if dis {
        (crate::blend(ground, crate::pal().sub, 0.12), crate::pal().line, crate::pal().sub)
    } else if pressed {
        (crate::blend(ground, crate::pal().text, 0.08), crate::pal().line, crate::pal().text)
    } else {
        (ground, crate::pal().line, crate::pal().text)
    };
    rfill(d.hDC, d.rcItem, sc(dpi, 6), fill, Some(border));
    let label = crate::text_of(d.hwndItem);
    crate::text(d.hDC, d.rcItem, &label, font, ink, DT_CENTER);
}

unsafe fn rfill(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let brush = CreateSolidBrush(crate::cref(fill));
    let pen = match border {
        Some(c) => CreatePen(PS_SOLID, 1, crate::cref(c)),
        None => HPEN(GetStockObject(NULL_PEN).0),
    };
    let op = SelectObject(dc, pen.into());
    let ob = SelectObject(dc, brush.into());
    let _ = RoundRect(dc, r.left, r.top, r.right, r.bottom, rad.max(1), rad.max(1));
    SelectObject(dc, op);
    SelectObject(dc, ob);
    let _ = DeleteObject(brush.into());
    if border.is_some() {
        let _ = DeleteObject(pen.into());
    }
}

fn chrome() -> u32 {
    let (c, dark) = crate::app(|a| (a.s.accent_now(), a.s.dark));
    let plain = if dark { crate::DARK.accents[1].1 } else { crate::LIGHT.accents[1].1 };
    if c == plain {
        if dark { 0x4CC2FF } else { 0x0067C0 }
    } else {
        c
    }
}

fn err_color() -> u32 {
    if crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed) { 0xFF7B72 } else { 0xC4314B }
}

fn summary_text(n: usize, changes: usize, errors: usize) -> String {
    match (crate::ui(), errors) {
        (1, 0) => format!("{n}개 · 변경 {changes}"),
        (1, e) => format!("{n}개 · 변경 {changes} · 오류 {e}"),
        (2, 0) => format!("{n} 件 · 変更 {changes}"),
        (2, e) => format!("{n} 件 · 変更 {changes} · エラー {e}"),
        (_, 0) => format!("{n} items · {changes} changes"),
        (_, e) => format!("{n} items · {changes} changes · {e} errors"),
    }
}

fn renamed_note(n: usize) -> String {
    match crate::ui() {
        1 => format!("{n}개 항목의 이름을 바꿨습니다"),
        2 => format!("{n} 件の名前を変更しました"),
        _ => format!("Renamed {n} items"),
    }
}

fn undone_note(n: usize) -> String {
    match crate::ui() {
        1 => format!("{n}개 항목을 되돌렸습니다"),
        2 => format!("{n} 件を元に戻しました"),
        _ => format!("Undid {n} items"),
    }
}

fn issue_text(issue: Issue) -> &'static str {
    match issue {
        Issue::Empty => tr("Empty name", "이름이 비어 있습니다", "名前が空です"),
        Issue::InvalidChars => tr("Invalid character", "사용할 수 없는 문자", "使えない文字"),
        Issue::Reserved => tr("Reserved name", "예약된 이름", "予約された名前"),
        Issue::Trailing => tr("Ends with space or dot", "공백이나 점으로 끝남", "末尾が空白かドット"),
        Issue::TooLong => tr("Path too long", "경로가 너무 깁니다", "パスが長すぎます"),
        Issue::Duplicate => tr("Duplicate name", "이름이 겹칩니다", "名前が重複"),
        Issue::Exists => tr("Already exists", "이미 있는 이름", "既に存在します"),
    }
}

unsafe fn collect_pane(i: usize) -> Vec<BatchItem> {
    let Some(fv) = crate::view_of::<IFolderView2>(i) else { return Vec::new() };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0).max(0);
    if n > 50_000 {
        return collect_selection(&fv);
    }
    let Ok(sf) = fv.GetFolder::<IShellFolder>() else { return collect_selection(&fv) };
    let mut out = Vec::new();
    for k in 0..n {
        if out.len() >= 10_000 {
            break;
        }
        let Ok(pidl) = fv.Item(k) else { continue };
        let selected = fv.GetSelectionState(pidl).unwrap_or(0) & SVSI_SELECT.0 as u32 != 0;
        if !selected {
            CoTaskMemFree(Some(pidl as _));
            continue;
        }
        let mut attr = SFGAO_FOLDER.0 | SFGAO_STREAM.0;
        let pidl_ref = pidl as *const ITEMIDLIST;
        let _ = sf.GetAttributesOf(&[pidl_ref], &mut attr);
        let is_dir = crate::real_folder(attr);
        let mut sr = STRRET::default();
        let mut name = PWSTR::null();
        let path = if sf.GetDisplayNameOf(pidl_ref, SHGDNF(SHGDN_FORPARSING.0), &mut sr).is_ok() && StrRetToStrW(&mut sr, Some(pidl_ref), &mut name).is_ok() {
            let p = crate::pwstr(name); // pwstr frees the buffer
            p
        } else {
            String::new()
        };
        CoTaskMemFree(Some(pidl as _));
        if let Some(it) = item_from_path(&path, is_dir) {
            out.push(it);
        }
    }
    out
}

unsafe fn collect_selection(fv: &IFolderView2) -> Vec<BatchItem> {
    let Ok(arr) = fv.GetSelection(false) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(10_000);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { continue };
        let attr = it.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).map(|f| f.0).unwrap_or(0);
        let is_dir = crate::real_folder(attr);
        let Ok(p) = it.GetDisplayName(SIGDN_FILESYSPATH) else { continue };
        let path = crate::pwstr(p);
        if let Some(item) = item_from_path(&path, is_dir) {
            out.push(item);
        }
    }
    out
}

fn item_from_path(path: &str, is_dir_hint: bool) -> Option<BatchItem> {
    if !crate::is_fs(path) {
        return None;
    }
    let path = path.trim_end_matches(['\\', '/']);
    let (dir, name) = match path.rsplit_once('\\').or_else(|| path.rsplit_once('/')) {
        Some((dir, name)) if !name.is_empty() => (dir.to_string(), name.to_string()),
        _ => return None,
    };
    let full = engine::join_path(&dir, &name);
    let is_dir = std::fs::metadata(&full).map(|m| m.is_dir()).unwrap_or(is_dir_hint);
    let (modified, created, accessed) = file_times(&full);
    let mut item = BatchItem::new(dir, name, is_dir);
    item.modified = modified;
    item.created = created;
    item.accessed = accessed;
    Some(item)
}

fn file_times(path: &str) -> (Option<i64>, Option<i64>, Option<i64>) {
    let Ok(meta) = std::fs::metadata(path) else { return (None, None, None) };
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64)
    };
    (secs(meta.modified()), secs(meta.created()), secs(meta.accessed()))
}

fn load_existing(items: &[BatchItem]) -> HashSet<(String, String)> {
    let mut dirs: Vec<String> = Vec::new();
    for it in items {
        if !dirs.iter().any(|d| engine::dir_key(d) == engine::dir_key(&it.dir)) {
            dirs.push(it.dir.clone());
        }
    }
    let mut set = HashSet::new();
    for d in &dirs {
        let path = if d.ends_with(':') { format!("{d}\\") } else { d.clone() };
        if let Ok(rd) = std::fs::read_dir(&path) {
            for ent in rd.flatten() {
                let name = ent.file_name().to_string_lossy().into_owned();
                set.insert((engine::dir_key(d), engine::name_key(&name)));
            }
        }
    }
    for it in items {
        set.insert((engine::dir_key(&it.dir), engine::name_key(&it.name)));
    }
    set
}

unsafe fn style_window(h: HWND) {
    let dark = crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed);
    let on = BOOL::from(dark);
    let _ = DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let caption = crate::cref(crate::pal().bg);
    let _ = DwmSetWindowAttribute(h, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
}

fn make_font(face: &str, px: i32, weight: i32, dpi: i32) -> HFONT {
    let mut lf = LOGFONTW {
        lfHeight: -(px * dpi / 96).max(1),
        lfWeight: weight,
        lfCharSet: DEFAULT_CHARSET,
        lfQuality: CLEARTYPE_QUALITY,
        ..Default::default()
    };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    unsafe { CreateFontIndirectW(&lf) }
}

fn rebuild_fonts() {
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let custom = crate::app(|a| a.s.font_face.clone());
    let face = if custom.is_empty() { crate::ui_face() } else { custom.as_str() };
    let new = (make_font(face, 14, 400, dpi), make_font(face, 12, 400, dpi));
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.small);
        (st.font, st.small) = new;
        old
    });
    unsafe { drop_font(old.0); drop_font(old.1); }
}

fn drop_fonts() {
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.small);
        st.font = HFONT::default();
        st.small = HFONT::default();
        old
    });
    unsafe {
        drop_font(old.0);
        drop_font(old.1);
    }
}

unsafe fn drop_font(f: HFONT) {
    if !f.is_invalid() {
        let _ = DeleteObject(f.into());
    }
}

unsafe fn apply_font(parent: HWND, font: HFONT) {
    let mut c = FindWindowExW(Some(parent), None, PCWSTR::null(), PCWSTR::null()).unwrap_or_default();
    while !c.0.is_null() {
        let _ = SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        theme_ctl(c);
        c = FindWindowExW(Some(parent), Some(c), PCWSTR::null(), PCWSTR::null()).unwrap_or_default();
    }
}

unsafe fn theme_ctl(h: HWND) {
    let dark = crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed);
    let name = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

unsafe fn theme_btn(h: HWND) {
    let dark = crate::DARK_ON.load(std::sync::atomic::Ordering::Relaxed);
    let name = if dark { w!("DarkMode_Explorer") } else { w!("Explorer") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

unsafe fn child(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    crate::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font)
}

fn id(i: usize, f: i32) -> i32 {
    ID_BASE + i as i32 * STRIDE + f
}

fn ctl(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

unsafe fn cue(h: HWND, text: &str) {
    if h.0.is_null() {
        return;
    }
    let hs = HSTRING::from(text);
    let _ = SendMessageW(h, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
}

unsafe fn set_btn(parent: HWND, id: i32, text: &str) {
    let c = ctl(parent, id);
    if !c.0.is_null() {
        let _ = SetWindowTextW(c, &HSTRING::from(text));
    }
}
