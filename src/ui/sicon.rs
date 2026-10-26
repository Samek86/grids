// Save and load on the tab row. A session is a stack of windows: the back frame sits up-left
// and the front frame holds a 2×2 pane grid. The corner badge is a disc with the mark cut
// out of it: a plus to keep the arrangement, an up arrow to restore it. Rasterised like the
// star — one coverage bitmap — so the ink follows hover, pressed, disabled, and dirty green.
// Frames snap to the pixel grid and stay solid (0 or 255) at 100/125/150/200%.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum SessionIcon {
    Save,
    Load,
}

/// Device pixels for the icon at `dpi` (96 = 100%). 16px at 96, tracking the 14px icon font
/// and the 15px star without crowding the 24px-tall toolbar button.
pub(crate) fn session_icon_px(dpi: i32) -> i32 {
    (16 * dpi.max(1) / 96).max(1)
}

/// Coverage, 0..=255, row-major `n`×`n`. `n` is [`session_icon_px`].
pub(crate) fn session_icon_coverage(n: i32, kind: SessionIcon) -> Vec<u8> {
    let n = n.max(1);
    let lay = layout(n);
    let mut out = vec![0u8; (n * n) as usize];
    for py in 0..n {
        for px in 0..n {
            let mut hit = 0u32;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = px as f32 + (sx as f32 + 0.5) / 4.0;
                    let y = py as f32 + (sy as f32 + 0.5) / 4.0;
                    hit += u32::from(sample(x, y, kind, &lay));
                }
            }
            out[(py * n + px) as usize] = (hit * 255 / 16) as u8;
        }
    }
    out
}

/// Premultiplied top-down BGRA of [`session_icon_coverage`], ink `rgb` as 0xRRGGBB.
pub(crate) fn session_icon_bgra(n: i32, kind: SessionIcon, rgb: u32) -> Vec<u8> {
    let cov = session_icon_coverage(n, kind);
    let mut px = vec![0u8; cov.len() * 4];
    for (i, &a) in cov.iter().enumerate() {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8;
        px[i * 4..i * 4 + 4].copy_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    px
}

#[derive(Clone, Copy)]
struct Frame {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

struct Lay {
    sw: f32,
    scale: f32,
    back: Frame,
    front: Frame,
    badge: (f32, f32),
    erase: f32,
    disc: f32,
}

fn stroke_px(n: i32) -> f32 {
    // One device pixel at 100% and 125%, two from 150% up. Matches the star's ~11% weight
    // on the window itself (an 8px frame with a 1px stroke) and stays an integer so edges land
    // between pixels.
    let t = n as f32 / 16.0;
    if t < 1.4 { 1.0 } else { t.round().max(2.0) }
}

fn snap(v: f32, sw: f32) -> f32 {
    if (sw as i32) % 2 == 0 {
        v.round()
    } else {
        (v - 0.5).round() + 0.5
    }
}

fn frame_at(sw: f32, scale: f32, l: f32, t: f32, r: f32, b: f32) -> Frame {
    Frame {
        l: snap(l * scale, sw),
        t: snap(t * scale, sw),
        r: snap(r * scale, sw),
        b: snap(b * scale, sw),
    }
}

fn layout(n: i32) -> Lay {
    let sw = stroke_px(n);
    let scale = n as f32 / 16.0;
    // 16px master. The stack sits up-left so a whole badge disc fits in the corner.
    let back = frame_at(sw, scale, 1.5, 1.5, 8.5, 7.5);
    let front = frame_at(sw, scale, 3.5, 4.5, 11.5, 12.5);
    Lay {
        sw,
        scale,
        back,
        front,
        badge: (snap(12.2 * scale, sw), snap(12.2 * scale, sw)),
        erase: 3.85 * scale,
        disc: 2.95 * scale,
    }
}

fn sample(x: f32, y: f32, kind: SessionIcon, lay: &Lay) -> bool {
    let mut on = on_frame(x, y, &lay.back, lay.sw);
    if in_outer(x, y, &lay.front, lay.sw) {
        on = false;
    }
    if on_frame(x, y, &lay.front, lay.sw) || on_grid(x, y, &lay.front, lay.sw) {
        on = true;
    }
    let (cx, cy) = lay.badge;
    let d = (x - cx).hypot(y - cy);
    if d <= lay.erase {
        on = false;
    }
    if d <= lay.disc {
        on = !on_mark(x, y, kind, lay);
    }
    on
}

fn on_frame(x: f32, y: f32, f: &Frame, sw: f32) -> bool {
    let h = sw * 0.5;
    let outer = x >= f.l - h && x < f.r + h && y >= f.t - h && y < f.b + h;
    let inner = x >= f.l + h && x < f.r - h && y >= f.t + h && y < f.b - h;
    outer && !inner
}

fn in_outer(x: f32, y: f32, f: &Frame, sw: f32) -> bool {
    let h = sw * 0.5;
    x >= f.l - h && x < f.r + h && y >= f.t - h && y < f.b + h
}

/// 2×2 split of the front window's client. Lines stop at the inner edge of the frame.
fn on_grid(x: f32, y: f32, f: &Frame, sw: f32) -> bool {
    let h = sw * 0.5;
    let (l, t, r, b) = (f.l + h, f.t + h, f.r - h, f.b - h);
    let (mx, my) = ((f.l + f.r) * 0.5, (f.t + f.b) * 0.5);
    let vert = x >= mx - h && x < mx + h && y >= t && y < b;
    let horz = y >= my - h && y < my + h && x >= l && x < r;
    vert || horz
}

/// Cut out of the badge disc. Save is a plus (keep). Load is an up arrow (restore).
fn on_mark(x: f32, y: f32, kind: SessionIcon, lay: &Lay) -> bool {
    let (cx, cy, sw, s) = (lay.badge.0, lay.badge.1, lay.sw, lay.scale);
    let h = sw * 0.5;
    let arm = 1.65 * s;
    match kind {
        SessionIcon::Save => {
            let vert = x >= cx - h && x < cx + h && y >= cy - arm && y < cy + arm;
            let horz = y >= cy - h && y < cy + h && x >= cx - arm && x < cx + arm;
            vert || horz
        }
        SessionIcon::Load => {
            // Flat-headed arrow: a pixel above the bar, the bar, then the shaft. All axis-aligned.
            let head_y = snap(cy - s, sw);
            let tip_top = snap(cy - 2.0 * s, sw);
            let shaft_bot = snap(cy + 1.5 * s, sw);
            let x0 = snap(cx - 1.5 * s, sw);
            let x1 = snap(cx + 1.5 * s, sw);
            let tip = x >= cx - h && x < cx + h && y >= tip_top - h && y < head_y;
            let head = y >= head_y - h && y < head_y + h && x >= x0 - h && x < x1 + h;
            let shaft = x >= cx - h && x < cx + h && y >= head_y - h && y < shaft_bot + h;
            tip || head || shaft
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(c: &[u8], n: i32, x: i32, y: i32) -> u8 {
        c[(y * n + x) as usize]
    }

    #[test]
    fn sizes_track_the_toolbar_dpi_steps() {
        assert_eq!(session_icon_px(96), 16);
        assert_eq!(session_icon_px(120), 20);
        assert_eq!(session_icon_px(144), 24);
        assert_eq!(session_icon_px(192), 32);
        assert_eq!(session_icon_px(1), 1);
    }

    #[test]
    fn save_and_load_share_frames_and_differ_at_the_badge() {
        for n in [16, 20, 24, 32] {
            let lay = layout(n);
            let save = session_icon_coverage(n, SessionIcon::Save);
            let load = session_icon_coverage(n, SessionIcon::Load);
            assert_eq!(save.len(), (n * n) as usize);
            assert_ne!(save, load, "{n}");
            let (cx, cy, r) = (lay.badge.0, lay.badge.1, lay.erase);
            let mut diffs = 0;
            for y in 0..n {
                for x in 0..n {
                    let (s, l) = (at(&save, n, x, y), at(&load, n, x, y));
                    let d = (x as f32 + 0.5 - cx).hypot(y as f32 + 0.5 - cy);
                    if d > r + 1.75 {
                        assert_eq!(s, l, "n={n} leaked at {x},{y}");
                        assert!(s == 0 || s == 255, "n={n} soft frame {x},{y}={s}");
                    }
                    if s != l {
                        diffs += 1;
                        assert!(d <= r + 1.75, "n={n} diff outside badge at {x},{y}");
                    }
                }
            }
            assert!(diffs > 4, "{n} badges are identical");
            // Back frame's outer top-left pixel is solid ink, and it is the same on both.
            let (x, y) = ((lay.back.l - lay.sw / 2.0) as i32, (lay.back.t - lay.sw / 2.0) as i32);
            assert_eq!((at(&save, n, x, y), at(&load, n, x, y)), (255, 255), "{n} back corner");
            // The 2×2 cross is inside the front window and clear of the badge.
            let (gx, gy) = ((lay.front.l + lay.front.r) * 0.5, (lay.front.t + lay.front.b) * 0.5);
            let (x, y) = ((gx - lay.sw / 2.0) as i32, (gy - lay.sw / 2.0) as i32);
            assert_eq!((at(&save, n, x, y), at(&load, n, x, y)), (255, 255), "{n} grid");
            assert_eq!(at(&save, n, 0, 0), 0, "{n} margin");
        }
    }

    #[test]
    fn bgra_is_premultiplied_from_coverage() {
        let cov = session_icon_coverage(16, SessionIcon::Save);
        let px = session_icon_bgra(16, SessionIcon::Save, 0x112233);
        assert!(cov.iter().any(|&a| a == 255));
        assert!(cov.iter().any(|&a| a == 0));
        for (i, &a) in cov.iter().enumerate() {
            let p = &px[i * 4..i * 4 + 4];
            if a == 0 {
                assert_eq!(p, &[0, 0, 0, 0]);
            } else if a == 255 {
                assert_eq!(p, &[0x33, 0x22, 0x11, 255]);
            }
        }
    }
}
