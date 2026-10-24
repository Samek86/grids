use std::cell::{Cell, RefCell};
use std::sync::atomic::Ordering;
use std::sync::Once;
use windows::core::{w, BOOL, HSTRING, PCWSTR};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWINDOWATTRIBUTE};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetWindowTheme, DRAWITEMSTRUCT, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, IsWindowEnabled, ReleaseCapture, SetCapture, TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::*;

pub unsafe fn open(owner: HWND) {
    ensure_class();
    let h = hwnd();
    if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
        let _ = ShowWindow(h, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(h);
        return;
    }
    let dpi = GetDpiForWindow(owner) as i32;
    let (ww, hh) = (sc(dpi.max(96), 1000), sc(dpi.max(96), 640));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE;
    let _ = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsSettings"), w!("grids"), style, x, y, ww, hh, Some(owner), None, None, None);
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
    cue(h, S_KEYQ, tx(s("Search actions or keys", "동작 또는 키 검색", "操作またはキーを検索")));
    layout(h);
    let _ = InvalidateRect(Some(h), None, true);
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
            lpszClassName: w!("GridsSettings"),
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
        match crate::ui() {
            1 => ("Malgun Gothic", 700), // no semibold; bold is the Korean title face
            2 => ("Yu Gothic UI", 700),
            _ => ("Segoe UI Semibold", 600),
        }
    } else {
        (face, 700)
    };
    let new = (
        make_font(face, 14, 400, dpi),
        make_font(face, 12, 400, dpi),
        make_font(title_face, 24, title_w, dpi),
        make_font("Consolas", 12, 400, dpi),
        make_font("Segoe MDL2 Assets", 14, 400, dpi),
    );
    let old = ST.with(|s| {
        let mut st = s.borrow_mut();
        let old = (st.font, st.small, st.title_font, st.mono, st.icon);
        (st.font, st.small, st.title_font, st.mono, st.icon) = new;
        old
    });
    unsafe {
        for f in [old.0, old.1, old.2, old.3, old.4] {
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
    for id in [S_START, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT] {
        combo(id, true);
    }
    let btn = |id, label: &str, on| child_at(h, w!("BUTTON"), label, 0xB | tab | if on { 0 } else { dis }, id, font);
    btn(S_CLOSE, tx(CLOSE_L), true);
    btn(S_RESET, tx(s("Reset all to defaults", "기본값으로 모두 되돌리기", "すべて既定に戻す")), true);
    btn(S_SESS, tx(s("Open folder", "폴더 열기", "フォルダーを開く")), true);
    btn(S_ACCENT, tx(s("Custom…", "직접 지정…", "直接指定…")), true);
    btn(S_GITHUB, tx(s("Open GitHub", "GitHub 열기", "GitHub を開く")), true);
    btn(S_ADV_OPEN, tx(s("Open folder", "폴더 열기", "フォルダーを開く")), true);
    btn(S_ADV_EX, tx(s("Export", "내보내기", "エクスポート")), true);
    btn(S_ADV_IM, tx(s("Import", "가져오기", "インポート")), true);
    btn(S_ADV_RESET, tx(s("Reset…", "초기화…", "初期化…")), true);
    btn(S_EV, ev_button_label(), true);
}

fn each_child(h: HWND, mut f: impl FnMut(HWND)) {
    let ids = [S_SEARCH, S_CLOSE, S_KEYQ, S_RESET, S_LANG, S_FACE, S_START, S_THEME, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SESS, S_ACCENT, S_GITHUB, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET, S_EV];
    for id in ids {
        let c = unsafe { GetDlgItem(Some(h), id).unwrap_or_default() };
        if !c.0.is_null() {
            f(c);
        }
    }
    for i in 0..256 {
        let c = unsafe { GetDlgItem(Some(h), S_FAV0 + i).unwrap_or_default() };
        if c.0.is_null() {
            continue;
        }
        f(c);
    }
}

unsafe fn fill_strings(h: HWND) {
    let auto = {
        let name = match Lang::Auto.resolve() {
            1 => "한국어",
            2 => "日本語",
            _ => "English",
        };
        format!("{} ({name})", tx(s("Follow Windows", "Windows 설정 따르기", "Windows の設定に従う")))
    };
    let lang = app(|a| a.s.lang);
    let lang_i = match lang {
        Lang::Auto => 0,
        Lang::En => 1,
        Lang::Ko => 2,
        Lang::Ja => 3,
    };
    fill_combo(h, S_LANG, &[auto, "English".into(), "한국어".into(), "日本語".into()], lang_i);
    fill_face(h);
    let theme = app(|a| a.s.theme) as usize;
    fill_combo(h, S_THEME, &[tx(s("Follow Windows", "Windows 설정 따르기", "Windows の設定に従う")).into(), tx(s("Light", "라이트", "ライト")).into(), tx(s("Dark", "다크", "ダーク")).into()], theme.min(2));
    let boot = app(|a| a.s.boot) as usize;
    fill_combo(h, S_START, &[tx(s("Restore last state", "마지막 상태 복원", "前回の状態を復元")).into(), tx(s("Open a session…", "세션 열기…", "セッションを開く…")).into(), tx(s("Default layout", "기본 레이아웃", "既定のレイアウト")).into()], boot.min(2));
    let view_i = match app(|a| a.s.view_new) { 3 => 1, 1 => 2, _ => 0 };
    fill_combo(h, S_VIEW, &[tx(s("Details", "자세히", "詳細")).into(), tx(s("List", "목록", "一覧")).into(), tx(s("Large icons", "큰 아이콘", "大きなアイコン")).into()], view_i);
    let start = app(|a| a.s.start_new) as usize;
    fill_combo(h, S_DIR, &[tx(s("This PC", "내 PC", "PC")).into(), tx(s("Desktop", "바탕 화면", "デスクトップ")).into(), tx(s("Choose a folder…", "지정 폴더…", "フォルダーを指定…")).into()], start.min(2));
    let del = app(|a| a.s.del) as usize;
    fill_combo(h, S_DEL, &[tx(s("Recycle Bin", "휴지통으로", "ごみ箱へ")).into(), tx(s("Delete permanently", "완전 삭제", "完全に削除")).into()], del.min(1));
    let custom = app(|a| a.s.new_name.clone());
    let new_label = if custom.is_empty() { tx(s("Custom…", "직접 지정…", "直接指定…")).to_string() } else { custom };
    let new_i = usize::from(!app(|a| a.s.new_name.is_empty()));
    fill_combo(h, S_NEW, &[tx(s("New Text Document.txt", "새 텍스트 문서.txt", "新しいテキスト ドキュメント.txt")).into(), new_label], new_i);
    let dropm = app(|a| a.s.dropm) as usize;
    fill_combo(h, S_DROP, &[tx(s("Windows rules (same drive = move)", "Windows 규칙(같은 드라이브=이동)", "Windows の規則（同じドライブ=移動）")).into(), tx(s("Always copy", "항상 복사", "常にコピー")).into(), tx(s("Always ask", "항상 묻기", "常に確認")).into()], dropm.min(2));
    let layout_new = app(|a| a.s.layout_new) as usize;
    fill_combo(h, S_LAYOUT, &[tx(s("Two panes, side by side", "좌우 2개", "左右に2つ")).into(), tx(s("Two panes, top and bottom", "위아래 2개", "上下に2つ")).into(), tx(s("2×2 (four panes)", "2×2 (4개)", "2×2（4つ）")).into(), tx(s("One pane", "1개", "1つ")).into()], layout_new.min(3));
    let labels = [
        (S_CLOSE, tx(CLOSE_L)),
        (S_RESET, tx(s("Reset all to defaults", "기본값으로 모두 되돌리기", "すべて既定に戻す"))),
        (S_SESS, tx(s("Open folder", "폴더 열기", "フォルダーを開く"))),
        (S_ACCENT, tx(s("Custom…", "직접 지정…", "直接指定…"))),
        (S_GITHUB, tx(s("Open GitHub", "GitHub 열기", "GitHub を開く"))),
        (S_ADV_OPEN, tx(s("Open folder", "폴더 열기", "フォルダーを開く"))),
        (S_ADV_EX, tx(s("Export", "내보내기", "エクスポート"))),
        (S_ADV_IM, tx(s("Import", "가져오기", "インポート"))),
        (S_ADV_RESET, tx(s("Reset…", "초기화…", "初期化…"))),
        (S_EV, ev_button_label()),
    ];
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
    let def = format!("{} ({})", tx(s("Default", "기본", "既定")), crate::ui_face());
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
    for id in [S_KEYQ, S_RESET, S_LANG, S_FACE, S_START, S_THEME, S_VIEW, S_DIR, S_DEL, S_NEW, S_DROP, S_LAYOUT, S_SESS, S_ACCENT, S_GITHUB, S_ADV_OPEN, S_ADV_EX, S_ADV_IM, S_ADV_RESET, S_EV] {
        hide(id);
    }
    let n = app(|a| a.s.favs.len()) as i32;
    for base in [S_FAV0, S_FAV0 + 128] {
        for i in 0..128 {
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
    move_to(ctl(h, S_SEARCH), inset(g.search, sc(sn.dpi, 2)), true);
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
                if c.spec.ctl == Ctl::AdvReset {
                    move_to(ctl(h, S_ADV_RESET), c.adv[0], intersects(c.adv[0], content));
                }
                if c.spec.ctl == Ctl::EvAct {
                    let b = ctl(h, S_EV);
                    let _ = SetWindowTextW(b, &HSTRING::from(ev_button_label()));
                    let _ = EnableWindow(b, !crate::everything::busy());
                    move_to(b, c.ctl, crate::everything::action().is_some() && intersects(c.ctl, content));
                }
            }
            Item::KeyBar { edit, reset } => {
                move_to(ctl(h, S_KEYQ), inset(*edit, sc(sn.dpi, 2)), intersects(*edit, content));
                move_to(ctl(h, S_RESET), *reset, intersects(*reset, content));
            }
            Item::FavRow { i, rename, btn, .. } if *i != usize::MAX && btn.right > btn.left => {
                let id = S_FAV0 + *i as i32;
                let mut b = ctl(h, id);
                if b.0.is_null() {
                    b = child_at(h, w!("BUTTON"), tx(s("Delete", "삭제", "削除")), 0xB | 0x00010000, id, sn.font);
                }
                let _ = SetWindowTextW(b, &HSTRING::from(tx(s("Delete", "삭제", "削除"))));
                move_to(b, *btn, intersects(*btn, content));
                let rid = S_FAV0 + 128 + *i as i32;
                let mut r = ctl(h, rid);
                if r.0.is_null() {
                    r = child_at(h, w!("BUTTON"), tx(s("Rename", "이름 변경", "名前の変更")), 0xB | 0x00010000, rid, sn.font);
                }
                let _ = SetWindowTextW(r, &HSTRING::from(tx(s("Rename", "이름 변경", "名前の変更"))));
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
    rfill(dc, g.search, sc(dpi, 4), crate::pal().card, Some(crate::pal().line));
    if !g.show.iter().any(|s| *s) {
        crate::text(dc, RECT { left: sc(dpi, 16), top: g.search.bottom + sc(dpi, 12), right: g.nav_w - sc(dpi, 8), bottom: g.search.bottom + sc(dpi, 40) }, tx(NONE), sn.font, crate::pal().sub, DT_LEFT);
    }
    for i in 0..9 {
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
    let title = tx(PAGES[sn.page]);
    crate::text(dc, g.title, title, sn.title_font, crate::pal().text, DT_LEFT);
    for item in &g.items {
        match item {
            Item::Card(c) => draw_card(dc, c, sn, accent),
            Item::Head(r, t) => crate::text(dc, *r, tx(*t), sn.font, crate::pal().text, DT_LEFT),
            Item::Table(r) => rfill(dc, *r, sc(dpi, 6), crate::pal().card, Some(crate::pal().line)),
            Item::FavHead(r) => {
                crate::text(dc, RECT { left: r.left + sc(dpi, 40), right: r.left + (r.right - r.left) * 32 / 100, ..*r }, tx(s("Name", "이름", "名前")), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: r.left + (r.right - r.left) * 32 / 100, ..*r }, tx(s("Path", "경로", "パス")), sn.small, crate::pal().sub, DT_LEFT);
                crate::fill(dc, RECT { top: r.bottom - 1, ..*r }, crate::pal().line);
            }
            Item::FavRow { i, rect, .. } => draw_fav_row(dc, *i, *rect, sn),
            Item::Note(r, t) => crate::text(dc, *r, tx(*t), sn.small, crate::pal().sub, DT_LEFT),
            Item::KeyBar { edit, .. } => rfill(dc, *edit, sc(dpi, 4), crate::pal().card, Some(crate::pal().line)),
            Item::KeyHead(r) => {
                let (a, b) = key_cols(*r);
                crate::text(dc, a, tx(s("Category", "분류", "分類")), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: a.right, right: b, ..*r }, tx(s("Action", "동작", "操作")), sn.small, crate::pal().sub, DT_LEFT);
                crate::text(dc, RECT { left: b, ..*r }, tx(s("Shortcut", "단축키", "ショートカット")), sn.small, crate::pal().sub, DT_LEFT);
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
    let color = if c.spec.live { crate::pal().text } else { crate::pal().sub };
    let title = if c.spec.ctl == Ctl::About {
        format!("grids v{}", env!("CARGO_PKG_VERSION"))
    } else {
        tx(c.spec.title).to_string()
    };
    let text_r = RECT { left: c.rect.left + sc(dpi, 16), top: c.rect.top + sc(dpi, 12), right: c.ctl.left.min(c.sw.first().map(|s| s.left).unwrap_or(i32::MAX)) - sc(dpi, 12), bottom: c.rect.bottom - sc(dpi, 12) };
    let title_line = if c.spec.desc[0].is_empty() { text_r } else { RECT { bottom: text_r.top + sc(dpi, 20), ..text_r } };
    crate::text(dc, title_line, &title, sn.font, color, DT_LEFT);
    if c.spec.badge {
        let bw = text_w(dc, sn.small, tx(NEW)) + sc(dpi, 10);
        let tw = text_w(dc, sn.font, &title);
        let br = RECT { left: title_line.left + tw + sc(dpi, 8), top: (title_line.top + title_line.bottom) / 2 - sc(dpi, 8), right: title_line.left + tw + sc(dpi, 8) + bw, bottom: (title_line.top + title_line.bottom) / 2 + sc(dpi, 8) };
        if br.right < text_r.right {
            rfill(dc, br, sc(dpi, 3), accent, None);
            crate::text(dc, br, tx(NEW), sn.small, on_chrome(accent), DT_CENTER);
        }
    }
    if c.spec.ctl == Ctl::EvStatus {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: title_line.bottom + line, ..text_r };
        crate::text(dc, desc_r, &ev_status_line(), sn.small, crate::pal().text, DT_LEFT);
    } else if c.spec.ctl == Ctl::EvAct {
        let line = sc(dpi, 18);
        let desc_r = RECT { top: title_line.bottom, bottom: (title_line.bottom + line).min(text_r.bottom), ..text_r };
        let act = ev_activity_line();
        let text = if !act.is_empty() {
            act
        } else if matches!(crate::everything::shown(), crate::everything::Shown::Ready(crate::everything::Presence::Running { .. })) {
            tx(s("Everything is already running.", "Everything이 이미 실행 중입니다.", "Everything はすでに実行中です。")).to_string()
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
    } else if !c.spec.desc[0].is_empty() {
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

unsafe fn draw_toggle(dc: HDC, r: RECT, on: bool, enabled: bool, accent: u32, dpi: i32) {
    // 40×20 pill. Off and disabled stay an outlined gray track on the white card.
    let (fill, border, knob) = if !enabled || !on {
        (crate::pal().card, crate::pal().sub, crate::pal().sub)
    } else {
        (accent, accent, on_chrome(accent))
    };
    let rad = (r.bottom - r.top).max(1);
    rfill(dc, r, rad, fill, Some(border));
    let k = sc(dpi, 12);
    let pad = sc(dpi, 3);
    let x = if on { r.right - pad - k } else { r.left + pad };
    let y = r.top + (r.bottom - r.top - k) / 2;
    ellipse(dc, RECT { left: x, top: y, right: x + k, bottom: y + k }, knob, None);
}

unsafe fn draw_fav_row(dc: HDC, i: usize, r: RECT, sn: &Snap) {
    let dpi = sn.dpi.max(96);
    if i == usize::MAX {
        crate::text(dc, r, tr("No favourites yet · add one with ☆+ in a pane header", "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가", "お気に入りはまだありません · ペインヘッダーの ☆+ で追加"), sn.small, crate::pal().sub, DT_CENTER);
        return;
    }
    let path = app(|a| a.s.favs.get(i).cloned().unwrap_or_default());
    if path.is_empty() {
        return;
    }
    let icon = RECT { left: r.left + sc(dpi, 12), right: r.left + sc(dpi, 34), ..r };
    crate::text(dc, icon, "\u{E8B7}", sn.icon, crate::pal().sub, DT_CENTER);
    let name_r = RECT { left: icon.right + sc(dpi, 4), right: r.left + (r.right - r.left) * 32 / 100, ..r };
    crate::text(dc, name_r, &crate::fav_label(&path), sn.font, crate::pal().text, DT_LEFT);
    let path_r = RECT { left: name_r.right + sc(dpi, 8), right: r.right - sc(dpi, 180), ..r };
    crate::text(dc, path_r, crate::fav_path(&path), sn.small, crate::pal().sub, DT_LEFT);
    crate::fill(dc, RECT { top: r.bottom - 1, ..r }, crate::pal().line);
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
        tx(s("Press a key…", "키를 누르세요…", "キーを押してください…")).to_string()
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
    mini(change, tx(s("Change", "변경", "変更")), true);
    mini(reset, tx(s("Reset", "되돌리기", "戻す")), crate::keys::is_custom(i));
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
    let on_card = id != S_CLOSE && id != S_RESET;
    let ground = if on_card { crate::pal().card } else { crate::pal().bg };
    crate::fill(dc_of(d), d.rcItem, ground);
    let (fill, border, ink) = if id == S_CLOSE && !dis {
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
        Some(crate::everything::EvAction::Start) => tx(s("Start Everything", "Everything 시작", "Everything を起動")),
        _ => tx(s("Install Everything", "Everything 설치", "Everything をインストール")),
    }
}

fn ev_status_line() -> String {
    use crate::everything::{Presence, Shown};
    match crate::everything::shown() {
        Shown::Unknown => tx(s("Checking…", "확인 중…", "確認しています…")).to_string(),
        Shown::Ready(Presence::NotInstalled) => tx(s("Not installed", "설치되어 있지 않음", "インストールされていません")).to_string(),
        Shown::Ready(Presence::InstalledStopped) => tx(s("Installed but not running", "설치되어 있지만 실행 중이 아님", "インストール済みですが実行されていません")).to_string(),
        Shown::Ready(Presence::Running { version }) => {
            let base = tx(s("Installed and running", "설치되어 실행 중", "インストール済みで実行中"));
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
        Activity::Starting => tx(s("Starting Everything…", "Everything을 시작하는 중…", "Everything を起動しています…")).to_string(),
        Activity::Installing => tx(s("Installing Everything…", "Everything을 설치하는 중…", "Everything をインストールしています…")).to_string(),
        Activity::Winget(line) => line,
        Activity::Started => tx(s("Everything is running.", "Everything이 실행 중입니다.", "Everything は実行中です。")).to_string(),
        Activity::Installed => tx(s("Everything was installed.", "Everything을 설치했습니다.", "Everything をインストールしました。")).to_string(),
        Activity::StartFailed => tx(s("Everything did not start.", "Everything을 시작하지 못했습니다.", "Everything を起動できませんでした。")).to_string(),
        Activity::InstallFailed(code) => format!("{} ({code})", tx(s("Install failed", "설치 실패", "インストールに失敗しました"))),
        Activity::OpenedPage => tx(s("Opened the download page.", "다운로드 페이지를 열었습니다.", "ダウンロードページを開きました。")).to_string(),
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
        TG_BAR => app(|a| a.s.bar ^= true),
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
        TG_AUTOSAVE => app(|a| a.s.autosave ^= true),
        TG_EV => {
            app(|a| a.s.use_everything ^= true);
            unsafe {
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
    app(|a| {
        a.s.ensure_fav_tree();
        crate::remove_leaf(&mut a.s.tree, k);
        a.s.sync_favs();
        a.fav_hot = None;
    });
    crate::favs_changed();
    crate::save_session();
}

unsafe fn on_command(h: HWND, id: i32, code: u32) {
    if BUSY.with(|b| b.get()) {
        return;
    }
    if code == EN_CHANGE && id == S_SEARCH {
        let q = crate::text_of(ctl(h, S_SEARCH));
        let page = ST.with(|s| s.borrow().page);
        let page = if page_match(&q, page) { page } else { (0..9).find(|p| page_match(&q, *p)).unwrap_or(page) };
        ST.with(|s| {
            let mut st = s.borrow_mut();
            st.query = q;
            st.page = page;
            st.scroll = 0;
        });
        layout(h);
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
        let lang = [Lang::Auto, Lang::En, Lang::Ko, Lang::Ja].get(sel).copied().unwrap_or(Lang::Auto);
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
        id if (S_FAV0..S_FAV0 + 128).contains(&id) => delete_fav((id - S_FAV0) as usize),
        id if (S_FAV0 + 128..S_FAV0 + 256).contains(&id) => rename_fav(h, (id - S_FAV0 - 128) as usize),
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
            let mode = [4, 3, 1][sel.min(2)];
            app(|a| a.s.view_new = mode);
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
                match unsafe { ask(h, tx(s("New file name", "새 파일 이름", "新しいファイル名")), &cur) } {
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
        S_START => {
            if sel == 1 {
                match unsafe { crate::pick_open(h) } {
                    Some(p) => app(|a| {
                        a.s.boot = 1;
                        a.s.boot_file = p;
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
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(s("Could not export the settings file.", "설정 파일을 내보내지 못했어요.", "設定ファイルを書き出せませんでした。"))), w!("grids"), MB_ICONWARNING);
    }
}

unsafe fn import_settings(h: HWND) {
    let Some(path) = crate::pick_settings(h, false) else { return };
    let Ok(text) = std::fs::read_to_string(&path) else {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(s("Could not read that file.", "그 파일을 읽지 못했어요.", "そのファイルを読めませんでした。"))), w!("grids"), MB_ICONWARNING);
        return;
    };
    let q = tx(s("Replace the current settings with this file?", "이 파일로 지금 설정을 바꿀까요?", "このファイルで現在の設定を置き換えますか?"));
    if MessageBoxW(Some(h), &HSTRING::from(q), w!("grids"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    if !crate::import_settings(&text) {
        let _ = MessageBoxW(Some(h), &HSTRING::from(tx(s("That file is not valid settings JSON.", "올바른 설정 JSON이 아니에요.", "正しい設定 JSON ではありません。"))), w!("grids"), MB_ICONWARNING);
    }
}

unsafe fn reset_ini(h: HWND) {
    let q = tx(s("Reset every preference to its default? The layout, folders and favourites stay.", "환경설정을 모두 기본값으로 되돌릴까요? 배치, 폴더, 즐겨찾기는 남아요.", "環境設定をすべて既定に戻しますか? レイアウト、フォルダー、お気に入りはそのままです。"));
    if MessageBoxW(Some(h), &HSTRING::from(q), w!("grids"), MB_YESNO | MB_ICONQUESTION) != IDYES {
        return;
    }
    crate::reset_prefs();
}

unsafe fn rename_fav(h: HWND, k: usize) {
    let cur = app(|a| a.s.favs.get(k).map(|f| crate::fav_label(f)).unwrap_or_default());
    let Some(name) = ask(h, tx(s("Rename", "이름 변경", "名前の変更")), &cur) else { return };
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    app(|a| {
        a.s.ensure_fav_tree();
        crate::rename_leaf(&mut a.s.tree, k, &name);
        a.s.sync_favs();
    });
    crate::favs_changed();
    crate::save_session();
}

unsafe fn reset_keys(h: HWND) {
    let q = tx(s(
        "Reset every shortcut to its default?",
        "단축키를 모두 기본값으로 되돌릴까요?",
        "ショートカットをすべて既定に戻しますか?",
    ));
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
    let q = if tr("en", "ko", "ja") == "ko" {
        format!("{other} 동작이 이미 {chord} 를 쓰고 있어요.\n두 단축키를 바꿀까요?\n\n예: 맞바꿈    아니오: 그대로 두기")
    } else if tr("en", "ko", "ja") == "ja" {
        format!("「{other}」がすでに {chord} を使っています。\n二つのショートカットを入れ替えますか?\n\nはい: 入れ替え    いいえ: そのまま")
    } else {
        format!("{other} already uses {chord}.\nSwap the two shortcuts?\n\nYes swaps them. No leaves both unchanged.")
    };
    confirm_box(hwnd(), &q)
}

unsafe fn confirm_box(owner: HWND, q: &str) -> bool {
    crate::keys::set_modal(true);
    let ans = MessageBoxW(Some(owner), &HSTRING::from(q), w!("grids"), MB_YESNO | MB_ICONWARNING);
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
    for i in 0..9 {
        if g.show[i] && inside(g.nav[i], x, y) {
            ST.with(|s| {
                let mut st = s.borrow_mut();
                st.page = i;
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
                ST.with(|s| s.borrow_mut().fav_drag = Some(*i));
                let _ = SetCapture(h);
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
                crate::everything::bind_settings(h.0 as isize);
                crate::everything::probe_async();
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
                if let Some(from) = ST.with(|s| s.borrow().fav_drag) {
                    let sn = snap();
                    let mut rc = RECT::default();
                    let _ = GetClientRect(h, &mut rc);
                    let g = geo(rc, &sn);
                    let to = g.items.iter().find_map(|it| match it {
                        Item::FavRow { i, rect, .. } if *i != usize::MAX && y >= rect.top && y < rect.bottom => Some(*i),
                        _ => None,
                    });
                    if let Some(to) = to.filter(|to| *to != from) {
                        app(|a| {
                            a.s.ensure_fav_tree();
                            if crate::reorder_leaves(&mut a.s.tree, from, to) {
                                a.s.sync_favs();
                            }
                        });
                        ST.with(|s| s.borrow_mut().fav_drag = Some(to));
                        layout(h);
                        let _ = InvalidateRect(Some(h), None, false);
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
                    let hot = (0..9).find(|&i| g.show[i] && inside(g.nav[i], x, y));
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
                    crate::favs_changed();
                    crate::save_session();
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
            WM_CLOSE => {
                let _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                crate::everything::bind_settings(0);
                crate::keys::cancel();
                let old = ST.with(|s| {
                    let mut st = s.borrow_mut();
                    let old = (st.font, st.small, st.title_font, st.mono, st.icon);
                    *st = St::default(); // drops the installed-family list
                    old
                });
                for f in [old.0, old.1, old.2, old.3, old.4] {
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
    match msg {
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
        _ => {}
    }
    DefWindowProcW(h, msg, wp, lp)
}

/// One-line prompt. None means cancel.
unsafe fn ask(owner: HWND, title: &str, initial: &str) -> Option<String> {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let Ok(m) = GetModuleHandleW(None) else { return };
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ask_proc),
            hInstance: m.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut _),
            lpszClassName: w!("GridsAsk"),
            ..Default::default()
        });
    });
    ASK_OUT.with(|o| *o.borrow_mut() = None);
    let dpi = GetDpiForWindow(owner).max(96) as i32;
    let (ww, hh) = (sc(dpi, 420), sc(dpi, 140));
    let mut orc = RECT::default();
    let _ = GetWindowRect(owner, &mut orc);
    let x = orc.left + (orc.right - orc.left - ww) / 2;
    let y = orc.top + (orc.bottom - orc.top - hh) / 2;
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU | WS_VISIBLE;
    let h = CreateWindowExW(WS_EX_DLGMODALFRAME, w!("GridsAsk"), &HSTRING::from(title), style, x, y, ww, hh, Some(owner), None, None, None).unwrap_or_default();
    if h.0.is_null() {
        return None;
    }
    let font = ST.with(|s| s.borrow().font);
    let edit = child_at(h, w!("EDIT"), initial, 0x80 | 0x00010000 | WS_BORDER.0, 10, font);
    ASK_EDIT.with(|e| e.set(edit));
    let _ = child_at(h, w!("BUTTON"), tx(s("OK", "확인", "OK")), 0xB | 0x00010000, 1, font);
    let _ = child_at(h, w!("BUTTON"), tx(CLOSE_L), 0xB | 0x00010000, 2, font);
    let _ = SetWindowPos(edit, None, sc(dpi, 16), sc(dpi, 16), ww - sc(dpi, 48), sc(dpi, 28), SWP_NOZORDER);
    let _ = SetWindowPos(ctl(h, 1), None, ww - sc(dpi, 200), sc(dpi, 60), sc(dpi, 80), sc(dpi, 28), SWP_NOZORDER);
    let _ = SetWindowPos(ctl(h, 2), None, ww - sc(dpi, 108), sc(dpi, 60), sc(dpi, 80), sc(dpi, 28), SWP_NOZORDER);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow(owner, false);
    let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
    let mut msg = MSG::default();
    while ASK_OUT.with(|o| o.borrow().is_none()) {
        if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
            PostQuitMessage(0);
            break;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
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
}
