use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::Once;
use windows::core::{w, BOOL, HSTRING, PCWSTR};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWINDOWATTRIBUTE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VK_ESCAPE, VK_RETURN};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::*;
use crate::tf;

pub unsafe fn open(owner: HWND) {
    ensure_class();
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        crate::winmerge::forget_status();
        crate::folderapp::reload_shown();
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(h);
        redraw();
        return;
    }
    let dpi = GetDpiForWindow(owner) as i32;
    let (ww, hh) = (sc(dpi.max(96), 1000), sc(dpi.max(96), 640));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE;
    let _ = CreateWindowExW(WINDOW_EX_STYLE(0), w!("PanegridSettings"), w!("Panegrid"), style, x, y, ww, hh, Some(owner), None, None, None);
}

pub unsafe fn close() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = DestroyWindow(h);
    }
}

pub unsafe fn captures(h: HWND) -> bool {
    let w = hwnd();
    !w.0.is_null() && IsWindow(Some(w)).as_bool() && (h == w || IsChild(w, h).as_bool())
}

pub unsafe fn redraw() {
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = InvalidateRect(Some(h), None, false);
    }
}

pub unsafe fn refresh() {
    crate::folderapp::reload_shown();
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    style_window(h);
    rebuild_fonts();
    let font = ST.with(|s| s.borrow().font);
    each_child(h, |c| {
        SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
        theme_ctl(c);
    });
    fill_strings(h);
    let _ = SetWindowTextW(h, &HSTRING::from(tx(TITLE)));
    cue(h, S_SEARCH, tx(FIND));
    cue(h, S_KEYQ, tx(crate::Key::SearchActionsOrKeys));
    cue(h, S_WM, tx(crate::Key::AutoDetect));
    layout(h);
    let _ = InvalidateRect(Some(h), None, true);
}

/// The right-click page is open and a menu just learned new item names.
pub(crate) unsafe fn refresh_ctx_labels() {
    let h = hwnd();
    if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
        return;
    }
    if ST.with(|s| s.borrow().page) != CTX_PAGE {
        return;
    }
    layout(h);
    let _ = InvalidateRect(Some(h), None, false);
}

fn ensure_class() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let hinst: HINSTANCE = m.into();
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
            lpfnWndProc: Some(settings_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(),
            lpszClassName: w!("PanegridSettings"),
            ..Default::default()
        });
    });
}

unsafe fn style_window(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
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
    let custom = app(|a| a.s.font_face.clone());
    let fallback = crate::ui_face();
    let face = if custom.is_empty() { fallback } else { custom.as_str() };
    // A chosen face is the page title at weight 700. The language default keeps its own title face.
    let (title_face, title_w) = if custom.is_empty() {
        crate::title_style()
    } else {
        (face, 700)
    };
    let new = (
        make_font(face, 14, 400, dpi),
        make_font(face, 12, 400, dpi),
        make_font(face, 16, 700, dpi),
        make_font(title_face, 24, title_w, dpi),
        make_font("Consolas", 12, 400, dpi),
        make_font("Segoe MDL2 Assets", 14, 400, dpi),
    );
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.small, st.bold, st.title_font, st.mono, st.icon);
        (st.font, st.small, st.bold, st.title_font, st.mono, st.icon) = new;
        old
    });
    unsafe {
        for f in [old.0, old.1, old.2, old.3, old.4, old.5] {
            if !f.is_invalid() {
                let _ = DeleteObject(f.into());
            }
        }
    }
}

unsafe fn theme_ctl(h: HWND) {
    let dark = crate::DARK_ON.load(Ordering::Relaxed);
    let name = if dark { w!("DarkMode_CFD") } else { w!("CFD") };
    let _ = SetWindowTheme(h, name, PCWSTR::null());
}

fn child_at(parent: HWND, class: PCWSTR, text: &str, style: u32, id: i32, font: HFONT) -> HWND {
    let t = HSTRING::from(text);
    unsafe { crate::child(parent, class, PCWSTR(t.as_ptr()), style, WINDOW_EX_STYLE(0), id, font) }
}

unsafe fn build_children(h: HWND) {
    let font = ST.with(|s| s.borrow().font);
    let tab = 0x00010000u32;
    let dis = 0x08000000u32;
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_SEARCH, font);
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_KEYQ, font);
    let combo = |id, on| child_at(h, w!("COMBOBOX"), "", 3 | 0x200 | 0x00200000 | tab | if on { 0 } else { dis }, id, font);
    combo(S_LANG, true);
    combo(S_FACE, true);
    combo(S_THEME, true);
    for id in [S_START, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SIDE] {
        combo(id, true);
    }
    let btn = |id, label: &str, on| child_at(h, w!("BUTTON"), label, 0xB | tab | if on { 0 } else { dis }, id, font);
    btn(S_CLOSE, tx(CLOSE_L), true);
    btn(S_RESET, tx(crate::Key::ResetAllDefaults), true);
    btn(S_SESS, tx(crate::Key::OpenFolder), true);
    btn(S_ACCENT, tx(crate::Key::Custom2), true);
    btn(S_GITHUB, tx(crate::Key::OpenGithub), true);
    btn(S_ADV_OPEN, tx(crate::Key::OpenFolder), true);
    btn(S_ADV_EX, tx(crate::Key::Export), true);
    btn(S_ADV_IM, tx(crate::Key::Import), true);
    btn(S_ADV_RESET, tx(crate::Key::Reset), true);
    btn(S_EV, ev_button_label(), true);
    btn(S_FV_DEF, tx(crate::Key::SetFromThisPane), true);
    btn(S_FV_RESET, tx(crate::Key::Reset2), true);
    btn(S_FV_CLEAR, tx(crate::Key::ClearAll), true);
    btn(S_CTX_RESET, tx(CTX_RESET_L), true);
    btn(S_CTX_REFRESH, tx(CTX_REFRESH_L), true);
    child_at(h, w!("EDIT"), "", 0x80 | tab, S_WM, font);
    btn(S_WM_BROWSE, tx(crate::Key::Browse), true);
}

fn each_child(h: HWND, mut f: impl FnMut(HWND)) {
    let ids = [S_SEARCH, S_CLOSE, S_KEYQ, S_RESET, S_LANG, S_FACE, S_START, S_THEME, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SIDE, S_SESS, S_ACCENT, S_GITHUB, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET, S_EV, S_FV_DEF, S_FV_RESET, S_FV_CLEAR, S_CTX_RESET, S_CTX_REFRESH, S_WM, S_WM_BROWSE];
    for id in ids {
        let c = unsafe { GetDlgItem(Some(h), id).unwrap_or_default() };
        if !c.0.is_null() {
            f(c);
        }
    }
    let made = FAV_MADE.load(std::sync::atomic::Ordering::Relaxed);
    for base in [S_FAV0, S_REN0] {
        for i in 0..made {
            let c = unsafe { GetDlgItem(Some(h), base + i).unwrap_or_default() };
            if !c.0.is_null() {
                f(c);
            }
        }
    }
}

unsafe fn fill_strings(h: HWND) {
    let options = crate::settings_languages();
    let lang = app(|a| a.s.lang);
    let lang_i = options.iter().position(|(l, _)| *l == lang).unwrap_or(0);
    let labels: Vec<String> = options.iter().map(|(_, name)| name.clone()).collect();
    fill_combo(h, S_LANG, &labels, lang_i);
    fill_face(h);
    let theme = app(|a| a.s.theme) as usize;
    fill_combo(h, S_THEME, &[tx(crate::Key::FollowWindows).into(), tx(crate::Key::Light).into(), tx(crate::Key::Dark).into()], theme.min(2));
    let boot = app(|a| a.s.boot) as usize;
    fill_combo(h, S_START, &[tx(crate::Key::RestoreLastState).into(), tx(crate::Key::OpenSession3).into(), tx(crate::Key::DefaultLayout).into()], boot.min(2));
    let view_labels: Vec<String> = crate::EXPLORER_VIEWS.iter().map(|&(_, _, key)| tr(key).to_string()).collect();
    let view_i = app(|a| {
        let (mode, icon) = a.s.default_view();
        crate::explorer_view_index(mode, icon).unwrap_or(5)
    });
    fill_combo(h, S_VIEW, &view_labels, view_i);
    let start = app(|a| a.s.start_new) as usize;
    fill_combo(h, S_DIR, &[tx(crate::Key::ThisPc).into(), tx(crate::Key::Desktop).into(), tx(crate::Key::ChooseFolder).into()], start.min(2));
    let del = app(|a| a.s.del) as usize;
    fill_combo(h, S_DEL, &[tx(crate::Key::RecycleBin).into(), tx(crate::Key::DeletePermanently).into()], del.min(1));
    let custom = app(|a| a.s.new_name.clone());
    let new_label = if custom.is_empty() { tx(crate::Key::Custom2).to_string() } else { custom };
    let new_i = usize::from(!app(|a| a.s.new_name.is_empty()));
    fill_combo(h, S_NEW, &[tx(crate::Key::NewTextDocumentTxt).into(), new_label], new_i);
    let dropm = app(|a| a.s.dropm) as usize;
    fill_combo(h, S_DROP, &[tx(crate::Key::WindowsRulesSameDriveMove).into(), tx(crate::Key::AlwaysCopy).into(), tx(crate::Key::AlwaysAsk).into()], dropm.min(2));
    let layout_new = app(|a| a.s.layout_new) as usize;
    fill_combo(h, S_LAYOUT, &[tx(crate::Key::TwoPanesSideBySide).into(), tx(crate::Key::TwoPanesTopAndBottom).into(), tx(crate::Key::K22FourPanes).into(), tx(crate::Key::OnePane).into()], layout_new.min(3));
    let side_i = usize::from(!app(|a| a.s.fav_top));
    fill_combo(h, S_SIDE, &[tx(SIDE_FAV).into(), tx(SIDE_TREE).into()], side_i.min(1));
    let labels = [
        (S_CLOSE, tx(CLOSE_L)),
        (S_RESET, tx(crate::Key::ResetAllDefaults)),
        (S_SESS, tx(crate::Key::OpenFolder)),
        (S_ACCENT, tx(crate::Key::Custom2)),
        (S_GITHUB, tx(crate::Key::OpenGithub)),
        (S_ADV_OPEN, tx(crate::Key::OpenFolder)),
        (S_ADV_EX, tx(crate::Key::Export)),
        (S_ADV_IM, tx(crate::Key::Import)),
        (S_ADV_RESET, tx(crate::Key::Reset)),
        (S_EV, ev_button_label()),
        (S_FV_DEF, tx(crate::Key::SetFromThisPane)),
        (S_FV_RESET, tx(crate::Key::Reset2)),
        (S_FV_CLEAR, tx(crate::Key::ClearAll)),
        (S_CTX_RESET, tx(CTX_RESET_L)),
        (S_CTX_REFRESH, tx(CTX_REFRESH_L)),
        (S_WM_BROWSE, tx(crate::Key::Browse)),
    ];
    BUSY.with(|b| b.set(true));
    let _ = SetWindowTextW(ctl(h, S_WM), &HSTRING::from(app(|a| a.s.winmerge_path.clone())));
    BUSY.with(|b| b.set(false));
    for (id, label) in labels {
        let _ = SetWindowTextW(ctl(h, id), &HSTRING::from(label));
    }
}

fn ensure_faces() {
    if ST.with(|s| s.borrow().faces.is_some()) {
        return;
    }
    // The family list exists only while this window does. Startup never calls this.
    let list = unsafe { crate::installed_families() };
    ST.with(|s| s.borrow_mut().faces = Some(list));
}

unsafe fn fill_face(h: HWND) {
    ensure_faces();
    let def = format!("{} ({})", tx(crate::Key::Default), crate::ui_face());
    let stored = app(|a| a.s.font_face.clone());
    let mut items = vec![def];
    let mut sel = 0usize;
    ST.with(|s| {
        if let Some(faces) = s.borrow().faces.as_ref() {
            for (i, name) in faces.iter().enumerate() {
                if !stored.is_empty() && crate::same_face(name, &stored) {
                    sel = i + 1;
                }
                items.push(name.clone());
            }
        }
    });
    if sel == 0 && !stored.is_empty() {
        items.push(stored);
        sel = items.len() - 1;
    }
    fill_combo(h, S_FACE, &items, sel);
    let c = ctl(h, S_FACE);
    if !c.0.is_null() {
        let dpi = ST.with(|s| s.borrow().dpi.max(96));
        SendMessageW(c, CB_SETDROPPEDWIDTH, Some(WPARAM(sc(dpi, 320) as usize)), None);
    }
}

unsafe fn combo_lb_text(h: HWND, id: i32, sel: usize) -> String {
    let c = ctl(h, id);
    if c.0.is_null() {
        return String::new();
    }
    let n = SendMessageW(c, CB_GETLBTEXTLEN, Some(WPARAM(sel)), None).0;
    if n <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; n as usize + 1];
    let got = SendMessageW(c, CB_GETLBTEXT, Some(WPARAM(sel)), Some(LPARAM(buf.as_mut_ptr() as isize))).0;
    if got <= 0 {
        return String::new();
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(got as usize).min(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

unsafe fn fill_combo(h: HWND, id: i32, items: &[String], sel: usize) {
    let c = ctl(h, id);
    if c.0.is_null() {
        return;
    }
    BUSY.with(|b| b.set(true));
    SendMessageW(c, CB_RESETCONTENT, None, None);
    for item in items {
        let hs = HSTRING::from(item.as_str());
        SendMessageW(c, CB_ADDSTRING, None, Some(LPARAM(hs.as_ptr() as isize)));
    }
    SendMessageW(c, CB_SETCURSEL, Some(WPARAM(sel)), None);
    SendMessageW(c, CB_SETMINVISIBLE, Some(WPARAM(8)), None);
    BUSY.with(|b| b.set(false));
}

unsafe fn cue(h: HWND, id: i32, text: &str) {
    let c = ctl(h, id);
    let hs = HSTRING::from(text);
    SendMessageW(c, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(hs.as_ptr() as isize)));
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let m = sc(dpi, 8) as isize;
    SendMessageW(c, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16)))); // EM_SETMARGINS
}

fn ctl(parent: HWND, id: i32) -> HWND {
    unsafe { GetDlgItem(Some(parent), id).unwrap_or_default() }
}

fn layout(h: HWND) {
    let mut sn = snap();
    let mut rc = RECT::default();
    unsafe { let _ = GetClientRect(h, &mut rc); }
    let mut g = geo(rc, &sn);
    if sn.scroll > g.scroll_max {
        sn.scroll = g.scroll_max;
        ST.with(|s| s.borrow_mut().scroll = sn.scroll);
        g = geo(rc, &sn);
    }
    ST.with(|s| {
        let mut st = s.borrow_mut();
        st.scroll_max = g.scroll_max;
        st.track = g.track;
        st.thumb_h = g.thumb.bottom - g.thumb.top;
    });
    unsafe { place(h, &g, &sn); }
}

unsafe fn place(h: HWND, g: &Geo, sn: &Snap) {
    let hide = |id: i32| {
        let c = ctl(h, id);
        if !c.0.is_null() {
            let _ = ShowWindow(c, SW_HIDE);
        }
    };
    for id in [S_KEYQ, S_RESET, S_LANG, S_FACE, S_START, S_THEME, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SIDE, S_SESS, S_ACCENT, S_GITHUB, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET, S_EV, S_FV_DEF, S_FV_RESET, S_FV_CLEAR, S_CTX_RESET, S_CTX_REFRESH, S_WM, S_WM_BROWSE] {
        hide(id);
    }
    let n = fav_tree_rows().len().min(FAV_MAX as usize) as i32;
    let made = FAV_MADE.load(std::sync::atomic::Ordering::Relaxed);
    for base in [S_FAV0, S_REN0] {
        for i in 0..made {
            let c = ctl(h, base + i);
            if c.0.is_null() {
                continue;
            }
            if i >= n {
                let _ = DestroyWindow(c);
            } else {
                let _ = ShowWindow(c, SW_HIDE);
            }
        }
    }
    crate::place_edit(ctl(h, S_SEARCH), inset(g.search, sc(sn.dpi, FIELD_INSET_DIP)), true);
    move_to(ctl(h, S_CLOSE), g.close, true);
    let content = g.content;
    for item in &g.items {
        match item {
            Item::Card(c) => {
                if let Some(id) = ctl_id(c.spec.ctl) {
                    move_to(ctl(h, id), c.ctl, intersects(c.ctl, content));
                }
                if c.spec.ctl == Ctl::Adv {
                    let ids = [S_ADV_OPEN, S_ADV_EX, S_ADV_IM];
                    for (i, id) in ids.iter().enumerate() {
                        move_to(ctl(h, *id), c.adv[i], intersects(c.adv[i], content));
                    }
                }
                if matches!(c.spec.ctl, Ctl::AdvReset | Ctl::FolderDefault | Ctl::FolderReset | Ctl::FolderClear) {
                    let id = match c.spec.ctl {
                        Ctl::FolderDefault => S_FV_DEF,
                        Ctl::FolderReset => S_FV_RESET,
                        Ctl::FolderClear => S_FV_CLEAR,
                        _ => S_ADV_RESET,
                    };
                    move_to(ctl(h, id), c.adv[0], intersects(c.adv[0], content));
                }
                if c.spec.ctl == Ctl::EvAct {
                    let b = ctl(h, S_EV);
                    let _ = SetWindowTextW(b, &HSTRING::from(ev_button_label()));
                    let _ = EnableWindow(b, !crate::everything::busy());
                    move_to(b, c.ctl, crate::everything::action().is_some() && intersects(c.ctl, content));
                }
                if c.spec.ctl == Ctl::WmPath {
                    move_to(ctl(h, S_WM_BROWSE), c.adv[0], intersects(c.adv[0], content));
                    crate::place_edit(ctl(h, S_WM), inset(c.ctl, sc(sn.dpi, FIELD_INSET_DIP)), intersects(c.ctl, content));
                }
            }
            Item::CtxBar { reset, refresh } => {
                move_to(ctl(h, S_CTX_REFRESH), *refresh, intersects(*refresh, content));
                move_to(ctl(h, S_CTX_RESET), *reset, intersects(*reset, content));
            }
            Item::KeyBar { edit, reset } => {
                crate::place_edit(ctl(h, S_KEYQ), inset(*edit, sc(sn.dpi, FIELD_INSET_DIP)), intersects(*edit, content));
                move_to(ctl(h, S_RESET), *reset, intersects(*reset, content));
            }
            Item::FavRow { i, rename, btn, .. } if *i != usize::MAX && *i < FAV_MAX as usize && btn.right > btn.left => {
                let id = S_FAV0 + *i as i32;
                FAV_MADE.fetch_max(*i as i32 + 1, std::sync::atomic::Ordering::Relaxed);
                let mut b = ctl(h, id);
                if b.0.is_null() {
                    b = child_at(h, w!("BUTTON"), tx(crate::Key::Delete), 0xB | 0x00010000, id, sn.font);
                }
                let _ = SetWindowTextW(b, &HSTRING::from(tx(crate::Key::Delete)));
                move_to(b, *btn, intersects(*btn, content));
                let rid = S_REN0 + *i as i32;
                let mut r = ctl(h, rid);
                if r.0.is_null() {
                    r = child_at(h, w!("BUTTON"), tx(crate::Key::Rename2), 0xB | 0x00010000, rid, sn.font);
                }
                let _ = SetWindowTextW(r, &HSTRING::from(tx(crate::Key::Rename2)));
                move_to(r, *rename, intersects(*rename, content));
            }
            _ => {}
        }
    }
}

fn intersects(a: RECT, b: RECT) -> bool {
    a.right > a.left && a.bottom > a.top && a.bottom > b.top && a.top < b.bottom && a.right > b.left && a.left < b.right
}

fn inset(r: RECT, d: i32) -> RECT {
    RECT { left: r.left + d, top: r.top + d, right: (r.right - d).max(r.left + d), bottom: (r.bottom - d).max(r.top + d) }
}

unsafe fn move_to(h: HWND, r: RECT, show: bool) {
    if h.0.is_null() {
        return;
    }
    if !show {
        let _ = ShowWindow(h, SW_HIDE);
        return;
    }
    let _ = SetWindowPos(h, None, r.left, r.top, (r.right - r.left).max(1), (r.bottom - r.top).max(1), SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

unsafe fn paint(h: HWND) {
    let sn = snap();
    crate::buffered(h, |dc, rc| {
        let g = geo(rc, &sn);
        draw(dc, rc, &g, &sn);
    });
}

unsafe fn draw(dc: HDC, rc: RECT, g: &Geo, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    let accent = chrome();
    crate::fill(dc, rc, crate::pal().bg);
    crate::fill(dc, RECT { bottom: g.footer.top + 1, top: g.footer.top, ..g.footer }, crate::pal().line);
    draw_field(dc, g.search, dpi);
    if !g.show.iter().any(|s| *s) {
        crate::text(dc, RECT { left: sc(dpi, 16), top: g.search.bottom + sc(dpi, 12), right: g.nav_w - sc(dpi, 8), bottom: g.search.bottom + sc(dpi, 40) }, tx(NONE), sn.font, crate::pal().sub, DT_LEFT);
    }
    for i in 0..PAGE_N {
        if !g.show[i] {
            continue;
        }
        let r = g.nav[i];
        let on = i == sn.page;
        if on || sn.hot == Some(i) {
            let t = if on { 0.08 } else { 0.04 };
            rfill(dc, r, sc(dpi, 4), crate::blend(crate::pal().bg, crate::pal().text, t), None);
        }
        if on {
            let bar = RECT { left: r.left, top: r.top + sc(dpi, 10), right: r.left + sc(dpi, 3), bottom: r.bottom - sc(dpi, 10) };
            rfill(dc, bar, sc(dpi, 2), accent, None);
        }
        crate::text(dc, RECT { left: r.left + sc(dpi, 14), ..r }, tx(PAGES[i]), sn.font, crate::pal().text, DT_LEFT);
    }
    crate::text(dc, g.hint, tx(FOOT), sn.small, crate::pal().sub, DT_LEFT);
    let _ = SaveDC(dc);
    let _ = IntersectClipRect(dc, g.content.left, g.content.top, g.content.right, g.content.bottom);
    // A miss keeps the user's page but must not paint it. Shortcuts would otherwise
    // show its title and the empty key table while the nav says nothing matched.
    if search_miss(&sn.query) {
        let r = RECT {
            left: g.content.left + sc(dpi, 28),
            top: g.content.top + sc(dpi, 24),
            right: g.content.right - sc(dpi, 28),
            bottom: g.content.top + sc(dpi, 72),
        };
        crate::text(dc, r, tx(NO_RESULTS), sn.title_font, crate::pal().text, DT_LEFT);
        let _ = RestoreDC(dc, -1);
        return;
    }
    let title = tx(PAGES[sn.page]);
    crate::text(dc, g.title, title, sn.title_font, crate::pal().text, DT_LEFT);
    let fav_rows = if sn.page == 4 { fav_tree_rows() } else { Vec::new() };
    for item in &g.items {
        match item {
            Item::Card(c) => draw_card(dc, c, sn, accent),
            Item::Head(r, t) => crate::text(dc, *r, tx(*t), sn.font, crate::pal().text, DT_LEFT),
            Item::Table(r) => rfill(dc, *r, sc(dpi, 6), crate::pal().card, Some(crate::pal().line)),
            Item::FavHead(r) => crate::paint_fav_head(dc, *r, dpi, sn.small),
            Item::FavRow { i, rect, .. } => {
                let empty = *i == usize::MAX;
                let row = if empty { None } else { fav_rows.get(*i) };
                draw_fav_row(dc, row, *rect, sn, empty);
            }
            Item::Note(r, t) => crate::text(dc, *r, tx(*t), sn.small, crate::pal().sub, DT_LEFT),
            Item::CtxNote(r) => {
                let _ = crate::wrap_text(dc, *r, tx(CTX_NOTE), sn.small, crate::pal().sub, false);
            }
            Item::CtxSeg { classic, win11 } => {
                let on_win = app(|a| a.s.ctx_win11_view);
                draw_seg(dc, *classic, tx(CTX_CLASSIC), !on_win, sn, accent);
                draw_seg(dc, *win11, tx(CTX_WIN11), on_win, sn, accent);
            }
            Item::CtxBar { .. } => {}
            Item::CtxRow { id, rect, mark, chev, name, detail, raw, tick, chips, packaged, .. } => {
                let open = !*packaged && sn.ctx_open.iter().any(|x| x == id);
                draw_ctx_row(dc, *rect, *mark, *chev, name, detail, raw, *tick, chips, open, *packaged, sn, accent);
            }
            Item::CtxSub { rect, mark, bit, tick, .. } => draw_ctx_sub(dc, *rect, *mark, *bit, *tick, sn, accent),
            Item::CtxEmpty(r, key) => crate::text(dc, *r, tx(*key), sn.small, crate::pal().sub, DT_LEFT),
            Item::KeyBar { edit, .. } => draw_field(dc, *edit, dpi),
            Item::KeyHead(r) => {
                let (a, b) = key_cols(*r);
                crate::text(dc, a, tx(crate::Key::Category), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: a.right, right: b, ..*r }, tx(crate::Key::Action), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: b, ..*r }, tx(crate::Key::Shortcut), sn.small, crate::pal().sub, DT_LEFT);
                crate::fill(dc, RECT { top: r.bottom - 1, ..*r }, crate::pal().line);
            }
            Item::KeyRow { i, rect, change, reset } => draw_key_row(dc, *i, *rect, *change, *reset, sn),
        }
    }
    if g.has_bar {
        rfill(dc, g.track, sc(dpi, 4), crate::blend(crate::pal().bg, crate::pal().text, 0.08), None);
        rfill(dc, g.thumb, sc(dpi, 4), crate::blend(crate::pal().bg, crate::pal().text, 0.35), None);
    }
    let _ = RestoreDC(dc, -1);
}

fn key_cols(r: RECT) -> (RECT, i32) {
    let w = r.right - r.left;
    let a = RECT { left: r.left + (w * 2 / 100).max(8), right: r.left + w * 16 / 100, ..r };
    (a, r.left + w * 40 / 100)
}

fn chip_h(dpi: i32) -> i32 {
    sc(dpi, 24)
}

fn chord_gap(dpi: i32) -> i32 {
    sc(dpi, 6)
}

/// Change and Reset, sitting at the right of a shortcut row.
pub(crate) fn key_btn_rects(r: RECT, dpi: i32) -> (RECT, RECT) {
    let h = sc(dpi, 24).min((r.bottom - r.top - sc(dpi, 4)).max(sc(dpi, 18)));
    let top = r.top + ((r.bottom - r.top - h) / 2).max(0);
    let gap = sc(dpi, 6);
    let edge = r.right - sc(dpi, 8);
    let reset = RECT { left: edge - sc(dpi, 72), top, right: edge, bottom: top + h };
    let change = RECT { left: reset.left - gap - sc(dpi, 76), top, right: reset.left - gap, bottom: top + h };
    (change, reset)
}

/// Shortcut column: x where chips start, and the width they must stay inside.
fn chord_box(r: RECT, dpi: i32) -> (i32, i32) {
    let (_, keys_x) = key_cols(r);
    let (change, _) = key_btn_rects(r, dpi);
    (keys_x, (change.left - sc(dpi, 8) - keys_x).max(sc(dpi, 36)))
}

/// Pack chip widths onto lines that stay within `max_w`. A chip wider than the
/// cell is clamped so the row cannot paint past the table's right edge.
fn pack_chords(widths: &[i32], max_w: i32, gap: i32) -> Vec<Vec<i32>> {
    let max_w = max_w.max(1);
    let mut lines: Vec<Vec<i32>> = Vec::new();
    let mut line: Vec<i32> = Vec::new();
    let mut used = 0i32;
    for &raw in widths {
        let w = raw.clamp(1, max_w);
        if !line.is_empty() && used.saturating_add(gap).saturating_add(w) > max_w {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        line.push(w);
        used = if line.len() == 1 { w } else { used.saturating_add(gap).saturating_add(w) };
    }
    if !line.is_empty() {
        lines.push(line);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

fn key_row_h_from_lines(n: i32, dpi: i32) -> i32 {
    let n = n.max(1);
    let min_h = sc(dpi, 40);
    if n == 1 {
        return min_h;
    }
    let pad = sc(dpi, 8);
    let gap = sc(dpi, 4);
    (pad * 2 + n * chip_h(dpi) + (n - 1) * gap).max(min_h)
}

unsafe fn chord_widths(dc: HDC, font: HFONT, keys: &[&str], dpi: i32) -> Vec<i32> {
    let pad = sc(dpi, 14);
    keys.iter()
        .map(|k| {
            let tw = if font.is_invalid() || dc.is_invalid() {
                sc(dpi, 8) * k.chars().count() as i32
            } else {
                text_w(dc, font, k)
            };
            (tw + pad).max(pad)
        })
        .collect()
}

unsafe fn key_row_height(dc: HDC, keys: &[&str], row: RECT, dpi: i32, font: HFONT) -> i32 {
    let (_, max_w) = chord_box(row, dpi);
    let lines = pack_chords(&chord_widths(dc, font, keys, dpi), max_w, chord_gap(dpi));
    key_row_h_from_lines(lines.len() as i32, dpi)
}

pub(crate) unsafe fn key_row_heights(labels: &[String], row: RECT, dpi: i32, font: HFONT) -> Vec<i32> {
    if labels.is_empty() {
        return vec![sc(dpi, 40)];
    }
    let dc = GetDC(None);
    let font = if dc.is_invalid() { HFONT::default() } else { font };
    let heights = labels.iter().map(|label| key_row_height(dc, &[label.as_str()], row, dpi, font)).collect();
    if !dc.is_invalid() {
        let _ = ReleaseDC(None, dc);
    }
    heights
}

unsafe fn draw_card(dc: HDC, c: &CardItem, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    // Disabled rows stay on a white (or dark-theme card) ground; only the label grays.
    rfill(dc, c.rect, sc(dpi, 6), crate::pal().card, Some(crate::pal().line));
    if c.spec.ctl == Ctl::WmPath {
        let title_r = RECT {
            left: c.rect.left + sc(dpi, 16),
            top: c.rect.top + sc(dpi, 12),
            right: c.adv[0].left - sc(dpi, 12),
            bottom: c.adv[0].bottom,
        };
        crate::text(dc, title_r, tx(c.spec.title), sn.font, crate::pal().text, DT_LEFT);
        let desc_r = RECT {
            left: c.rect.left + sc(dpi, 16),
            top: c.adv[0].bottom + sc(dpi, 2),
            right: c.rect.right - sc(dpi, 16),
            bottom: c.ctl.top - sc(dpi, 4),
        };
        crate::text(dc, desc_r, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
        draw_field(dc, c.ctl, dpi);
        return;
    }
    let color = if c.spec.live { crate::pal().text } else { crate::pal().sub };
    let title = if c.spec.ctl == Ctl::About {
        format!("Panegrid v{}", env!("CARGO_PKG_VERSION"))
    } else {
        tx(c.spec.title).to_string()
    };
    let text_r = RECT { left: c.rect.left + sc(dpi, 16), top: c.rect.top + sc(dpi, 12), right: c.ctl.left.min(c.sw.first().map(|s| s.left).unwrap_or(i32::MAX)) - sc(dpi, 12), bottom: c.rect.bottom - sc(dpi, 12) };
    let title_line = if tx(c.spec.desc).is_empty() { text_r } else { RECT { bottom: text_r.top + sc(dpi, 20), ..text_r } };
    crate::text(dc, title_line, &title, sn.font, color, DT_LEFT);
    if c.spec.ctl == Ctl::EvStatus || c.spec.ctl == Ctl::WmStatus {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: title_line.bottom + line, ..text_r };
        if c.spec.ctl == Ctl::WmStatus {
            let found = crate::winmerge::status_now(&app(|a| a.s.winmerge_path.clone()));
            let (head, detail) = crate::winmerge::status_text(&found);
            crate::text(dc, desc_r, &head, sn.small, crate::pal().text, DT_LEFT);
            let more = RECT { top: desc_r.bottom, bottom: (desc_r.bottom + line).min(text_r.bottom), ..text_r };
            crate::text(dc, more, &detail, sn.small, crate::pal().sub, DT_LEFT);
        } else {
            crate::text(dc, desc_r, &ev_status_line(), sn.small, crate::pal().text, DT_LEFT);
        }
    } else if c.spec.ctl == Ctl::EvAct {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: (title_line.bottom + line).min(text_r.bottom), ..text_r };
        let act = ev_activity_line();
        let text = if !act.is_empty() {
            act
        } else if matches!(crate::everything::shown(), crate::everything::Shown::Ready(crate::everything::Presence::Running { .. })) {
            tx(crate::Key::EverythingIsAlreadyRunning).to_string()
        } else {
            tx(c.spec.desc).to_string()
        };
        crate::text(dc, desc_r, &text, sn.small, crate::pal().sub, DT_LEFT);
    } else if c.spec.ctl == Ctl::Adv {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: title_line.bottom + line, ..text_r };
        crate::text(dc, desc_r, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
        let path = crate::store::settings_path().to_string_lossy().into_owned();
        let path_r = RECT { left: c.rect.left + sc(dpi, 16), top: desc_r.bottom, right: c.rect.right - sc(dpi, 16), bottom: c.rect.bottom - sc(dpi, 6) };
        crate::text(dc, path_r, &path, sn.small, crate::pal().text, DT_LEFT);
    } else if !tx(c.spec.desc).is_empty() {
        crate::text(dc, RECT { top: title_line.bottom, ..text_r }, tx(c.spec.desc), sn.small, crate::pal().sub, DT_LEFT);
    }
    match c.spec.ctl {
        Ctl::Toggle(id) => draw_toggle(dc, c.ctl, toggle_on(id), c.spec.live, accent, dpi),
        Ctl::CopyEng => crate::text(dc, c.ctl, "IFileOperation", sn.small, crate::pal().sub, DT_RIGHT),
        Ctl::Accent => {
            let colors = crate::pal().accents;
            let cur = app(|a| a.s.accent_now());
            for (i, r) in c.sw.iter().enumerate() {
                let col = colors.get(i).map(|(_, c)| *c).unwrap_or(0);
                if col == cur {
                    ellipse(dc, RECT { left: r.left - sc(dpi, 2), top: r.top - sc(dpi, 2), right: r.right + sc(dpi, 2), bottom: r.bottom + sc(dpi, 2) }, crate::pal().text, None);
                }
                ellipse(dc, *r, col, None);
            }
        }
        _ => {}
    }
}

unsafe fn draw_tick(dc: HDC, r: RECT, tick: crate::shell::ctxmenu::Tick, accent: u32, dpi: i32, icon: HFONT, ground: u32) {
    let rad = sc(dpi, 3);
    match tick {
        crate::shell::ctxmenu::Tick::On => {
            aa_rfill(dc, r, rad, accent, Some(accent), ground);
            crate::text(dc, r, "\u{E73E}", icon, on_chrome(accent), DT_CENTER);
        }
        crate::shell::ctxmenu::Tick::Mixed => {
            aa_rfill(dc, r, rad, crate::pal().card, Some(accent), ground);
            let mid = (r.top + r.bottom) / 2;
            crate::fill(dc, RECT { left: r.left + sc(dpi, 3), top: mid - sc(dpi, 1), right: r.right - sc(dpi, 3), bottom: mid + sc(dpi, 1) }, accent);
        }
        crate::shell::ctxmenu::Tick::Off => aa_rfill(dc, r, rad, crate::pal().card, Some(crate::pal().line), ground),
    }
}

unsafe fn draw_seg(dc: HDC, r: RECT, label: &str, on: bool, sn: &Snap, accent: u32) {
    let h = (r.bottom - r.top).max(1);
    let ground = crate::pal().bg;
    if on {
        aa_rfill(dc, r, h / 2, accent, None, ground);
        crate::text(dc, r, label, sn.font, on_chrome(accent), DT_CENTER);
    } else {
        aa_rfill(dc, r, h / 2, crate::pal().card, Some(crate::pal().line), ground);
        crate::text(dc, r, label, sn.font, crate::pal().text, DT_CENTER);
    }
}

unsafe fn draw_ctx_row(dc: HDC, rect: RECT, mark: RECT, chev: RECT, name: &str, detail: &str, raw: &str, tick: crate::shell::ctxmenu::Tick, chips: &[(u8, RECT)], open: bool, packaged: bool, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    rfill(dc, rect, sc(dpi, 6), crate::pal().card, Some(crate::pal().line));
    draw_tick(dc, mark, tick, accent, dpi, sn.icon, crate::pal().card);
    let text_left = if packaged {
        mark.right + sc(dpi, 8)
    } else {
        let chev_box = RECT { top: mark.top, bottom: mark.bottom, ..chev };
        crate::text(dc, chev_box, if open { "\u{E70D}" } else { "\u{E76C}" }, sn.icon, crate::pal().sub, DT_CENTER);
        chev.right
    };
    let name_right = chips.first().map(|(_, r)| r.left - sc(dpi, 8)).unwrap_or(rect.right - sc(dpi, 12));
    let text_r = RECT { left: text_left, right: name_right, ..rect };
    let line = detail;
    if line.is_empty() && raw.is_empty() {
        crate::text(dc, text_r, name, sn.bold, crate::pal().text, DT_LEFT);
    } else {
        let title = RECT { top: rect.top + sc(dpi, 6), bottom: rect.top + sc(dpi, 28), ..text_r };
        crate::text(dc, title, name, sn.bold, crate::pal().text, DT_LEFT);
        let sub = RECT { top: title.bottom, bottom: rect.bottom - sc(dpi, 4), ..text_r };
        let mut left = sub.left;
        if !line.is_empty() {
            let w = line_width(dc, line, sn.small);
            let shown = RECT { right: (left + w).min(sub.right), ..sub };
            crate::text(dc, shown, line, sn.small, crate::pal().sub, DT_LEFT);
            left = shown.right + sc(dpi, 10);
        }
        if !raw.is_empty() && left + sc(dpi, 24) < sub.right {
            crate::text(dc, RECT { left, ..sub }, raw, sn.small, crate::blend(crate::pal().sub, crate::pal().card, 0.45), DT_LEFT);
        }
    }
    for (bit, r) in chips {
        rfill(dc, *r, sc(dpi, 4), crate::blend(crate::pal().card, crate::pal().text, 0.06), Some(crate::pal().line));
        crate::text(dc, *r, crate::shell::ctxmenu::target_label(*bit), sn.small, crate::pal().sub, DT_CENTER);
    }
}

unsafe fn line_width(dc: HDC, s: &str, font: HFONT) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    DrawTextW(dc, &mut w, &mut r, DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right - r.left
}

unsafe fn draw_ctx_sub(dc: HDC, rect: RECT, mark: RECT, bit: u8, tick: crate::shell::ctxmenu::Tick, sn: &Snap, accent: u32) {
    let dpi = sn.dpi.max(96);
    draw_tick(dc, mark, tick, accent, dpi, sn.icon, crate::pal().bg);
    crate::text(dc, RECT { left: mark.right + sc(dpi, 8), ..rect }, crate::shell::ctxmenu::target_label(bit), sn.font, crate::pal().text, DT_LEFT);
}

fn toggle_knob_rect(r: RECT, on: bool, dpi: i32) -> RECT {
    let k = sc(dpi, 12);
    let pad = sc(dpi, 3);
    let x = if on { r.right - pad - k } else { r.left + pad };
    let y = r.top + (r.bottom - r.top - k) / 2;
    RECT { left: x, top: y, right: x + k, bottom: y + k }
}

unsafe fn draw_toggle(dc: HDC, r: RECT, on: bool, enabled: bool, accent: u32, dpi: i32) {
    // 40×20 pill. Off and disabled stay an outlined gray track on the white card.
    let (fill, border, knob) = if !enabled || !on {
        (crate::pal().card, crate::pal().sub, crate::pal().sub)
    } else {
        (accent, accent, on_chrome(accent))
    };
    let rad = (r.bottom - r.top).max(1);
    let knob_r = toggle_knob_rect(r, on, dpi);
    // One pass so the knob's edge blends into the pill, not into a second square blit.
    let painted = aa_blit(dc, r, crate::pal().card, |mem, scale, ox, oy| unsafe {
        paint_round(mem, scaled_from(r, r, scale, ox, oy), rad * scale, fill, Some(border), scale);
        ellipse(mem, scaled_from(r, knob_r, scale, ox, oy), knob, None);
    });
    if !painted {
        rfill(dc, r, rad, fill, Some(border));
        ellipse(dc, knob_r, knob, None);
    }
}

unsafe fn draw_fav_row(dc: HDC, row: Option<&crate::FavRow>, r: RECT, sn: &Snap, empty: bool) {
    crate::paint_fav_row(dc, row, r, sn.dpi.max(96), sn.font, sn.small, sn.icon, empty, false);
}

unsafe fn draw_key_row(dc: HDC, i: usize, r: RECT, change: RECT, reset: RECT, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    if i == usize::MAX || i >= crate::keys::len() {
        crate::text(dc, r, tx(NONE), sn.small, crate::pal().sub, DT_CENTER);
        return;
    }
    if crate::keys::capture_row() == Some(i) {
        crate::fill(dc, r, crate::blend(crate::pal().bg, chrome(), 0.16));
    }
    let (cat_s, act_s, chord) = crate::keys::row(i);
    let shown = if crate::keys::capture_row() == Some(i) {
        tx(crate::Key::PressKey).to_string()
    } else {
        chord
    };
    let (cat, keys_x) = key_cols(r);
    crate::text(dc, cat, &cat_s, sn.font, crate::pal().sub, DT_LEFT);
    crate::text(dc, RECT { left: cat.right, right: keys_x - sc(dpi, 8), ..r }, &act_s, sn.font, crate::pal().text, DT_LEFT);
    let (_, max_w) = chord_box(r, dpi);
    let widths = chord_widths(dc, sn.mono, &[&shown], dpi);
    let lines = pack_chords(&widths, max_w, chord_gap(dpi));
    let n = lines.len().max(1) as i32;
    let ch = chip_h(dpi);
    let lg = sc(dpi, 4);
    let stack = n * ch + (n - 1) * lg;
    let y = r.top + (r.bottom - r.top - stack).max(0) / 2;
    let limit = keys_x + max_w;
    let border = if crate::keys::is_custom(i) { chrome() } else { crate::pal().line };
    let mut x = keys_x;
    if let Some(line) = lines.first() {
        for &w in line {
            let right = (x + w).min(limit);
            let chip = RECT { left: x, top: y, right, bottom: (y + ch).min(r.bottom - 1) };
            if chip.right > chip.left && chip.bottom > chip.top {
                rfill(dc, chip, sc(dpi, 6), crate::pal().bg, Some(border));
                crate::text(dc, chip, &shown, sn.mono, crate::pal().text, DT_CENTER);
            }
            x = right + chord_gap(dpi);
        }
    }
    let mini = |rect: RECT, label: &str, on: bool| {
        if rect.right <= rect.left {
            return;
        }
        let ink = if on { crate::pal().text } else { crate::pal().sub };
        rfill(dc, rect, sc(dpi, 4), crate::pal().card, Some(if on { crate::pal().line } else { crate::pal().line }));
        crate::text(dc, rect, label, sn.small, ink, DT_CENTER);
    };
    mini(change, tx(crate::Key::Change), true);
    mini(reset, tx(crate::Key::Reset3), crate::keys::is_custom(i));
    crate::fill(dc, RECT { top: r.bottom - 1, ..r }, crate::pal().line);
}

unsafe fn text_w(dc: HDC, font: HFONT, s: &str) -> i32 {
    if s.is_empty() {
        return 0;
    }
    let mut w: Vec<u16> = s.encode_utf16().collect();
    let mut r = RECT::default();
    let old = SelectObject(dc, font.into());
    let _ = DrawTextW(dc, &mut w, &mut r, DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old);
    r.right
}

/// Search-box frame: card fill, `line` border, 4 DIP corners. The name prompt uses this too.
unsafe fn draw_field(dc: HDC, r: RECT, dpi: i32) {
    rfill(dc, r, sc(dpi, FIELD_RADIUS_DIP), crate::pal().card, Some(crate::pal().line));
}

/// Supersample factor. 3× is enough for the knob; the DIB is thrown away after the blit.
const AA_SCALE: i32 = 3;
/// Parent-coloured pixels around the shape so a curve that crosses a sample block can blend.
const AA_PAD: i32 = 2;

fn scaled_from(outer: RECT, part: RECT, scale: i32, ox: i32, oy: i32) -> RECT {
    RECT {
        left: ox + (part.left - outer.left) * scale,
        top: oy + (part.top - outer.top) * scale,
        right: ox + (part.right - outer.left) * scale,
        bottom: oy + (part.bottom - outer.top) * scale,
    }
}

/// Average each `scale`×`scale` block. Flat fills stay exact; only an edge that mixes both colours softens.
fn box_down(src: &[u8], sw: usize, dst: &mut [u8], dw: usize, dh: usize, scale: usize) {
    let area = (scale * scale) as u32;
    for y in 0..dh {
        for x in 0..dw {
            let mut b = 0u32;
            let mut g = 0u32;
            let mut rch = 0u32;
            let y0 = y * scale;
            let x0 = x * scale;
            for sy in 0..scale {
                let row = (y0 + sy) * sw + x0;
                for sx in 0..scale {
                    let i = (row + sx) * 4;
                    b += src[i] as u32;
                    g += src[i + 1] as u32;
                    rch += src[i + 2] as u32;
                }
            }
            let o = (y * dw + x) * 4;
            dst[o] = (b / area) as u8;
            dst[o + 1] = (g / area) as u8;
            dst[o + 2] = (rch / area) as u8;
            // Already composited against `ground`. Opaque so the buffered DC does not blend it again.
            dst[o + 3] = 255;
        }
    }
}

unsafe fn dib_section(dc: HDC, w: i32, h: i32) -> Option<(HBITMAP, *mut u8)> {
    if w <= 0 || h <= 0 {
        return None;
    }
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let bmp = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
    if bits.is_null() {
        let _ = DeleteObject(bmp.into());
        return None;
    }
    Some((bmp, bits.cast()))
}

/// Draw the shape at `AA_SCALE` into a DIB cleared to `ground`, box-filter it, and copy it down.
/// `paint` gets the memory DC, the scale, and the origin of `r` inside the padded bitmap.
unsafe fn aa_blit(dc: HDC, r: RECT, ground: u32, paint: impl FnOnce(HDC, i32, i32, i32)) -> bool {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || h <= 0 || w > 2400 || h > 800 {
        return false;
    }
    let scale = AA_SCALE;
    let pad = AA_PAD;
    let (bw, bh) = (w + pad * 2, h + pad * 2);
    let (sw, sh) = (bw * scale, bh * scale);
    let Some((big, big_bits)) = dib_section(dc, sw, sh) else { return false };
    let mem = CreateCompatibleDC(Some(dc));
    if mem.is_invalid() {
        let _ = DeleteObject(big.into());
        return false;
    }
    let old = SelectObject(mem, big.into());
    crate::fill(mem, RECT { left: 0, top: 0, right: sw, bottom: sh }, ground);
    paint(mem, scale, pad * scale, pad * scale);
    let _ = GdiFlush();
    SelectObject(mem, old);
    let _ = DeleteDC(mem);

    let Some((small, small_bits)) = dib_section(dc, bw, bh) else {
        let _ = DeleteObject(big.into());
        return false;
    };
    box_down(
        std::slice::from_raw_parts(big_bits, (sw as usize) * (sh as usize) * 4),
        sw as usize,
        std::slice::from_raw_parts_mut(small_bits, (bw as usize) * (bh as usize) * 4),
        bw as usize,
        bh as usize,
        scale as usize,
    );
    let _ = DeleteObject(big.into());

    let mem = CreateCompatibleDC(Some(dc));
    if mem.is_invalid() {
        let _ = DeleteObject(small.into());
        return false;
    }
    let old = SelectObject(mem, small.into());
    let copied = StretchBlt(dc, r.left - pad, r.top - pad, bw, bh, Some(mem), 0, 0, bw, bh, SRCCOPY);
    SelectObject(mem, old);
    let _ = DeleteDC(mem);
    let _ = DeleteObject(small.into());
    copied.as_bool()
}

pub(crate) unsafe fn aa_rfill(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>, ground: u32) {
    let painted = aa_blit(dc, r, ground, |mem, scale, ox, oy| unsafe {
        paint_round(mem, scaled_from(r, r, scale, ox, oy), rad.max(1) * scale, fill, border, scale);
    });
    if !painted {
        rfill(dc, r, rad, fill, border);
    }
}

/// Border is a 1px ring: the outer shape is the border colour, the fill sits `inset` source pixels in.
unsafe fn paint_round(dc: HDC, r: RECT, rad: i32, fill: u32, border: Option<u32>, inset: i32) {
    let Some(edge) = border.filter(|c| *c != fill) else {
        rfill(dc, r, rad.max(1), fill, None);
        return;
    };
    rfill(dc, r, rad.max(1), edge, None);
    let inner = RECT { left: r.left + inset, top: r.top + inset, right: r.right - inset, bottom: r.bottom - inset };
    if inner.right <= inner.left || inner.bottom <= inner.top {
        return;
    }
    rfill(dc, inner, (rad - inset).max(1), fill, None);
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

unsafe fn ellipse(dc: HDC, r: RECT, fill: u32, border: Option<u32>) {
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
    let _ = Ellipse(dc, r.left, r.top, r.right, r.bottom);
    SelectObject(dc, op);
    SelectObject(dc, ob);
    let _ = DeleteObject(brush.into());
    if border.is_some() {
        let _ = DeleteObject(pen.into());
    }
}

unsafe fn draw_btn(d: &DRAWITEMSTRUCT) {
    let id = d.CtlID as i32;
    let dis = d.itemState.0 & 4 != 0;
    let pressed = d.itemState.0 & 1 != 0;
    let hot = app(|a| a.hover) == d.hwndItem;
    let dpi = ST.with(|s| s.borrow().dpi.max(96));
    let font = ST.with(|s| s.borrow().font);
    let accent = chrome();
    // Prompt Close is also id 2 (S_CLOSE). Only a settings child uses that id as the accent;
    // the prompt accents OK, which is id 1.
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_settings = parent.0.is_null() || parent == hwnd();
    let accent_btn = if on_settings { id == S_CLOSE } else { id == 1 };
    let on_card = if on_settings { id != S_CLOSE && id != S_RESET } else { true };
    let ground = if on_card { crate::pal().card } else { crate::pal().bg };
    crate::fill(dc_of(d), d.rcItem, ground);
    let (fill, border, ink) = if accent_btn && !dis {
        (accent, accent, on_chrome(accent))
    } else if dis {
        (crate::blend(ground, crate::pal().sub, 0.12), crate::pal().line, crate::pal().sub)
    } else if pressed {
        (crate::blend(ground, crate::pal().text, 0.08), crate::pal().line, crate::pal().text)
    } else if hot {
        (crate::blend(ground, accent, 0.12), accent, crate::pal().text)
    } else if id == S_ACCENT && app(|a| !crate::pal().accents.iter().any(|(_, c)| *c == a.s.accent_now())) {
        (ground, accent, crate::pal().text)
    } else if id == S_ADV_RESET {
        (ground, crate::pal().line, 0xC42B1C)
    } else {
        (ground, crate::pal().line, crate::pal().text)
    };
    rfill(dc_of(d), d.rcItem, sc(dpi, 4), fill, Some(border));
    let label = crate::text_of(d.hwndItem);
    crate::text(dc_of(d), d.rcItem, &label, font, ink, DT_CENTER);
}

fn dc_of(d: &DRAWITEMSTRUCT) -> HDC {
    d.hDC
}

fn ev_button_label() -> &'static str {
    match crate::everything::action() {
        Some(crate::everything::EvAction::Start) => tx(crate::Key::StartEverything),
        _ => tx(crate::Key::InstallEverything),
    }
}

fn ev_status_line() -> String {
    use crate::everything::{Presence, Shown};
    match crate::everything::shown() {
        Shown::Unknown => tx(crate::Key::Checking).to_string(),
        Shown::Ready(Presence::NotInstalled) => tx(crate::Key::NotInstalled).to_string(),
        Shown::Ready(Presence::InstalledStopped) => tx(crate::Key::InstalledButNotRunning).to_string(),
        Shown::Ready(Presence::Running { version }) => {
            let base = tx(crate::Key::InstalledAndRunning);
            match version {
                Some(v) => format!("{base} · {}", v.label()),
                None => base.to_string(),
            }
        }
    }
}

fn ev_activity_line() -> String {
    use crate::everything::Activity;
    match crate::everything::activity() {
        Activity::Idle | Activity::Checking => String::new(),
        Activity::Starting => tx(crate::Key::StartingEverything).to_string(),
        Activity::Installing => tx(crate::Key::InstallingEverything).to_string(),
        Activity::Winget(line) => line,
        Activity::Started => tx(crate::Key::EverythingIsRunning).to_string(),
        Activity::Installed => tx(crate::Key::EverythingWasInstalled).to_string(),
        Activity::StartFailed => tx(crate::Key::EverythingDidNotStart).to_string(),
        Activity::InstallFailed(code) => format!("{} ({code})", tx(crate::Key::InstallFailed)),
        Activity::OpenedPage => tx(crate::Key::OpenedDownloadPage).to_string(),
    }
}

fn flip(id: u8) {
    match id {
        TG_SINGLE => app(|a| a.s.single ^= true),
        TG_ONE => app(|a| a.s.one_window ^= true),
        TG_DBLUP => app(|a| a.s.dblup ^= true),
        TG_HIDDEN => {
            unsafe { crate::toggle_hidden() };
            return;
        }
        TG_BAR => {
            app(|a| a.s.bar ^= true);
            unsafe {
                crate::save_session();
                // Every window shares the switch. Lay each one out, or a window that was not
                // current would keep the icons while its tabs already took that width.
                crate::across_windows(|| {
                    crate::relayout();
                    crate::redraw_all();
                });
                redraw();
            }
            return;
        }
        TG_DRIVES => {
            app(|a| a.s.drive_buttons ^= true);
            unsafe {
                crate::save_session();
                // Every window shares the switch. Lay each one out, or a window that was not
                // current would keep the letters while its tabs already took that width.
                crate::across_windows(|| {
                    crate::relayout();
                    crate::redraw_all();
                });
                redraw();
            }
            return;
        }
        TG_STATUS => app(|a| a.s.stat ^= true),
        TG_EXT => {
            let on = app(|a| {
                a.s.ext ^= true;
                a.s.ext
            });
            unsafe {
                crate::apply_ext(on);
                crate::save_session();
                redraw();
            }
            return;
        }
        TG_TREE => {
            unsafe { crate::toggle_tree() };
            return;
        }
        TG_EDGE => app(|a| a.s.edge ^= true),
        TG_CONFIRM => app(|a| a.s.confirm ^= true),
        TG_FAST => app(|a| a.s.fast_copy ^= true),
        TG_AUTOSAVE => app(|a| a.s.autosave ^= true),
        TG_UPDATE => app(|a| a.s.update_check ^= true),
        TG_EV => {
            app(|a| a.s.use_everything ^= true);
            unsafe {
                crate::save_session();
                redraw();
            }
            return;
        }
        TG_SPACE => app(|a| a.s.space_preview ^= true),
        TG_WHEEL => app(|a| a.s.peek_wheel_nav ^= true),
        TG_FSIZE => {
            app(|a| a.s.folder_sizes ^= true);
            let (show, net) = app(|a| (a.s.folder_sizes, a.s.folder_sizes_network));
            crate::set_opts(show, net);
            if !show {
                crate::clear_pending();
            }
            app(|a| a.status_key.clear());
        }
        TG_FSIZE_NET => {
            app(|a| a.s.folder_sizes_network ^= true);
            let (show, net) = app(|a| (a.s.folder_sizes, a.s.folder_sizes_network));
            crate::set_opts(show, net);
            crate::clear_pending();
            app(|a| a.status_key.clear());
        }
        TG_REMEMBER => {
            app(|a| a.s.remember_views ^= true);
            let on = app(|a| a.s.remember_views);
            unsafe {
                // Other windows read the flag from the shared session. Arm or capture before they poll.
                crate::publish_shared();
                crate::across_windows(|| {
                    if on {
                        crate::arm_folder_watches();
                    } else {
                        // Each pane keeps the view on screen as its own, now that folders no longer override it.
                        let n = app(|a| a.s.panes());
                        for i in 0..n {
                            crate::save_view(i);
                        }
                    }
                });
                crate::save_session();
                redraw();
            }
            return;
        }
        TG_DOT => {
            app(|a| a.s.dot ^= true);
            unsafe {
                crate::save_session();
                crate::check_dirty();
                redraw();
            }
            return;
        }
        TG_FOLDER => {
            let want = !crate::folderapp::shown_on();
            crate::folderapp::set_registered(want);
            unsafe {
                if crate::folderapp::shown_on() != want {
                    let q = tx(crate::Key::CouldNotChangeDefaultFolderApp);
                    let _ = MessageBoxW(Some(hwnd()), &HSTRING::from(q), w!("Panegrid"), MB_ICONWARNING);
                }
                redraw();
            }
            return;
        }
        _ => return,
    }
    unsafe {
        crate::save_session();
        crate::relayout();
        crate::redraw_all();
        redraw();
    }
}

unsafe fn delete_fav(k: usize) {
    let Some(id) = fav_node(k).map(|r| r.id) else { return };
    app(|a| {
        a.s.ensure_fav_tree();
        crate::delete_at(&mut a.s.tree, &id);
        a.s.sync_favs();
        a.fav_hot = None;
    });
    crate::commit_favs();
}

unsafe fn on_command(h: HWND, id: i32, code: u32) {
    if BUSY.with(|b| b.get()) {
        return;
    }
    if code == EN_CHANGE && id == S_SEARCH {
        let q = crate::text_of(ctl(h, S_SEARCH));
        let (anchor, page) = ST.with(|s| {
            let st = s.borrow();
            (st.anchor, st.page)
        });
        let (anchor, page) = commit_search(anchor, page, &q);
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.query = q;
            st.anchor = anchor;
            st.page = page;
            st.scroll = 0;
        });
        layout(h);
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == EN_CHANGE && id == S_WM {
        let path = crate::text_of(ctl(h, S_WM));
        app(|a| a.s.winmerge_path = path);
        crate::winmerge::forget_status();
        crate::save_session();
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == EN_CHANGE && id == S_KEYQ {
        let q = crate::text_of(ctl(h, S_KEYQ));
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.keyq = q;
            st.scroll = 0;
        });
        layout(h);
        let _ = InvalidateRect(Some(h), None, false);
        return;
    }
    if code == CBN_SELCHANGE && id == S_LANG {
        let sel = SendMessageW(ctl(h, S_LANG), 0x0147, None, None).0 as usize;
        let lang = crate::settings_languages().get(sel).map(|(l, _)| *l).unwrap_or(Lang::Auto);
        if app(|a| a.s.lang) != lang {
            crate::set_language(lang);
        }
        return;
    }
    if code == CBN_SELCHANGE {
        let sel = SendMessageW(ctl(h, id), 0x0147, None, None).0 as usize;
        if combo_pick(h, id, sel) {
            return;
        }
    }
    if code != 0 {
        return;
    }
    match id {
        S_CLOSE => close(),
        S_RESET => reset_keys(h),
        S_SESS => {
            let d = crate::sessions_dir();
            let _ = std::fs::create_dir_all(&d);
            let p = HSTRING::from(d.to_string_lossy().as_ref());
            ShellExecuteW(Some(h), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ACCENT => {
            crate::pick_accent();
            crate::save_session();
            redraw();
        }
        S_GITHUB => {
            ShellExecuteW(Some(h), w!("open"), &HSTRING::from(crate::HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ADV_OPEN => {
            let d = crate::store::config_dir();
            let _ = std::fs::create_dir_all(&d);
            let p = HSTRING::from(d.to_string_lossy().as_ref());
            ShellExecuteW(Some(h), w!("open"), &p, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        }
        S_ADV_EX => export_settings(h),
        S_ADV_IM => import_settings(h),
        S_ADV_RESET => reset_ini(h),
        S_FV_DEF => crate::set_default_folder_view(),
        S_FV_RESET => crate::reset_active_folder_view(),
        S_CTX_RESET => {
            app(|a| {
                let s = &mut a.s;
                crate::shell::ctxmenu::reset_menu(&mut s.ctx_off, &mut s.ctx_on);
                s.ctx_packaged_off.clear();
            });
            crate::save_session();
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
        S_CTX_REFRESH => {
            crate::shell::ctxmenu::invalidate();
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
        S_FV_CLEAR => {
            let q = tx(crate::Key::ForgetSavedViewOfEveryFolderDefault);
            if confirm_box(h, q) {
                crate::clear_remembered_folder_views();
            }
        }
        S_EV => {
            if crate::everything::busy() {
                return;
            }
            match crate::everything::action() {
                Some(crate::everything::EvAction::Start) => crate::everything::start_async(),
                Some(crate::everything::EvAction::Install) => crate::everything::install_async(),
                None => {}
            }
        }
        S_WM_BROWSE => {
            let cur = app(|a| a.s.winmerge_path.clone());
            let Some(path) = crate::winmerge::pick_exe(h, &cur) else { return };
            app(|a| a.s.winmerge_path = path.clone());
            crate::winmerge::forget_status();
            BUSY.with(|b| b.set(true));
            let _ = SetWindowTextW(ctl(h, S_WM), &HSTRING::from(&path));
            BUSY.with(|b| b.set(false));
            crate::save_session();
            let _ = InvalidateRect(Some(h), None, false);
        }
        id if (S_FAV0..S_REN0).contains(&id) => delete_fav((id - S_FAV0) as usize),
        id if (S_REN0..S_REN0 + FAV_MAX).contains(&id) => rename_fav(h, (id - S_REN0) as usize),
        _ => {}
    }
}

fn combo_pick(h: HWND, id: i32, sel: usize) -> bool {
    match id {
        S_THEME => {
            app(|a| a.s.theme = sel.min(2) as u8);
            unsafe {
                crate::apply_theme();
                crate::save_session();
            }
        }
        S_FACE => {
            let name = if sel == 0 { String::new() } else { unsafe { combo_lb_text(h, S_FACE, sel) } };
            if sel != 0 && name.is_empty() {
                return true;
            }
            let changed = app(|a| {
                if a.s.font_face == name {
                    false
                } else {
                    a.s.font_face = name;
                    true
                }
            });
            if changed {
                unsafe {
                    crate::publish_shared();
                    crate::apply_font_all();
                    crate::save_session();
                    refresh();
                }
            }
        }
        S_VIEW => {
            let sel = sel.min(crate::EXPLORER_VIEWS.len() - 1);
            let (mode, icon, ..) = crate::EXPLORER_VIEWS[sel];
            app(|a| {
                a.s.view_new = mode;
                a.s.view_icon = crate::stored_view_icon(mode, icon);
            });
            unsafe { crate::save_session(); }
        }
        S_DIR => {
            if sel == 2 {
                let path = unsafe { crate::pick_folder(h) };
                match path {
                    Some(p) => app(|a| {
                        a.s.start_new = 2;
                        a.s.start_path = p;
                    }),
                    None => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            } else {
                app(|a| a.s.start_new = sel.min(1) as u8);
            }
            unsafe { crate::save_session() };
        }
        S_DEL => {
            app(|a| a.s.del = if sel == 1 { 1 } else { 0 });
            unsafe { crate::save_session() };
        }
        S_NEW => {
            if sel == 0 {
                app(|a| a.s.new_name.clear());
            } else {
                let cur = app(|a| if a.s.new_name.is_empty() { crate::default_new_name() } else { a.s.new_name.clone() });
                match unsafe { ask(h, tx(crate::Key::NewFileName), &cur) } {
                    Some(name) if !name.trim().is_empty() => app(|a| a.s.new_name = name),
                    _ => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            }
            unsafe {
                crate::save_session();
                refresh();
            }
        }
        S_DROP => {
            app(|a| a.s.dropm = sel.min(2) as u8);
            unsafe { crate::save_session() };
        }
        S_LAYOUT => {
            app(|a| a.s.layout_new = sel.min(3) as u8);
            unsafe { crate::save_session() };
        }
        S_SIDE => {
            let fav_top = sel != 1;
            let changed = app(|a| {
                if a.s.fav_top == fav_top {
                    false
                } else {
                    a.s.fav_top = fav_top;
                    true
                }
            });
            if changed {
                unsafe {
                    crate::publish_shared();
                    crate::save_session();
                    crate::across_windows(|| {
                        crate::relayout();
                        crate::redraw_all();
                    });
                    redraw();
                }
            }
        }
        S_START => {
            if sel == 1 {
                match unsafe { crate::pick_session(h, crate::tr(crate::Key::SessionOpenOnStartup)) } {
                    Some(p) => app(|a| {
                        a.s.boot = 1;
                        a.s.boot_file = p.to_string_lossy().into_owned();
                    }),
                    None => {
                        unsafe { refresh() };
                        return true;
                    }
                }
            } else {
                app(|a| a.s.boot = if sel >= 2 { 2 } else { 0 });
            }
            unsafe { crate::save_session() };
        }
        _ => return false,
    }
    true
}

unsafe fn export_settings(h: HWND) {
    let Some(path) = crate::pick_settings(h, true) else { return };
    crate::save_session();
    let text = crate::store::current_text();
    if crate::store::write_atomic(std::path::Path::new(&path), text.as_bytes()).is_err() {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(crate::Key::CouldNotExportSettingsFile)), w!("Panegrid"), MB_ICONWARNING);
    }
}

unsafe fn import_settings(h: HWND) {
    let Some(path) = crate::pick_settings(h, false) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(crate::Key::CouldNotReadThatFile)), w!("Panegrid"), MB_ICONWARNING);
        return;
    };
    let q = tx(crate::Key::ReplaceCurrentSettingsWithThisFile);
    if MessageBoxW(Some(h), &HSTRING::from(q), w!("Panegrid"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    if !crate::import_settings(&text) {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(crate::Key::ThatFileIsNotValidSettingsJson)), w!("Panegrid"), MB_ICONWARNING);
    }
}

unsafe fn reset_ini(h: HWND) {
    let q = tx(crate::Key::ResetEveryPreferenceItsDefaultLayoutFold);
    if MessageBoxW(Some(h), &HSTRING::from(q), w!("Panegrid"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    crate::reset_prefs();
}

unsafe fn rename_fav(h: HWND, k: usize) {
    let Some(row) = fav_node(k) else { return };
    let Some(name) = ask(h, tx(crate::Key::Rename2), &row.name) else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let key = (row.name.clone(), row.path.clone(), row.group);
    let done = app(|a| {
        a.s.ensure_fav_tree();
        let Some(id) = crate::locate(&a.s.tree, &row.id, &key) else { return false };
        crate::rename_at(&mut a.s.tree, &id, &name);
        a.s.sync_favs();
        true
    });
    if done {
        crate::commit_favs();
    }
}

unsafe fn reset_keys(h: HWND) {
    let q = tx(crate::Key::ResetEveryShortcutItsDefault);
    if !confirm_box(h, q) {
        return;
    }
    crate::keys::reset_all();
    crate::apply_hotkey(true);
    crate::save_session();
    refresh();
}

unsafe fn reset_one(i: usize) {
    let Some(chord) = crate::keys::default_chord(i) else { return };
    if crate::keys::same(i, &chord) {
        return;
    }
    if let Some(other) = crate::keys::conflict(i, chord) {
        if !confirm_swap(&crate::keys::format_chord(&chord), &crate::keys::action(other)) {
            return;
        }
    }
    crate::keys::rebind(i, chord);
    crate::apply_hotkey(true);
    crate::save_session();
    refresh();
}

fn key_click(g: &Geo, x: i32, y: i32) -> Option<(usize, u8)> {
    for item in &g.items {
        if let Item::KeyRow { i, rect, change, reset } = item {
            if *i == usize::MAX || !inside(*rect, x, y) {
                continue;
            }
            if inside(*change, x, y) {
                return Some((*i, 1));
            }
            if inside(*reset, x, y) {
                return Some((*i, 2));
            }
            return Some((*i, 0));
        }
    }
    None
}

pub(crate) unsafe fn confirm_swap(chord: &str, other: &str) -> bool {
    let q = tf(crate::Key::ShortcutSwap, &[("other", other), ("chord", chord)]);
    confirm_box(hwnd(), &q)
}

unsafe fn confirm_box(owner: HWND, q: &str) -> bool {
    crate::keys::set_modal(true);
    let ans = MessageBoxW(Some(owner), &HSTRING::from(q), w!("Panegrid"), MB_YESNO | MB_ICONWARNING);
    crate::keys::set_modal(false);
    ans == IDYES
}

unsafe fn on_click(h: HWND, x: i32, y: i32) {
    let sn = snap();
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let g = geo(rc, &sn);
    if let Some((i, kind)) = key_click(&g, x, y) {
        if kind == 1 {
            crate::keys::begin(i);
            let _ = InvalidateRect(Some(h), None, false);
            return;
        }
        if kind == 2 {
            reset_one(i);
            return;
        }
    }
    for i in 0..PAGE_N {
        if g.show[i] && inside(g.nav[i], x, y) {
            if i == CTX_PAGE {
                crate::shell::ctxmenu::invalidate();
            }
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.page = i;
                st.anchor = i;
                st.scroll = 0;
            });
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
            return;
        }
    }
    for item in &g.items {
        if let Item::FavRow { i, rect, rename, btn } = item {
            if *i != usize::MAX && inside(*rect, x, y) && !inside(*rename, x, y) && !inside(*btn, x, y) {
                if let Some(row) = fav_node(*i) {
                    ST.with(|s| s.borrow_mut().fav_drag = Some(row.id));
                    let _ = SetCapture(h);
                }
                return;
            }
        }
        if let Item::CtxSeg { classic, win11 } = item {
            let next = if inside(*classic, x, y) {
                Some(false)
            } else if inside(*win11, x, y) {
                Some(true)
            } else {
                None
            };
            if let Some(win) = next {
                let changed = app(|a| {
                    if a.s.ctx_win11_view == win {
                        false
                    } else {
                        a.s.ctx_win11_view = win;
                        true
                    }
                });
                if changed {
                    crate::save_session();
                    ST.with(|s| s.borrow_mut().scroll = 0);
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                }
                return;
            }
        }
        if let Item::CtxRow { id, ids, scopes, rect, mark, packaged, .. } = item {
            if inside(*rect, x, y) {
                if *packaged {
                    let id = id.clone();
                    app(|a| crate::shell::ctxpack::toggle(&mut a.s.ctx_packaged_off, &id));
                    crate::save_session();
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                    return;
                }
                if inside(*mark, x, y) {
                    let ids = ids.clone();
                    let scopes = scopes.clone();
                    app(|a| {
                        let s = &mut a.s;
                        crate::shell::ctxmenu::toggle_ids_master(&mut s.ctx_off, &mut s.ctx_on, &ids, &scopes);
                    });
                    crate::save_session();
                } else {
                    let id = id.clone();
                    ST.with(|s| {
                        let mut st = s.borrow_mut();
                        if let Some(at) = st.ctx_open.iter().position(|x| x == &id) {
                            st.ctx_open.remove(at);
                        } else {
                            st.ctx_open.push(id);
                        }
                    });
                }
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
                return;
            }
        }
        if let Item::CtxSub { ids, scopes, bit, rect, .. } = item {
            if inside(*rect, x, y) {
                let ids = ids.clone();
                let scopes = scopes.clone();
                let bit = *bit;
                app(|a| {
                    let s = &mut a.s;
                    crate::shell::ctxmenu::toggle_ids_target(&mut s.ctx_off, &mut s.ctx_on, &ids, &scopes, bit);
                });
                crate::save_session();
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
                return;
            }
        }
        if let Item::Card(c) = item {
            if let Ctl::Toggle(id) = c.spec.ctl {
                if c.spec.live && inside(c.ctl, x, y) {
                    flip(id);
                    return;
                }
            }
            if c.spec.ctl == Ctl::Accent {
                let colors = crate::pal().accents;
                for (i, r) in c.sw.iter().enumerate() {
                    if inside(*r, x, y) {
                        if let Some((_, col)) = colors.get(i) {
                            crate::set_accent(*col);
                            crate::save_session();
                        }
                        return;
                    }
                }
            }
        }
    }
    if g.has_bar && inside(g.track, x, y) {
        if inside(g.thumb, x, y) {
            ST.with(|s| s.borrow_mut().drag = Some(y - g.thumb.top));
            let _ = SetCapture(h);
        } else {
            let page = (g.content.bottom - g.content.top).max(1);
            let dir = if y < g.thumb.top { -1 } else { 1 };
            let max = g.scroll_max;
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.scroll = (st.scroll + dir * page).clamp(0, max);
            });
            layout(h);
            let _ = InvalidateRect(Some(h), None, false);
        }
    }
}

extern "system" fn settings_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| settings_proc_inner(h, msg, wp, lp)))
}
fn settings_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let dpi = GetDpiForWindow(h) as i32;
                ST.with(|s| {
                    let mut st = s.borrow_mut();
                    *st = St::default();
                    st.hwnd = h;
                    st.dpi = dpi.max(96);
                });
                rebuild_fonts();
                build_children(h);
                crate::note_side(h);
                crate::everything::bind_settings(h.0 as isize);
                crate::everything::probe_async();
                crate::winmerge::forget_status();
                refresh();
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
                m.ptMinTrackSize.x = sc(dpi, 780);
                m.ptMinTrackSize.y = sc(dpi, 520);
                return LRESULT(0);
            }
            WM_PAINT => paint(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_DRAWITEM => {
                draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC => {
                let dc = HDC(wp.0 as _);
                let hw = HWND(lp.0 as _);
                let off = !IsWindowEnabled(hw).as_bool();
                SetBkMode(dc, TRANSPARENT);
                SetBkColor(dc, crate::cref(crate::pal().card));
                SetTextColor(dc, crate::cref(if off { crate::pal().sub } else { crate::pal().text }));
                SetDCBrushColor(dc, crate::cref(crate::pal().card));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
            }
            WM_COMMAND => on_command(h, (wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32),
            WM_LBUTTONDOWN => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                on_click(h, x, y);
            }
            WM_LBUTTONDBLCLK => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                let sn = snap();
                let mut rc = RECT::default();
                let _ = GetClientRect(h, &mut rc);
                let g = geo(rc, &sn);
                if let Some((i, 0)) = key_click(&g, x, y) {
                    crate::keys::begin(i);
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSEMOVE => {
                let x = (lp.0 & 0xffff) as i16 as i32;
                let y = ((lp.0 >> 16) & 0xffff) as i16 as i32;
                if let Some(from) = ST.with(|s| s.borrow().fav_drag.clone()) {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let hit = g.items.iter().find_map(|it| match it {
                        Item::FavRow { i, rect, .. } if *i != usize::MAX && y >= rect.top && y < rect.bottom => Some((*i, *rect)),
                        _ => None,
                    });
                    if let Some((to_i, rect)) = hit {
                        let moved = app(|a| {
                            a.s.ensure_fav_tree();
                            let target = crate::tree_rows(&a.s.tree).get(to_i).map(|r| (r.id.clone(), r.group));
                            let Some((tid, group)) = target else { return None };
                            if tid == from {
                                return None;
                            }
                            let zone = crate::drop_zone(group, y - rect.top, (rect.bottom - rect.top).max(1));
                            let new_id = crate::apply_drop(&mut a.s.tree, &from, &tid, zone)?;
                            if new_id == from {
                                return None;
                            }
                            a.s.sync_favs();
                            Some(new_id)
                        });
                        if let Some(new_id) = moved {
                            ST.with(|s| s.borrow_mut().fav_drag = Some(new_id));
                            layout(h);
                            let _ = InvalidateRect(Some(h), None, false);
                        }
                    }
                    let _ = x;
                    return LRESULT(0);
                }
                if let Some(off) = ST.with(|s| s.borrow().drag) {
                    let (track, thumb_h, max) = ST.with(|s| {
                        let st = s.borrow();
                        (st.track, st.thumb_h, st.scroll_max)
                    });
                    let span = (track.bottom - track.top - thumb_h).max(1);
                    let top = (y - off).clamp(track.top, track.bottom - thumb_h);
                    let scroll = if max == 0 { 0 } else { (top - track.top) * max / span };
                    ST.with(|s| s.borrow_mut().scroll = scroll.clamp(0, max));
                    layout(h);
                    let _ = InvalidateRect(Some(h), None, false);
                } else {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let hot = (0..PAGE_N).find(|&i| g.show[i] && inside(g.nav[i], x, y));
                    let changed = ST.with(|s| {
                        let mut st = s.borrow_mut();
                        if st.hot != hot {
                            st.hot = hot;
                            true
                        } else {
                            false
                        }
                    });
                    if changed {
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                    let _ = TrackMouseEvent(&mut t);
                }
            }
            WM_LBUTTONUP => {
                let fav = ST.with(|s| s.borrow_mut().fav_drag.take());
                if ST.with(|s| s.borrow().drag.is_some()) || fav.is_some() {
                    ST.with(|s| s.borrow_mut().drag = None);
                    let _ = ReleaseCapture();
                }
                if fav.is_some() {
                    crate::commit_favs();
                }
            }
            WM_CAPTURECHANGED => {
                // Capture taken away mid-drag (Alt+Tab, a popup): end the drag as a button-up would.
                // A normal button-up has already cleared both, so this does nothing then.
                let fav = ST.with(|s| s.borrow_mut().fav_drag.take());
                ST.with(|s| s.borrow_mut().drag = None);
                if fav.is_some() {
                    crate::commit_favs();
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) as i16 as i32) / 120;
                if delta != 0 {
                    let dpi = ST.with(|s| s.borrow().dpi.max(96));
                    let (scroll, max) = ST.with(|s| {
                        let st = s.borrow();
                        (st.scroll, st.scroll_max)
                    });
                    let next = (scroll - delta * sc(dpi, 48)).clamp(0, max);
                    if next != scroll {
                        ST.with(|s| s.borrow_mut().scroll = next);
                        layout(h);
                        let _ = InvalidateRect(Some(h), None, false);
                    }
                }
            }
            WM_MOUSELEAVE => {
                ST.with(|s| s.borrow_mut().hot = None);
                let _ = InvalidateRect(Some(h), None, false);
            }
            crate::WM_EV => {
                layout(h);
                let _ = InvalidateRect(Some(h), None, false);
            }
            crate::WM_FAV_ICON => {
                crate::clear_icon_refresh();
                if ST.with(|s| s.borrow().page) == 4 {
                    let _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_CLOSE => {
                let _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                crate::drop_side(h);
                crate::everything::bind_settings(0);
                crate::keys::cancel();
                let old = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let old = (st.font, st.small, st.bold, st.title_font, st.mono, st.icon);
                    *st = St::default(); // drops the installed-family list
                    old
                });
                for f in [old.0, old.1, old.2, old.3, old.4, old.5] {
                    if !f.is_invalid() {
                        let _ = DeleteObject(f.into());
                    }
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

thread_local! {
    static ASK_EDIT: Cell<HWND> = Cell::new(HWND(std::ptr::null_mut()));
    static ASK_OUT: RefCell<Option<Option<String>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn ask_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| unsafe { ask_proc_inner(h, msg, wp, lp) }))
}
unsafe fn ask_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint_ask(h);
            return LRESULT(0);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
            let dc = HDC(wp.0 as _);
            SetBkMode(dc, TRANSPARENT);
            SetBkColor(dc, crate::cref(crate::pal().card));
            SetTextColor(dc, crate::cref(crate::pal().text));
            SetDCBrushColor(dc, crate::cref(crate::pal().card));
            return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
        }
        WM_DRAWITEM => {
            draw_btn(&*(lp.0 as *const DRAWITEMSTRUCT));
            return LRESULT(1);
        }
        WM_COMMAND => {
            let id = (wp.0 & 0xffff) as i32;
            if id == 1 {
                let text = crate::text_of(ASK_EDIT.with(|e| e.get()));
                ASK_OUT.with(|o| *o.borrow_mut() = Some(Some(text)));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
            if id == 2 {
                ASK_OUT.with(|o| *o.borrow_mut() = Some(None));
                let _ = DestroyWindow(h);
                return LRESULT(0);
            }
        }
        WM_CLOSE => {
            ASK_OUT.with(|o| *o.borrow_mut() = Some(None));
            let _ = DestroyWindow(h);
            return LRESULT(0);
        }
        WM_DPICHANGED => {
            let r = &*(lp.0 as *const RECT);
            let _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            let font = ask_font();
            if !font.is_invalid() {
                for id in [1, 2, 10] {
                    SendMessageW(ctl(h, id), WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                }
            }
            layout_ask(h);
            return LRESULT(0);
        }
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

fn ask_font() -> HFONT {
    let font = ST.with(|s| s.borrow().font);
    if font.is_invalid() { app(|a| a.font) } else { font }
}

fn ask_style() -> WINDOW_STYLE {
    WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_CLIPCHILDREN
}

const ASK_MARGIN_DIP: i32 = 16;
const ASK_GAP_DIP: i32 = 16;
const ASK_BTN_W_DIP: i32 = 80;
const ASK_BTN_H_DIP: i32 = 28;
const ASK_BTN_GAP_DIP: i32 = 12;

/// Client height that puts the same margin above the field and below the buttons.
fn ask_client_h(dpi: i32) -> i32 {
    let dpi = dpi.max(96);
    search_frame_h(dpi) + sc(dpi, ASK_GAP_DIP) + sc(dpi, ASK_BTN_H_DIP) + 2 * sc(dpi, ASK_MARGIN_DIP)
}

/// Outer size. Width stays 420 DIP. Height grows so the client fits `ask_client_h` under the caption.
fn ask_outer(dpi: i32) -> (i32, i32) {
    let dpi = dpi.max(96);
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: ask_client_h(dpi) };
    let h = unsafe {
        if AdjustWindowRectExForDpi(&mut r, ask_style(), false, WS_EX_DLGMODALFRAME, dpi as u32).is_ok() {
            (r.bottom - r.top).max(ask_client_h(dpi))
        } else {
            ask_client_h(dpi) + sc(dpi, 48)
        }
    };
    (sc(dpi, 420), h)
}

struct AskLay {
    field: RECT,
    edit: RECT,
    ok: RECT,
    cancel: RECT,
}

/// Field, edit slot and buttons inside the client, under the caption title.
/// The field is the settings search frame. The edit slot is that frame inset by the same 2 DIP,
/// and `place_edit` centres the line in the slot with `centered_edit_rect`.
fn ask_lay(client_w: i32, client_h: i32, dpi: i32) -> AskLay {
    let dpi = dpi.max(96);
    let s = |v: i32| sc(dpi, v);
    let margin = s(ASK_MARGIN_DIP);
    let gap = s(ASK_GAP_DIP);
    let btn_w = s(ASK_BTN_W_DIP);
    let btn_h = s(ASK_BTN_H_DIP);
    let btn_gap = s(ASK_BTN_GAP_DIP);
    let frame_h = search_frame_h(dpi).max(1);
    let cw = client_w.max(1);
    let ch = client_h.max(1);
    let block = frame_h + gap + btn_h;
    let top = ((ch - block) / 2).max(0);
    let left = margin.min(cw / 2);
    let right = (cw - margin).max(left + 1);
    let field = RECT { left, top, right, bottom: top + frame_h };
    let edit = inset(field, s(FIELD_INSET_DIP).min(frame_h / 2));
    let by = field.bottom + gap;
    let cancel = RECT { left: (right - btn_w).max(left), top: by, right, bottom: by + btn_h };
    let ok_right = (cancel.left - btn_gap).max(left);
    let ok = RECT { left: (ok_right - btn_w).max(0), top: by, right: ok_right, bottom: by + btn_h };
    AskLay { field, edit, ok, cancel }
}

unsafe fn paint_ask(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    crate::buffered(h, |dc, rc| {
        crate::fill(dc, rc, crate::pal().card);
        let lay = ask_lay(rc.right - rc.left, rc.bottom - rc.top, dpi);
        draw_field(dc, lay.field, dpi);
    });
}

/// Place the borderless edit in the search-height frame, and the buttons under it.
unsafe fn layout_ask(h: HWND) {
    let dpi = GetDpiForWindow(h).max(96) as i32;
    let mut rc = RECT::default();
    let _ = GetClientRect(h, &mut rc);
    let lay = ask_lay(rc.right - rc.left, rc.bottom - rc.top, dpi);
    let edit = ASK_EDIT.with(|e| e.get());
    let m = sc(dpi, 8) as isize;
    SendMessageW(edit, 0x00D3, Some(WPARAM(3)), Some(LPARAM(m | (m << 16)))); // EM_SETMARGINS, same as the search box
    crate::place_edit(edit, lay.edit, true);
    move_to(ctl(h, 1), lay.ok, true);
    move_to(ctl(h, 2), lay.cancel, true);
    let _ = InvalidateRect(Some(h), None, false);
}

/// One-line prompt. None means cancel.
pub(crate) unsafe fn ask(owner: HWND, title: &str, initial: &str) -> Option<String> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(ask_proc),
            hInstance: m.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("PanegridAsk"),
            ..Default::default()
        });
    });
    ASK_OUT.with(|o| *o.borrow_mut() = None);
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = ask_outer(dpi);
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    // Shown only after the dark caption attributes are set, or the first frame stays light.
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("PanegridAsk"), &HSTRING::from(title), ask_style(), x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return None;
    }
    style_window(h);
    let font = ask_font();
    let edit = child_at(h, w!("EDIT"), initial, 0x80 | 0x00010000, 10, font);
    theme_ctl(edit);
    ASK_EDIT.with(|e| e.set(edit));
    let _ = child_at(h, w!("BUTTON"), tx(crate::Key::Ok), 0xB | 0x00010000, 1, font);
    let _ = child_at(h, w!("BUTTON"), tx(CLOSE_L), 0xB | 0x00010000, 2, font);
    layout_ask(h);
    let _ = ShowWindow(h, SW_SHOW);
    crate::keys::set_modal(true);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
    let mut msg = MSG::default();
    while ASK_OUT.with(|o| o.borrow().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        // No dialog manager: Enter and Esc never become IDOK / IDCANCEL on their own.
        let mine = msg.hwnd == h || IsChild(h, msg.hwnd).as_bool();
        if msg.message == WM_KEYDOWN && mine {
            let cmd: usize = if msg.wParam.0 == VK_RETURN.0 as usize {
                1
            } else if msg.wParam.0 == VK_ESCAPE.0 as usize {
                2
            } else {
                0
            };
            if cmd != 0 {
                let _ = SendMessageW(h, WM_COMMAND, Some(WPARAM(cmd)), Some(LPARAM(0)));
                continue;
            }
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    crate::keys::set_modal(false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, true);
    let _ = SetForegroundWindow(owner);
    ASK_OUT.with(|o| o.borrow_mut().take().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    

    fn span(line: &[i32], gap: i32) -> i32 {
        if line.is_empty() {
            0
        } else {
            line.iter().sum::<i32>() + gap * (line.len() as i32 - 1)
        }
    }

    fn column_w(client: i32, bar: bool) -> i32 {
        let nav = sc(96, 220);
        let pad = sc(96, 28);
        let right_pad = if bar { sc(96, 18) } else { 0 };
        let row = RECT { left: nav + pad, top: 0, right: client - pad - right_pad, bottom: 0 };
        chord_box(row, 96).1
    }

    #[test]
    fn shortcut_chords_wrap_inside_the_cell() {
        let gap = chord_gap(96);
        // Four Alt+Shift arrow chips, about 110px each in Consolas 12.
        let arrows = [110, 110, 110, 110];
        for (client, bar) in [(1000, false), (1000, true), (780, true)] {
            let max_w = column_w(client, bar);
            let lines = pack_chords(&arrows, max_w, gap);
            assert!(lines.len() >= 2, "client {client} bar {bar} max {max_w} stayed one line");
            assert_eq!(lines.iter().map(|l| l.len()).sum::<usize>(), 4);
            for line in &lines {
                assert!(span(line, gap) <= max_w, "line {line:?} exceeds {max_w}");
            }
            assert!(key_row_h_from_lines(lines.len() as i32, 96) > sc(96, 40));
        }
        // A chord wider than the cell is clamped to the cell.
        assert_eq!(pack_chords(&[900], 200, gap), vec![vec![200]]);
        // Short chords stay on one line and keep the 40px row.
        assert_eq!(pack_chords(&[70, 60, 40], column_w(1000, false), gap).len(), 1);
        assert_eq!(key_row_h_from_lines(1, 96), sc(96, 40));
        assert!(key_row_h_from_lines(2, 96) > key_row_h_from_lines(1, 96));
    }

    #[test]
    fn ask_field_matches_the_settings_search_frame() {
        // The painted search rect is top 12, bottom 44: 32px at 96 DPI, not sc(32) at every DPI.
        assert_eq!(search_frame_h(96), 32);
        assert_eq!(search_frame_h(191), sc(191, 44) - sc(191, 12));
        assert_ne!(search_frame_h(191), sc(191, 32));
        for dpi in [96, 120, 144, 191, 192] {
            let cw = sc(dpi, 388);
            let ch = ask_client_h(dpi);
            let lay = ask_lay(cw, ch, dpi);
            let frame_h = lay.field.bottom - lay.field.top;
            let inset_d = sc(dpi, FIELD_INSET_DIP);
            assert_eq!(frame_h, search_frame_h(dpi));
            assert_eq!(frame_h, sc(dpi, 44) - sc(dpi, 12));
            assert!(frame_h > sc(dpi, 16), "frame stays taller than one text line");
            assert_eq!(lay.edit.top, lay.field.top + inset_d);
            assert_eq!(lay.edit.bottom, lay.field.bottom - inset_d);
            assert_eq!((lay.edit.left, lay.edit.right), (lay.field.left + inset_d, lay.field.right - inset_d));
            // Same centring the borderless edit gets from place_edit / centered_edit_rect.
            for px in [15, 16, 19] {
                let line = (px * dpi / 96).max(1);
                let text = crate::centered_edit_rect(lay.edit, line);
                let slot_h = lay.edit.bottom - lay.edit.top;
                let used = line.min(slot_h);
                assert_eq!(text.bottom - text.top, used);
                assert_eq!(text.top, lay.edit.top + (slot_h - used) / 2);
                assert!(text.top >= lay.field.top && text.bottom <= lay.field.bottom);
                assert!(text.bottom - text.top < frame_h);
            }
            let above = lay.field.top;
            let below = ch - lay.ok.bottom;
            assert_eq!(above, below);
            assert_eq!(above, sc(dpi, ASK_MARGIN_DIP));
            assert_eq!(lay.ok.top - lay.field.bottom, sc(dpi, ASK_GAP_DIP));
            assert_eq!(lay.cancel.top, lay.ok.top);
            assert_eq!(lay.cancel.bottom - lay.cancel.top, sc(dpi, ASK_BTN_H_DIP));
            assert_eq!(lay.ok.bottom - lay.ok.top, sc(dpi, ASK_BTN_H_DIP));
            assert_eq!(lay.cancel.left - lay.ok.right, sc(dpi, ASK_BTN_GAP_DIP));
            assert_eq!(lay.cancel.right, lay.field.right);
            assert_eq!(lay.field.left, sc(dpi, ASK_MARGIN_DIP));
            assert_eq!(cw - lay.field.right, sc(dpi, ASK_MARGIN_DIP));
            assert!(lay.ok.left >= lay.field.left);
            assert!(lay.ok.bottom <= ch && lay.field.top >= 0);
            assert!(lay.ok.top >= lay.field.bottom);
        }
    }

    fn point_in_capsule(x: f32, y: f32, w: f32, h: f32) -> bool {
        let rad = h / 2.0;
        if y < 0.0 || y > h {
            return false;
        }
        if x >= rad && x <= w - rad {
            return true;
        }
        let cx = if x < rad { rad } else { w - rad };
        let (dx, dy) = (x - cx, y - rad);
        dx * dx + dy * dy <= rad * rad + 0.01
    }

    #[test]
    fn toggle_knob_stays_inside_the_capsule_at_96_120_144() {
        for dpi in [96, 120, 144] {
            let w = sc(dpi, 40);
            let h = sc(dpi, 20);
            let r = RECT { left: 0, top: 0, right: w, bottom: h };
            for on in [false, true] {
                let k = toggle_knob_rect(r, on, dpi);
                assert_eq!(k.right - k.left, sc(dpi, 12));
                assert_eq!(k.bottom - k.top, sc(dpi, 12));
                assert!(k.left >= r.left && k.right <= r.right && k.top >= r.top && k.bottom <= r.bottom);
                for (x, y) in [(k.left, k.top), (k.right - 1, k.top), (k.left, k.bottom - 1), (k.right - 1, k.bottom - 1)] {
                    assert!(point_in_capsule(x as f32 + 0.5, y as f32 + 0.5, w as f32, h as f32), "dpi {dpi} on {on} at {x},{y}");
                }
            }
            // The blit pad stays on the card (16 DIP) and inside the segment gap (8 DIP).
            assert!(AA_PAD < sc(dpi, 16));
            assert!(AA_PAD * 2 < sc(dpi, 8));
        }
    }

    #[test]
    fn box_down_keeps_a_flat_fill_and_blends_an_edge() {
        let scale = 3usize;
        let (sw, sh) = (6usize, 3usize);
        let mut src = vec![0u8; sw * sh * 4];
        for y in 0..sh {
            for x in 0..sw {
                let i = (y * sw + x) * 4;
                if x < 3 {
                    src[i + 2] = 255;
                } else {
                    src[i] = 255;
                }
                src[i + 3] = 255;
            }
        }
        let mut dst = vec![0u8; 8];
        box_down(&src, sw, &mut dst, 2, 1, scale);
        assert_eq!(&dst[0..4], &[0, 0, 255, 255]);
        assert_eq!(&dst[4..8], &[255, 0, 0, 255]);
        for y in 0..sh {
            let i = (y * sw + 3) * 4;
            src[i] = 0;
            src[i + 2] = 255;
        }
        box_down(&src, sw, &mut dst, 2, 1, scale);
        assert_eq!(dst[4], (255u32 * 6 / 9) as u8);
        assert_eq!(dst[5], 0);
        assert_eq!(dst[6], (255u32 * 3 / 9) as u8);
        assert_eq!(dst[7], 255);
    }
}
