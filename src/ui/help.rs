// In-app Help. F1 opens it. Esc or the close mark puts focus back.
// Ctrl+/ still opens the compact shortcut list. This window stays up while the main window is used.

use crate::imports::*;
use crate::app::*;
use crate::i18n::*;
use crate::pane::focus_view;
use super::*;

pub(crate) const HELP_TABS: usize = 8;
pub(crate) const HELP_SHORTCUTS: usize = 5;
pub(crate) const HELP_PAD: i32 = 16;
pub(crate) const HELP_GAP: i32 = 8;
pub(crate) const HELP_TIGHT: i32 = 4;
pub(crate) const HELP_HEAD: i32 = 44;
pub(crate) const HELP_MIN_W: i32 = 480;
pub(crate) const HELP_MIN_H: i32 = 360;

pub(crate) const HELP_TAB_ORDER: [crate::Key; HELP_TABS] = [
    crate::Key::HelpTabGettingStarted,
    crate::Key::HelpTabPanesTabs,
    crate::Key::HelpTabFavorites,
    crate::Key::HelpTabFileOperations,
    crate::Key::HelpTabServerConnections,
    crate::Key::HelpTabShortcuts,
    crate::Key::HelpTabSettings,
    crate::Key::HelpTabTroubleshooting,
];

pub(crate) const HELP_BODY_ORDER: [crate::Key; HELP_TABS] = [
    crate::Key::HelpBodyGettingStarted,
    crate::Key::HelpBodyPanesTabs,
    crate::Key::HelpBodyFavorites,
    crate::Key::HelpBodyFileOperations,
    crate::Key::HelpBodyServerConnections,
    crate::Key::HelpBodyShortcuts,
    crate::Key::HelpBodySettings,
    crate::Key::HelpBodyTroubleshooting,
];

pub(crate) struct HelpLay {
    pub head: i32,
    pub title: RECT,
    pub close: RECT,
    pub tabs: Vec<RECT>,
    pub tabs_bottom: i32,
    pub body: RECT,
    pub text_h: i32,
    pub extra_h: i32,
    pub content_h: i32,
}

/// Tabs stay on one row when labels fit; they wrap only on a very narrow window.
/// The body sits 16 DIP under the last row. `extra_h` is the live shortcut list.
pub(crate) fn help_layout(client_w: i32, client_h: i32, dpi: i32, tab_text_w: &[i32], line_h: i32, text_h: i32, extra_h: i32) -> HelpLay {
    let dpi = dpi.max(1);
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, tight) = (s(HELP_PAD), s(HELP_GAP), s(HELP_TIGHT));
    let head = s(HELP_HEAD);
    let cw = client_w.max(1);
    let ch = client_h.max(1);
    let close = keys_close(cw, dpi);
    let title = RECT { left: pad, top: 0, right: (close.left - gap).max(pad + 1), bottom: head };
    let tab_h = line_h.max(1) + 2 * tight;
    let limit = (cw - pad).max(pad + 1);
    let mut x = pad;
    let mut y = head + gap;
    let mut tabs = Vec::with_capacity(tab_text_w.len());
    // Tighter chip padding so ko/ja/en fit one row in the default Help width.
    let chip_pad = tight;
    let chip_gap = tight.max(s(2));
    for w in tab_text_w {
        let cell = (*w).max(0) + 2 * chip_pad;
        if x > pad && x + cell > limit {
            x = pad;
            y += tab_h + gap;
        }
        tabs.push(RECT { left: x, top: y, right: x + cell, bottom: y + tab_h });
        x += cell + chip_gap;
    }
    let tabs_bottom = tabs.iter().map(|r| r.bottom).max().unwrap_or(head);
    let body_top = tabs_bottom + s(HELP_PAD);
    let body_left = pad;
    let body_right = (cw - pad - s(HELP_GAP)).max(body_left + 1);
    let body_bottom = (ch - pad).max(body_top);
    let text_h = text_h.max(0);
    let extra_h = extra_h.max(0);
    let content_h = if extra_h > 0 { text_h + s(HELP_PAD) + extra_h } else { text_h };
    HelpLay {
        head,
        title,
        close,
        tabs,
        tabs_bottom,
        body: RECT { left: body_left, top: body_top, right: body_right, bottom: body_bottom },
        text_h,
        extra_h,
        content_h,
    }
}

pub(crate) fn help_tab_at(tabs: &[RECT], x: i32, y: i32) -> Option<usize> {
    tabs.iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)
}

pub(crate) fn help_scroll_max(content_h: i32, view_h: i32) -> i32 {
    (content_h - view_h.max(0)).max(0)
}

/// Initial outer size. Same 85% cap and centering as the shortcut list, with a taller floor.
pub(crate) fn help_fit_size(owner_w: i32, owner_h: i32, dpi: i32) -> (i32, i32) {
    let (w, h) = keys_fit_size(owner_w, owner_h, dpi, |_| i32::MAX / 4);
    let px = |dip: i32| dip * dpi.max(1) / 96;
    (w.max(px(HELP_MIN_W)), h.max(px(HELP_MIN_H)))
}

fn help_accent() -> u32 {
    let (c, dark) = app(|a| (a.s.accent_now(), a.s.dark));
    let plain = if dark { DARK.accents[1].1 } else { LIGHT.accents[1].1 };
    if c == plain {
        if dark { 0x4CC2FF } else { 0x0067C0 }
    } else {
        c
    }
}

unsafe fn text_px(font: HFONT, t: &str) -> (i32, i32) {
    if t.is_empty() || font.is_invalid() {
        return (0, 16);
    }
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut sz = SIZE::default();
    _ = GetTextExtentPoint32W(dc, &t.encode_utf16().collect::<Vec<_>>(), &mut sz);
    SelectObject(dc, old);
    _ = ReleaseDC(None, dc);
    (sz.cx.max(0), sz.cy.max(1))
}

unsafe fn block_h(font: HFONT, text: &str, width: i32) -> i32 {
    if text.is_empty() {
        return 0;
    }
    let dc = GetDC(None);
    let (_, h) = wrap_text(dc, RECT { right: width.max(1), ..Default::default() }, text, font, 0, true);
    _ = ReleaseDC(None, dc);
    h.max(1)
}

unsafe fn live_lay(client_w: i32, client_h: i32, dpi: i32, tab: usize) -> HelpLay {
    let font = app(|a| a.font);
    let tab = tab.min(HELP_TABS - 1);
    let widths: Vec<i32> = HELP_TAB_ORDER.map(|k| text_px(font, tr(k)).0).to_vec();
    let line = text_px(font, "Ag").1.max(1);
    let draft = help_layout(client_w, client_h, dpi, &widths, line, 0, 0);
    let body_w = (draft.body.right - draft.body.left).max(1);
    let text = tr(HELP_BODY_ORDER[tab]);
    let text_h = block_h(font, text, body_w);
    let extra = if tab == HELP_SHORTCUTS { keys_geo(body_w, dpi).h } else { 0 };
    help_layout(client_w, client_h, dpi, &widths, line, text_h, extra)
}

fn shift(r: RECT, dx: i32, dy: i32) -> RECT {
    RECT { left: r.left + dx, top: r.top + dy, right: r.right + dx, bottom: r.bottom + dy }
}

unsafe fn paint_help(h: HWND) {
    let dpi = app(|a| a.dpi).max(96);
    let (font, small, icon_font, scroll, tab) = app(|a| (a.font, a.small_font, a.icon_font, a.help_scroll, a.help_tab));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let lay = live_lay(rc.right, rc.bottom, dpi, tab);
    let s = |v: i32| v * dpi / 96;
    let accent = help_accent();
    let on = on_color(accent);
    let body_text = tr(HELP_BODY_ORDER[tab.min(HELP_TABS - 1)]);
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        fill(dc, RECT { bottom: lay.head, ..rc }, pal().card);
        text(dc, lay.title, tr(crate::Key::Help), font, pal().text, DT_LEFT);
        text(dc, lay.close, "\u{E711}", icon_font, pal().sub, DT_CENTER);
        fill(dc, RECT { top: lay.head - 1, bottom: lay.head, ..rc }, pal().line);
        let rad = s(HELP_TIGHT).max(1);
        for (i, r) in lay.tabs.iter().enumerate() {
            let title = tr(HELP_TAB_ORDER[i]);
            if i == tab {
                aa_rfill(dc, *r, rad, accent, None, pal().card);
                text(dc, *r, title, font, on, DT_CENTER);
            } else {
                aa_rfill(dc, *r, rad, pal().pill, Some(pal().line), pal().card);
                text(dc, *r, title, font, pal().text, DT_CENTER);
            }
        }
        if lay.tabs_bottom > lay.head {
            fill(dc, RECT { top: lay.tabs_bottom, bottom: lay.tabs_bottom + 1, ..rc }, pal().line);
        }
        let saved = SaveDC(dc);
        if saved != 0 {
            IntersectClipRect(dc, lay.body.left, lay.body.top, lay.body.right, lay.body.bottom);
        }
        let top = lay.body.top - scroll;
        if lay.text_h > 0 {
            let block = RECT { left: lay.body.left, top, right: lay.body.right, bottom: top + lay.text_h };
            wrap_text(dc, block, body_text, font, pal().text, false);
        }
        if tab == HELP_SHORTCUTS && lay.extra_h > 0 {
            let rows = key_rows(&crate::keys::help_text());
            let width = (lay.body.right - lay.body.left).max(1);
            let g = keys_layout(&rows, width, dpi, &measure_chords(font));
            let dx = lay.body.left;
            let dy = top + lay.text_h + s(HELP_PAD);
            let names = [tr(crate::Key::Keyboard), tr(crate::Key::MouseAndButtons)];
            let face = if small.is_invalid() { font } else { small };
            for (t, r) in names.iter().zip(g.titles) {
                text(dc, shift(r, dx, dy), t, face, pal().sub, DT_LEFT);
            }
            for (gr, r) in g.rows.iter().zip(rows.iter().flatten()) {
                for (k, kr) in r.keys.iter().zip(&gr.keys) {
                    let kr = shift(*kr, dx, dy);
                    if gr.sec == 0 {
                        fill(dc, kr, pal().pill);
                        frame(dc, kr, 1, pal().line);
                        text(dc, RECT { left: kr.left + s(4), right: kr.right - s(4), ..kr }, k, font, pal().text, DT_CENTER);
                    } else {
                        wrap_text(dc, kr, k, font, pal().text, false);
                    }
                }
                wrap_text(dc, shift(gr.desc, dx, dy), &r.desc, font, pal().text, false);
            }
        }
        if saved != 0 {
            let _ = RestoreDC(dc, saved);
        }
        let view = lay.body.bottom - lay.body.top;
        if lay.content_h > view && lay.content_h > 0 && view > 0 {
            let th = (view * view / lay.content_h).max(s(24)).min(view);
            let travel = (view - th).max(1);
            let max = (lay.content_h - view).max(1);
            let thumb = lay.body.top + scroll.clamp(0, max) * travel / max;
            fill(dc, RECT { left: rc.right - s(6), top: thumb, right: rc.right - s(3), bottom: thumb + th }, pal().sub);
        }
        frame(dc, rc, 1, pal().line);
    });
}

fn measure_chords(font: HFONT) -> impl Fn(&str, i32, bool) -> (i32, i32) {
    move |t: &str, max_w: i32, cap: bool| unsafe {
        if cap {
            text_px(font, t)
        } else {
            let dc = GetDC(None);
            let size = wrap_text(dc, RECT { right: max_w, ..Default::default() }, t, font, 0, true);
            _ = ReleaseDC(None, dc);
            size
        }
    }
}

unsafe fn help_scroll_by(h: HWND, to: impl FnOnce(i32, i32) -> i32) {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let dpi = app(|a| a.dpi).max(96);
    let tab = app(|a| a.help_tab);
    let lay = live_lay(rc.right, rc.bottom, dpi, tab);
    let view = lay.body.bottom - lay.body.top;
    let max = help_scroll_max(lay.content_h, view);
    if app(|a| {
        let y = to(a.help_scroll, view).clamp(0, max);
        std::mem::replace(&mut a.help_scroll, y) != y
    }) {
        _ = InvalidateRect(Some(h), None, false);
    }
}

unsafe fn set_help_tab(h: HWND, tab: usize) {
    app(|a| {
        a.help_tab = tab % HELP_TABS;
        a.help_scroll = 0;
    });
    _ = InvalidateRect(Some(h), None, false);
}

pub(crate) unsafe fn open_help_window() {
    let (main, dpi, open) = app(|a| (a.main, a.dpi.max(96), a.help_popup));
    if main.0.is_null() {
        return;
    }
    if !open.0.is_null() && IsWindow(Some(open)).as_bool() {
        _ = ShowWindow(open, SW_SHOW);
        _ = SetFocus(Some(open));
        return;
    }
    let mut client = RECT::default();
    _ = GetClientRect(main, &mut client);
    let mut tl = POINT { x: client.left, y: client.top };
    let mut br = POINT { x: client.right, y: client.bottom };
    _ = ClientToScreen(main, &mut tl);
    _ = ClientToScreen(main, &mut br);
    let mut frame = RECT::default();
    _ = GetWindowRect(main, &mut frame);
    let (w, h) = help_fit_size(br.x - tl.x, br.y - tl.y, dpi);
    let mut mi = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    let work = if GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() { mi.rcWork } else { RECT::default() };
    let (x, y) = keys_fit_origin(frame, w, h, work);
    let prev = GetFocus();
    app(|a| {
        a.help_prev = prev;
        a.help_scroll = 0;
        a.help_tab = 0;
    });
    let title = HSTRING::from(tr(crate::Key::Help));
    let style = WS_POPUP | WS_THICKFRAME | WS_VISIBLE | WS_CLIPCHILDREN;
    let Ok(p) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("MaguroHelp"), &title, style, x, y, w, h, Some(main), None, None, None) else {
        return;
    };
    let dark = DARK_ON.load(Ordering::Relaxed);
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(p, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4);
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(p, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.help_popup = p);
    _ = SetFocus(Some(p));
}

/// Esc or the close mark. Focus goes back. Losing activation does not call this.
pub(crate) unsafe fn close_help() {
    let (h, prev, active) = app(|a| (a.help_popup, a.help_prev, a.s.active));
    if h.0.is_null() {
        return;
    }
    _ = DestroyWindow(h);
    if !prev.0.is_null() && IsWindow(Some(prev)).as_bool() {
        _ = SetFocus(Some(prev));
    } else {
        focus_view(active);
    }
}

pub(crate) extern "system" fn help_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    crate::crash::catch_wnd(std::panic::AssertUnwindSafe(|| help_proc_inner(h, msg, wp, lp)))
}

fn help_proc_inner(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        let dpi = app(|a| a.dpi).max(96);
        let s = |v: i32| v * dpi / 96;
        match msg {
            WM_PAINT => paint_help(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_SIZE => {
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let tab = app(|a| a.help_tab);
                let lay = live_lay(rc.right, rc.bottom, dpi, tab);
                let max = help_scroll_max(lay.content_h, lay.body.bottom - lay.body.top);
                app(|a| a.help_scroll = a.help_scroll.clamp(0, max));
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_GETMINMAXINFO => {
                let m = &mut *(lp.0 as *mut MINMAXINFO);
                m.ptMinTrackSize.x = HELP_MIN_W * dpi / 96;
                m.ptMinTrackSize.y = HELP_MIN_H * dpi / 96;
            }
            WM_MOUSEWHEEL => help_scroll_by(h, |y, _| y - s((wp.0 >> 16) as i16 as i32)),
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                let tab = app(|a| a.help_tab);
                let lay = live_lay(rc.right, rc.bottom, dpi, tab);
                let close = lay.close;
                if x >= close.left && x < close.right && y >= close.top && y < close.bottom {
                    close_help();
                } else if let Some(i) = help_tab_at(&lay.tabs, x, y) {
                    set_help_tab(h, i);
                }
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
                if vk == VK_ESCAPE {
                    close_help();
                } else if vk == VK_LEFT || vk == VK_RIGHT || vk == VK_TAB || (ctrl && (vk == VK_PRIOR || vk == VK_NEXT)) {
                    let back = vk == VK_LEFT || (vk == VK_TAB && shift) || (ctrl && vk == VK_PRIOR);
                    let cur = app(|a| a.help_tab);
                    set_help_tab(h, if back { cur + HELP_TABS - 1 } else { cur + 1 });
                } else if vk == VK_UP {
                    help_scroll_by(h, |y, _| y - s(40));
                } else if vk == VK_DOWN {
                    help_scroll_by(h, |y, _| y + s(40));
                } else if vk == VK_PRIOR {
                    help_scroll_by(h, |y, p| y - p * 9 / 10);
                } else if vk == VK_NEXT {
                    help_scroll_by(h, |y, p| y + p * 9 / 10);
                } else if vk == VK_HOME {
                    help_scroll_by(h, |_, _| 0);
                } else if vk == VK_END {
                    help_scroll_by(h, |_, _| i32::MAX);
                }
            }
            WM_CLOSE => close_help(),
            WM_DESTROY => app(|a| {
                if a.help_popup == h {
                    a.help_popup = HWND::default();
                }
            }),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlap(a: RECT, b: RECT) -> bool {
        a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
    }

    #[test]
    fn help_window_fits_like_the_shortcut_list_and_has_a_floor() {
        let (w, h) = help_fit_size(1200, 780, 96);
        assert_eq!((w, h), (760, 780 * 85 / 100));
        assert_eq!(help_fit_size(2000, 1600, 96).0, 760);
        assert_eq!(help_fit_size(100, 80, 96), (HELP_MIN_W, HELP_MIN_H));
        assert_eq!(help_fit_size(100, 80, 192), (HELP_MIN_W * 2, HELP_MIN_H * 2));
        let owner = RECT { left: 100, top: 80, right: 1300, bottom: 860 };
        assert_eq!(keys_fit_origin(owner, 760, 663, RECT::default()), (320, 138));
        let work = RECT { left: 0, top: 0, right: 800, bottom: 600 };
        assert_eq!(keys_fit_origin(owner, 760, 663, work), (40, 0));
    }

    #[test]
    fn help_tabs_wrap_and_the_body_scrolls_under_them() {
        let widths = vec![120; HELP_TABS];
        let wide = help_layout(1200, 800, 96, &widths, 16, 2000, 0);
        assert_eq!(wide.tabs.len(), HELP_TABS);
        assert!(wide.tabs.iter().all(|r| r.top == wide.tabs[0].top));
        assert_eq!(wide.tabs[0].left, HELP_PAD);
        assert_eq!(wide.tabs[1].left - wide.tabs[0].right, HELP_TIGHT);
        assert_eq!(wide.tabs[0].top, HELP_HEAD + HELP_GAP);
        assert_eq!(wide.body.left, HELP_PAD);
        assert_eq!(wide.body.top, wide.tabs_bottom + HELP_PAD);
        assert_eq!(wide.content_h, 2000);
        assert!(help_scroll_max(wide.content_h, wide.body.bottom - wide.body.top) > 0);
        let narrow = help_layout(280, 900, 96, &widths, 16, 40, 200);
        assert!(narrow.tabs.iter().map(|r| r.top).collect::<std::collections::BTreeSet<_>>().len() > 1);
        assert_eq!(narrow.content_h, 40 + HELP_PAD + 200);
        for dpi in [96, 144, 192] {
            let lay = help_layout(900 * dpi / 96, 700 * dpi / 96, dpi, &vec![80 * dpi / 96; HELP_TABS], 16 * dpi / 96, 100, 0);
            let pad = HELP_PAD * dpi / 96;
            assert_eq!(lay.body.left, pad);
            assert_eq!(lay.close.bottom <= lay.head, true);
            assert!(lay.title.right <= lay.close.left);
            for (i, a) in lay.tabs.iter().enumerate() {
                assert!(!overlap(*a, lay.body));
                for b in lay.tabs.iter().skip(i + 1) {
                    assert!(!overlap(*a, *b));
                }
            }
            assert_eq!(help_tab_at(&lay.tabs, lay.tabs[3].left, lay.tabs[3].top), Some(3));
            assert_eq!(help_tab_at(&lay.tabs, lay.body.left, lay.body.top), None);
        }
        assert_eq!(help_scroll_max(10, 40), 0);
        assert_eq!(help_scroll_max(100, 40), 60);
    }

    #[test]
    fn shortcuts_tab_is_the_live_chord_list() {
        assert_eq!(HELP_BODY_ORDER[HELP_SHORTCUTS], crate::Key::HelpBodyShortcuts);
        assert_eq!(HELP_TAB_ORDER.len(), HELP_TABS);
        let text = crate::keys::help_text();
        assert!(text.lines().filter(|l| l.contains('\t')).count() >= 10);
        assert!(text.contains("F1"));
        assert!(text.contains("Ctrl+/"));
    }
}
