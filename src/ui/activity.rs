// Narrow activity bar at the far left of the plate area. It switches the left panel
// between the folder tree and favourites, and collapses that panel. Search is drawn
// and does nothing yet. One owner-drawn window, no extra shell controls.

use crate::imports::*;
use crate::app::*;
use crate::pane::*;
use crate::session::*;
use super::*;

const ICON_FOLDER: &str = "\u{E8B7}";
const ICON_STAR: &str = "\u{E734}";

/// What a click on an activity icon does. Search (2) is disabled.
pub(crate) fn panel_after_click(panel: u8, open: bool, icon: u8) -> Option<(u8, bool)> {
    if icon >= 2 {
        return None;
    }
    if icon == panel {
        Some((panel, !open))
    } else {
        Some((icon, true))
    }
}

pub(crate) unsafe fn set_panel(icon: u8) {
    let changed = app(|a| {
        let Some((panel, open)) = panel_after_click(a.s.panel, a.s.tree_on, icon) else { return false };
        a.s.panel = panel;
        a.s.tree_on = open;
        true
    });
    if !changed {
        return;
    }
    publish_shared();
    across_windows(|| relayout());
    note_save();
}

fn icon_at(y: i32, dpi: i32) -> Option<u8> {
    let s = |v: i32| v * dpi / 96;
    let h = s(36);
    if y < 0 {
        return None;
    }
    let i = y / h;
    (i < 3).then_some(i as u8)
}

pub(crate) unsafe fn paint_activity(h: HWND) {
    let (dpi, icon, panel, open, accent, hot) = app(|a| (a.dpi, a.icon_font, a.s.panel, a.s.tree_on, a.s.accent_now(), a.act_hot));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        fill(dc, rc, pal().bg);
        fill(dc, RECT { left: rc.right - 1, ..rc }, pal().line);
        let icons = [ICON_FOLDER, ICON_STAR, ICON_SEARCH];
        for (i, glyph) in icons.iter().enumerate() {
            let r = RECT { left: 0, top: s(36) * i as i32, right: rc.right, bottom: s(36) * (i as i32 + 1) };
            if r.top >= rc.bottom {
                break;
            }
            let on = open && panel as usize == i;
            let over = hot == Some(i as u8);
            if on {
                fill(dc, r, pal().card);
                fill(dc, RECT { right: s(2), ..r }, accent);
            } else if over && i < 2 {
                fill(dc, r, blend(pal().card, pal().bg, 0.45));
            }
            let ink = if i == 2 {
                blend(pal().sub, pal().bg, 0.45)
            } else if on {
                pal().text
            } else {
                pal().sub
            };
            text(dc, r, glyph, icon, ink, DT_CENTER);
        }
    });
}

pub(crate) extern "system" fn act_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        bind_hwnd(h);
        match msg {
            WM_PAINT => paint_activity(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let y = xy(lp).1;
                let hot = icon_at(y, app(|a| a.dpi));
                let paint = app(|a| {
                    let paint = a.act_hot != hot;
                    a.act_hot = hot;
                    paint
                });
                if paint {
                    _ = InvalidateRect(Some(h), None, false);
                }
                let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                _ = TrackMouseEvent(&mut t);
            }
            WM_MOUSELEAVE => {
                app(|a| a.act_hot = None);
                _ = InvalidateRect(Some(h), None, false);
            }
            WM_LBUTTONUP => {
                if let Some(i) = icon_at(xy(lp).1, app(|a| a.dpi)) {
                    set_panel(i);
                }
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

#[cfg(test)]
mod tests {
    use super::panel_after_click;

    #[test]
    fn activity_click_toggles_collapses_and_ignores_search() {
        assert_eq!(panel_after_click(0, true, 0), Some((0, false)));
        assert_eq!(panel_after_click(0, false, 0), Some((0, true)));
        assert_eq!(panel_after_click(0, false, 1), Some((1, true)));
        assert_eq!(panel_after_click(1, true, 1), Some((1, false)));
        assert_eq!(panel_after_click(1, true, 2), None);
        assert_eq!(panel_after_click(0, true, 2), None);
    }
}
