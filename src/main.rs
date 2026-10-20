// Q-Dir style multi-pane file manager: shell tree on the left, 1x1..4x4 grid of Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen, GdipDrawPath, GdipFillPath,
    GdipSetPixelOffsetMode, GdipSetSmoothingMode, GdiplusStartup, GdiplusStartupInput, GpGraphics, PixelOffsetModeHalf,
    SmoothingModeAntiAlias, UnitPixel,
};
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::SFGAO_FOLDER;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Controls::Dialogs::{
    ChooseFontW, CF_FORCEFONTEXIST, CF_INITTOLOGFONTSTRUCT, CF_NOSCRIPTSEL, CF_NOVERTFONTS, CF_SCREENFONTS, CHOOSEFONTW,
};
use windows::Win32::UI::Controls::{
    BeginBufferedPaint, BufferedPaintInit, EndBufferedPaint, BPBF_TOPDOWNDIB, DRAWITEMSTRUCT, ODS_SELECTED, TOOLTIPS_CLASSW,
    TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTTOOLINFOW, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::Common::{COMDLG_FILTERSPEC, ITEMIDLIST, STRRET};
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const MAXP: usize = 16;
const THIS_PC: &str = "::{20D04FE0-3AEA-1069-A2D8-08002B30309D}";
// child ids inside a pane
const ID_BACK: i32 = 101;
const ID_FWD: i32 = 102;
const ID_UP: i32 = 103;
const ID_EDIT: i32 = 104;
const ID_OPEN_IN: i32 = 105;
const ID_FILTER: i32 = 106;
// top toolbar buttons (children of the main window)
const ID_TB_LAYOUT: i32 = 201;
const ID_TB_SESS: i32 = 202;
const ID_TB_VIEW: i32 = 203;
// menu ids
const ID_HIDDEN: usize = 1100;
const ID_KEYS: usize = 1101;
const ID_FONT: usize = 1102;
const ID_SESS_SAVE: usize = 1200;
const ID_SESS_DIR: usize = 1201;
const ID_SESS: usize = 1300; // + index into App::sess_files
const EM_SETSEL: u32 = 0xB1;
const EM_SETCUEBANNER: u32 = 0x1501;
const EN_CHANGE: u32 = 0x0300;
const TIMER_STATUS: usize = 1;
const TIMER_FILTER: usize = 2;
const COPY_ID: usize = 0x5144_4952; // "QDIR": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

// "Pebble" theme, 0xRRGGBB: soft gray ground, white rounded plates, one accent.
const BG: u32 = 0xEEF0F3;
const CARD: u32 = 0xFFFFFF;
const LINE: u32 = 0xE3E6EB;
const PILL: u32 = 0xF2F3F6;
const TEXT: u32 = 0x1D2026;
const SUB: u32 = 0x6B7280;
const ACCENT: u32 = 0x3478F6;
const ACCENT_SOFT: u32 = 0xD9E6FD;
const SHADOW: u32 = 0x0F172A;
// Segoe Fluent Icons / MDL2 Assets code points
const ICON_SEARCH: &str = "\u{E721}";

fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_TB_LAYOUT => "\u{E8A9}",
        ID_TB_SESS => "\u{E728}",
        _ => "\u{E890}",
    }
}

const KEYS_HELP: &str = "Ctrl+Tab / Ctrl+Shift+Tab\t다음 / 이전 패널\n\
Alt+← / Alt+→\t뒤로 / 앞으로\n\
Alt+↑\t상위 폴더\n\
Ctrl+L, Alt+D, F4\t주소창\n\
Ctrl+F\t필터 (*.pdf;*.txt 또는 이름 일부, 폴더는 항상 표시)\n\
Enter / Esc\t주소창·필터 적용 / 취소\n\
Ctrl+H\t숨김 파일 표시\n\
F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n\
레이아웃 버튼: 마우스로 행×열을 고르고 클릭 (방향키+Enter도 가능)\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기\n\
세션: 레이아웃+폴더를 .qdr로 저장 / 불러오기\n\
보기 → 글꼴: exe 옆 fonts 폴더의 글꼴(예: Pretendard)도 설치 없이 사용\n\
스플리터 더블클릭: 균등 분할\n\
실행 인자: qdir.exe C:\\a D:\\b ... 또는 qdir.exe 작업.qdr\n\
이미 실행 중이면 그 창에서 엽니다.";

// ---------- session (qdir.ini next to the exe; named sessions are the same format as .qdr) ----------

#[derive(Debug, PartialEq, Clone)]
struct Session {
    win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    side: i32,             // sidebar width in px
    rows: usize,
    cols: usize,
    colr: Vec<f64>,
    rowr: Vec<f64>,
    active: usize,
    paths: Vec<String>,     // parsing names per pane slot
    views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    font: String,           // UI font face; empty = first installed of the defaults
    font_pt: i32,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            win: None,
            side: 0,
            rows: 2,
            cols: 2,
            colr: even(2),
            rowr: even(2),
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            font: String::new(),
            font_pt: 10,
        }
    }
}

fn even(n: usize) -> Vec<f64> {
    vec![1.0 / n as f64; n]
}

fn nums<T: std::str::FromStr>(v: &str) -> Vec<T> {
    v.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

fn join<T: Display>(v: &[T]) -> String {
    v.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(",")
}

// Ratios from a hand-edited ini can be anything: wrong length, zero, negative -> fall back to equal.
fn fix(r: Vec<f64>, n: usize) -> Vec<f64> {
    if r.len() != n || r.iter().any(|x| !(*x > 0.0)) {
        return even(n);
    }
    let sum: f64 = r.iter().sum();
    r.iter().map(|x| x / sum).collect()
}

impl Session {
    fn parse(text: &str) -> Session {
        let mut s = Session::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                "side" => s.side = v.parse().unwrap_or(0),
                "grid" => {
                    if let [r, c] = nums::<usize>(v)[..] {
                        (s.rows, s.cols) = (r.clamp(1, 4), c.clamp(1, 4));
                    }
                }
                "colr" => s.colr = nums(v),
                "rowr" => s.rowr = nums(v),
                "active" => s.active = v.parse().unwrap_or(0),
                "font" => s.font = v.trim().to_string(),
                "font_pt" => s.font_pt = v.parse().unwrap_or(10).clamp(7, 24),
                _ => {
                    if let Some(i) = slot('p') {
                        s.paths[i] = v.to_string();
                    } else if let (Some(i), [m, z]) = (slot('v'), &nums::<i32>(v)[..]) {
                        s.views[i] = (*m, *z);
                    }
                }
            }
        }
        s.colr = fix(std::mem::take(&mut s.colr), s.cols);
        s.rowr = fix(std::mem::take(&mut s.rowr), s.rows);
        if s.active >= s.rows * s.cols {
            s.active = 0;
        }
        s
    }

    fn to_ini(&self) -> String {
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        o += &format!("side={}\ngrid={},{}\n", self.side, self.rows, self.cols);
        o += &format!("colr={}\nrowr={}\nactive={}\n", join(&self.colr), join(&self.rowr), self.active);
        o += &format!("font={}\nfont_pt={}\n", self.font, self.font_pt);
        for i in 0..MAXP {
            if !self.paths[i].is_empty() {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
        }
        o
    }

    /// Takes grid + folders from a named session; window position and sidebar width stay as they are.
    fn take_layout(&mut self, o: &Session) {
        (self.rows, self.cols, self.colr, self.rowr) = (o.rows, o.cols, o.colr.clone(), o.rowr.clone());
        (self.active, self.paths, self.views) = (o.active, o.paths.clone(), o.views.clone());
    }
}

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
fn resolve(p: String) -> String {
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
}

// A lone *.qdr argument means "load this session".
fn qdr(args: &[String]) -> Option<&str> {
    match args {
        [f] if f.to_lowercase().ends_with(".qdr") => Some(f),
        _ => None,
    }
}

// `qdir.exe A B C` opens the paths in pane order, growing the grid if needed.
fn apply_args(s: &mut Session, args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    let n = args.len().min(MAXP);
    for (i, p) in args.into_iter().take(MAXP).enumerate() {
        s.paths[i] = p;
    }
    if n > s.rows * s.cols {
        s.cols = (n as f64).sqrt().ceil() as usize;
        s.rows = n.div_ceil(s.cols);
        (s.colr, s.rowr) = (even(s.cols), even(s.rows));
    }
    s.active = 0;
}

fn ini_path() -> PathBuf {
    std::env::current_exe().unwrap_or_default().with_file_name("qdir.ini")
}

fn sessions_dir() -> PathBuf {
    ini_path().with_file_name("sessions")
}

fn human(b: u64) -> String {
    let (mut v, mut k) = (b as f64, 0);
    while v >= 1024.0 && k < 4 {
        v /= 1024.0;
        k += 1;
    }
    match k {
        0 => format!("{b} B"),
        _ => format!("{v:.1} {}", ["", "KB", "MB", "GB", "TB"][k]),
    }
}

/// "pdf; *.txt" -> "*pdf*;*.txt" for PathMatchSpec: plain words match anywhere in the name.
fn filter_spec(text: &str) -> String {
    let parts = text.split(';').map(str::trim).filter(|t| !t.is_empty());
    let parts = parts.map(|t| if t.contains(['*', '?']) { t.to_string() } else { format!("*{t}*") });
    parts.collect::<Vec<_>>().join(";")
}

// ---------- grid geometry ----------

/// Splits `len` px starting at `start` into cells by `ratios`, with `gap` px splitters between them.
fn cells(start: i32, len: i32, ratios: &[f64], gap: i32) -> Vec<(i32, i32)> {
    let avail = (len - gap * (ratios.len() as i32 - 1)).max(0) as f64;
    let mut acc = 0.0;
    let mut out = Vec::with_capacity(ratios.len());
    for (i, r) in ratios.iter().enumerate() {
        let a = start + (acc * avail).round() as i32 + i as i32 * gap;
        acc += r;
        out.push((a, start + (acc * avail).round() as i32 + i as i32 * gap));
    }
    out
}

/// Moves the boundary after cell `k` to `pos`, keeping both neighbours at least `min` px.
fn drag_ratio(r: &mut [f64], k: usize, pos: i32, start: i32, len: i32, gap: i32, min: i32) {
    let avail = (len - gap * (r.len() as i32 - 1)).max(1) as f64;
    let before: f64 = r[..k].iter().sum();
    let pair = r[k] + r[k + 1];
    let m = (min as f64 / avail).min(pair / 2.0);
    let t = ((pos - start - k as i32 * gap) as f64 / avail - before).clamp(m, pair - m);
    (r[k], r[k + 1]) = (t, pair - t);
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Side,
    Grid(Option<usize>, Option<usize>), // (column boundary, row boundary)
}

// Main window: toolbar on top, status line at the bottom, plates (sidebar + grid) in between.
// The gaps between plates are the splitters.
struct Geo {
    w: i32,
    h: i32,   // client height
    top: i32, // toolbar bottom = plates top
    bot: i32, // plates bottom = status line top
    pad: i32,
    gap: i32,
    side: i32, // sidebar plate width
    gx: i32,   // grid left
    xs: Vec<(i32, i32)>,
    ys: Vec<(i32, i32)>,
}

fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap, top) = (s(12), s(10), s(50));
    let bot = (rc.bottom - s(30)).max(top);
    let side = a.s.side.clamp(s(120), (rc.right - 2 * pad - gap - s(240)).max(s(120)));
    let gx = pad + side + gap;
    Geo {
        xs: cells(gx, rc.right - pad - gx, &a.s.colr, gap),
        ys: cells(top, bot - top, &a.s.rowr, gap),
        w: rc.right,
        h: rc.bottom,
        top,
        bot,
        pad,
        gap,
        side,
        gx,
    }
}

fn side_rect(g: &Geo) -> RECT {
    RECT { left: g.pad, top: g.top, right: g.pad + g.side, bottom: g.bot }
}

fn status_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.bot, right: g.w, bottom: g.h }
}

fn cell_rect(g: &Geo, i: usize, cols: usize) -> RECT {
    let ((x0, x1), (y0, y1)) = (g.xs[i % cols], g.ys[i / cols]);
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; the Explorer view below.
struct PaneGeo {
    btn: [RECT; 4], // back, forward, up, open-in
    addr: RECT,
    filt: RECT,
    view: RECT,
}

fn pane_geo(w: i32, h: i32, dpi: i32) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(46), s(30), s(30), s(8));
    let y = (bar - bh) / 2;
    let r = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let fw = s(180).min((w - 2 * pad - 4 * b) / 3).max(0);
    let filt = r(w - pad - fw, fw);
    let open = r(filt.left - s(4) - b, b);
    let addr_x = pad + 3 * b + s(6);
    PaneGeo {
        btn: [r(pad, b), r(pad + b, b), r(pad + 2 * b, b), open],
        addr: r(addr_x, open.left - s(6) - addr_x),
        filt,
        // inset so the square list sits inside the plate's rounded corners
        view: RECT { left: s(6), top: bar, right: w - s(6), bottom: h - s(6) },
    }
}

fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y < g.top || y >= g.bot {
        return None;
    }
    if x >= g.pad + g.side && x < g.gx {
        return Some(Hit::Side);
    }
    if x < g.gx {
        return None;
    }
    let between = |v: &[(i32, i32)], p: i32| v.windows(2).position(|w| p >= w[0].1 && p < w[1].0);
    let (c, r) = (between(&g.xs, x), between(&g.ys, y));
    (c.is_some() || r.is_some()).then_some(Hit::Grid(c, r))
}

// ---------- app state ----------
// Single UI thread. Rule: never call into Win32/COM that can re-enter our window procs while inside app().

struct Pane {
    hwnd: HWND,
    eb: IExplorerBrowser,
    cookie: u32,
    disp: String,
}

struct App {
    s: Session,
    main: HWND,
    tree: Option<INameSpaceTreeControl>,
    tree_hwnd: HWND,
    tree_input: bool, // a click/key reached the tree since its last selection change
    font: HFONT,
    icon_font: HFONT,
    fh: i32, // UI font line height in px
    pill_brush: HBRUSH,
    dpi: i32,
    panes: Vec<Option<Pane>>,
    drag: Option<Hit>,
    sizing: bool, // inside the window's own move/size loop
    status: String,
    status_key: String,
    sess_menu: HMENU,
    view_menu: HMENU,
    sess_files: Vec<PathBuf>,
    hover: HWND, // owner-drawn button under the mouse
    tip: HWND,
    grid: HWND, // layout picker popup while open
    grid_hover: (usize, usize),
    grid_closed: Option<Instant>,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        tree_input: false,
        font: HFONT::default(),
        icon_font: HFONT::default(),
        fh: 16,
        pill_brush: HBRUSH::default(),
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        sizing: false,
        status: String::new(),
        status_key: String::new(),
        sess_menu: HMENU::default(),
        view_menu: HMENU::default(),
        sess_files: Vec::new(),
        hover: HWND::default(),
        tip: HWND::default(),
        grid: HWND::default(),
        grid_hover: (0, 0),
        grid_closed: None,
    });
}

// Per-pane filter spec. Not in APP: the shell view may call IncludeObject from its own thread.
static FILTERS: Mutex<[String; MAXP]> = Mutex::new([const { String::new() }; MAXP]);

fn filter_of(i: usize) -> String {
    FILTERS.lock().map(|f| f[i].clone()).unwrap_or_default()
}

fn app<R>(f: impl FnOnce(&mut App) -> R) -> R {
    APP.with(|a| f(&mut a.borrow_mut()))
}

fn eb_of(i: usize) -> Option<IExplorerBrowser> {
    app(|a| a.panes.get(i)?.as_ref().map(|p| p.eb.clone()))
}

unsafe fn view_of<T: Interface>(i: usize) -> Option<T> {
    eb_of(i)?.GetCurrentView::<T>().ok()
}

fn pane_of(h: HWND) -> Option<usize> {
    app(|a| {
        a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == h || unsafe { IsChild(p.hwnd, h).as_bool() }))
    })
}

fn pane_ctl(i: usize, id: i32) -> Option<HWND> {
    let h = app(|a| a.panes.get(i)?.as_ref().map(|p| p.hwnd))?;
    unsafe { GetDlgItem(Some(h), id).ok() }
}

unsafe fn text_of(h: HWND) -> String {
    let mut buf = vec![0u16; GetWindowTextLengthW(h) as usize + 1];
    let n = GetWindowTextW(h, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..n])
}

// ---------- fonts ----------

fn logfont(face: &str, px: i32) -> LOGFONTW {
    let mut lf = LOGFONTW { lfHeight: -px, lfCharSet: DEFAULT_CHARSET, lfQuality: CLEARTYPE_QUALITY, ..Default::default() };
    for (d, c) in lf.lfFaceName.iter_mut().take(31).zip(face.encode_utf16()) {
        *d = c;
    }
    lf
}

// GDI silently substitutes missing faces, so ask what it actually picked.
unsafe fn font_exists(face: &str) -> bool {
    let f = CreateFontIndirectW(&logfont(face, 12));
    let dc = GetDC(None);
    let old = SelectObject(dc, f.into());
    let mut buf = [0u16; 64];
    let n = GetTextFaceW(dc, Some(&mut buf)).max(0) as usize;
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    _ = DeleteObject(f.into());
    String::from_utf16_lossy(&buf[..n.min(64)]).trim_end_matches('\0').eq_ignore_ascii_case(face)
}

unsafe fn ui_face(pref: &str) -> String {
    let faces = [pref, "Pretendard", "Pretendard Variable", "Segoe UI Variable Text", "Malgun Gothic"];
    faces.into_iter().find(|f| !f.is_empty() && font_exists(f)).unwrap_or("Segoe UI").to_string()
}

// Fonts dropped into `fonts\` next to the exe work without installing (portable).
unsafe fn load_private_fonts() {
    let entries = std::fs::read_dir(ini_path().with_file_name("fonts")).into_iter().flatten().flatten();
    for p in entries.map(|e| e.path()) {
        let ext = p.extension().map(|x| x.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["ttf", "otf", "ttc"].contains(&ext.as_str()) {
            AddFontResourceExW(&HSTRING::from(p.to_string_lossy().as_ref()), FR_PRIVATE, None);
        }
    }
}

// (Re)creates the UI + icon fonts for the current face/size/DPI and pushes them to every control.
unsafe fn apply_font() {
    let (pref, pt, dpi) = app(|a| (a.s.font.clone(), a.s.font_pt, a.dpi));
    let font = CreateFontIndirectW(&logfont(&ui_face(&pref), (pt * dpi + 36) / 72));
    let icons = if font_exists("Segoe Fluent Icons") { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    let icon_font = CreateFontIndirectW(&logfont(icons, 16 * dpi / 96));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    let (old, old_icon, panes, tree) = app(|a| {
        a.fh = tm.tmHeight;
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        (old, old_icon, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), a.tree_hwnd)
    });
    let set = |h: HWND| _ = SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    for p in panes {
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p), id) {
                set(c);
            }
        }
    }
    if let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) {
        set(tv);
    }
    for f in [old, old_icon].into_iter().filter(|f| !f.is_invalid()) {
        _ = DeleteObject(f.into());
    }
    relayout();
    redraw_all();
}

unsafe fn pick_font() {
    let (main, pref, pt) = app(|a| (a.main, a.s.font.clone(), a.s.font_pt));
    let mut lf = logfont(&ui_face(&pref), (pt * GetDpiForSystem() as i32 + 36) / 72);
    let mut cf = CHOOSEFONTW {
        lStructSize: size_of::<CHOOSEFONTW>() as u32,
        hwndOwner: main,
        lpLogFont: &mut lf,
        Flags: CF_SCREENFONTS | CF_INITTOLOGFONTSTRUCT | CF_NOVERTFONTS | CF_FORCEFONTEXIST | CF_NOSCRIPTSEL,
        ..Default::default()
    };
    if ChooseFontW(&mut cf).as_bool() {
        let face = String::from_utf16_lossy(&lf.lfFaceName).trim_end_matches('\0').to_string();
        app(|a| (a.s.font, a.s.font_pt) = (face, (cf.iPointSize / 10).clamp(7, 24)));
        apply_font();
    }
}

// ---------- drawing ----------

fn cref(rgb: u32) -> COLORREF {
    COLORREF((rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF))
}

fn argb(rgb: u32, a: u8) -> u32 {
    (a as u32) << 24 | rgb
}

/// RECT as (x, y, w, h), shrunk by `inset` on every side (negative grows it).
fn rf(r: RECT, inset: f32) -> (f32, f32, f32, f32) {
    let (w, h) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
    (r.left as f32 + inset, r.top as f32 + inset, w - 2.0 * inset, h - 2.0 * inset)
}

unsafe fn fill(dc: HDC, r: RECT, rgb: u32) {
    let b = CreateSolidBrush(cref(rgb));
    FillRect(dc, &r, b);
    _ = DeleteObject(b.into());
}

unsafe fn text(dc: HDC, mut r: RECT, s: &str, font: HFONT, rgb: u32, align: DRAW_TEXT_FORMAT) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    if w.is_empty() {
        return; // an empty Vec's dangling pointer crashes DrawTextW even with length 0
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(rgb));
    DrawTextW(dc, &mut w, &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
    SelectObject(dc, old);
}

/// Anti-aliased shapes via GDI+ (plain GDI can't do smooth rounded corners or alpha).
struct Gfx(*mut GpGraphics);

impl Gfx {
    unsafe fn new(dc: HDC) -> Gfx {
        let mut g = std::ptr::null_mut();
        GdipCreateFromHDC(dc, &mut g);
        GdipSetSmoothingMode(g, SmoothingModeAntiAlias);
        GdipSetPixelOffsetMode(g, PixelOffsetModeHalf);
        Gfx(g)
    }

    unsafe fn round(&self, (x, y, w, h): (f32, f32, f32, f32), rad: f32, fill: Option<u32>, stroke: Option<(u32, f32)>) {
        let d = (rad * 2.0).min(w).min(h).max(0.1);
        let mut p = std::ptr::null_mut();
        GdipCreatePath(FillModeAlternate, &mut p);
        GdipAddPathArc(p, x, y, d, d, 180.0, 90.0);
        GdipAddPathArc(p, x + w - d, y, d, d, 270.0, 90.0);
        GdipAddPathArc(p, x + w - d, y + h - d, d, d, 0.0, 90.0);
        GdipAddPathArc(p, x, y + h - d, d, d, 90.0, 90.0);
        GdipClosePathFigure(p);
        if let Some(c) = fill {
            let mut b = std::ptr::null_mut();
            GdipCreateSolidFill(c, &mut b);
            GdipFillPath(self.0, b.cast(), p);
            GdipDeleteBrush(b.cast());
        }
        if let Some((c, width)) = stroke {
            let mut pen = std::ptr::null_mut();
            GdipCreatePen1(c, width, UnitPixel, &mut pen);
            GdipDrawPath(self.0, pen, p);
            GdipDeletePen(pen);
        }
        GdipDeletePath(p);
    }

    /// Soft drop shadow: fading 1px rings just outside the plate, nudged down.
    unsafe fn shadow(&self, r: RECT, rad: f32) {
        for (i, a) in [20u8, 14, 10, 7, 4, 2].into_iter().enumerate() {
            let k = i as f32 + 1.0;
            let (x, y, w, h) = rf(r, 0.5 - k);
            self.round((x, y + 1.5, w, h), rad + k, None, Some((argb(SHADOW, a), 1.0)));
        }
    }
}

impl Drop for Gfx {
    fn drop(&mut self) {
        unsafe { GdipDeleteGraphics(self.0) };
    }
}

// Paints into an off-screen buffer and blits once: no flicker while plates are redrawn during a drag.
unsafe fn buffered(h: HWND, hdc: HDC, draw: impl FnOnce(HDC, RECT)) {
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let mut mem = HDC::default();
    let pb = BeginBufferedPaint(hdc, &rc, BPBF_TOPDOWNDIB, None, &mut mem);
    if pb == 0 {
        return draw(hdc, rc);
    }
    draw(mem, rc);
    _ = EndBufferedPaint(pb, true);
}

unsafe fn draw_button(d: &DRAWITEMSTRUCT) {
    let (hover, dpi, font, icon_font, main) = app(|a| (a.hover, a.dpi, a.font, a.icon_font, a.main));
    let on_main = GetParent(d.hwndItem).is_ok_and(|p| p == main);
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = hover == d.hwndItem;
    fill(dc, rc, if on_main { BG } else { CARD });
    if pressed || hot {
        let shade = match (on_main, pressed) {
            (true, true) => 0xD8DCE2,
            (true, false) => 0xE2E5EA,
            (false, true) => 0xE2E5EA,
            (false, false) => 0xEDEFF3,
        };
        Gfx::new(dc).round(rf(rc, 0.0), (8 * dpi / 96) as f32, Some(argb(shade, 255)), None);
    }
    let fg = if hot || pressed { TEXT } else { SUB };
    if !on_main {
        return text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
    }
    let s = |v: i32| v * dpi / 96;
    let icon = RECT { left: rc.left + s(8), right: rc.left + s(30), ..rc };
    text(dc, icon, glyph(id), icon_font, fg, DT_CENTER);
    text(dc, RECT { left: icon.right + s(4), ..rc }, &text_of(d.hwndItem), font, TEXT, DT_LEFT);
}

// Hover highlight for owner-drawn buttons (they have no hot state of their own).
unsafe fn set_hover(h: HWND) {
    let old = app(|a| std::mem::replace(&mut a.hover, h));
    if old == h {
        return;
    }
    for w in [old, h].into_iter().filter(|w| !w.0.is_null()) {
        _ = InvalidateRect(Some(w), None, false);
    }
    if !h.0.is_null() {
        let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
        _ = TrackMouseEvent(&mut t);
    }
}

unsafe fn is_icon_button(h: HWND) -> bool {
    let mut c = [0u16; 16];
    let n = GetClassNameW(h, &mut c) as usize;
    String::from_utf16_lossy(&c[..n]).eq_ignore_ascii_case("Button")
        && GetWindowLongPtrW(h, GWL_STYLE) as i32 & 0xF == BS_OWNERDRAW
}

unsafe fn add_tip(btn: HWND, tip: PCWSTR) {
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: GetParent(btn).unwrap_or_default(),
        uId: btn.0 as usize,
        lpszText: PWSTR(tip.0 as *mut u16),
        ..Default::default()
    };
    SendMessageW(app(|a| a.tip), TTM_ADDTOOLW, None, Some(LPARAM(&ti as *const _ as isize)));
}

unsafe fn pwstr(p: PWSTR) -> String {
    let s = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as _));
    s
}

unsafe fn name(pidl: *const ITEMIDLIST, kind: SIGDN) -> String {
    SHGetNameFromIDList(pidl, kind).map(|p| pwstr(p)).unwrap_or_default()
}

// ---------- navigation ----------

unsafe fn navigate(i: usize, path: &str) -> bool {
    let Some(eb) = eb_of(i) else { return false };
    let mut pidl = std::ptr::null_mut();
    if SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, 0, None).is_err() {
        return false;
    }
    let ok = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

unsafe fn browse(i: usize, flags: u32) {
    if let Some(eb) = eb_of(i) {
        _ = eb.BrowseToIDList(std::ptr::null(), flags);
    }
}

unsafe fn browse_item(i: usize, si: &IShellItem) {
    let Some(eb) = eb_of(i) else { return };
    if let Ok(pidl) = SHGetIDListFromObject(si) {
        _ = eb.BrowseToIDList(pidl, SBSP_ABSOLUTE);
        CoTaskMemFree(Some(pidl as _));
    }
}

unsafe fn focus_view(i: usize) {
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32);
        if let Ok(h) = sv.GetWindow() {
            _ = SetFocus(Some(h));
        }
    }
}

unsafe fn focus_ctl(i: usize, id: i32) -> bool {
    if let Some(e) = pane_ctl(i, id) {
        _ = SetFocus(Some(e));
        SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    true
}

unsafe fn go_edit(i: usize) {
    let Some(e) = pane_ctl(i, ID_EDIT) else { return };
    if navigate(i, text_of(e).trim().trim_matches('"')) {
        focus_view(i);
    } else {
        _ = MessageBeep(MB_ICONWARNING);
    }
}

// The selected folder (or, with none selected, the folder on display) of `src` opens in `dst`.
unsafe fn open_in(src: usize, dst: usize) {
    let Some(fv) = view_of::<IFolderView2>(src) else { return };
    let is_dir = |it: &IShellItem| it.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
    let sel = fv.GetSelection(false).and_then(|a| a.GetItemAt(0)).ok().filter(is_dir);
    if let Some(item) = sel.or_else(|| fv.GetFolder::<IShellItem>().ok()) {
        browse_item(dst, &item);
        set_active(dst);
        focus_view(dst);
    }
}

unsafe fn open_in_menu(h: HWND, src: usize) {
    let items = app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.rows * a.s.cols);
        live.filter(|(k, _)| *k != src).filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone()))).collect::<Vec<_>>()
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("선택한 폴더(없으면 현재 폴더)를 열 패널:"));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (k, disp) in &items {
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("패널 {}    {disp}", k + 1)));
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(src, ID_OPEN_IN) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 {
        open_in(src, pick as usize - 1);
    }
}

unsafe fn apply_filter(i: usize) {
    let Some(f) = pane_ctl(i, ID_FILTER) else { return };
    let spec = filter_spec(&text_of(f));
    let changed = FILTERS.lock().map(|mut fl| std::mem::replace(&mut fl[i], spec.clone()) != spec).unwrap_or(false);
    if let (true, Some(sv)) = (changed, view_of::<IShellView>(i)) {
        _ = sv.Refresh(); // re-enumerates, asking IncludeObject again
    }
}

// Pane-sticky view mode: remember it before leaving a folder, re-apply after arriving.
unsafe fn save_view(i: usize) {
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    if fv.GetViewModeAndIconSize(&mut m, &mut z).is_ok() {
        app(|a| a.s.views[i] = (m.0, z));
    }
}

unsafe fn on_nav(i: usize, pidl: *const ITEMIDLIST) {
    let parse = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let disp = name(pidl, SIGDN_DESKTOPABSOLUTEEDITING);
    let Some((h, eb, view)) = app(|a| {
        a.s.paths[i] = parse;
        let v = a.s.views[i];
        a.panes[i].as_mut().map(|p| {
            p.disp = disp.clone();
            (p.hwnd, p.eb.clone(), v)
        })
    }) else {
        return;
    };
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp));
    }
    if view.0 != 0 {
        if let Ok(fv) = eb.GetCurrentView::<IFolderView2>() {
            _ = fv.SetViewModeAndIconSize(FOLDERVIEWMODE(view.0), view.1);
        }
    }
}

#[implement(IExplorerBrowserEvents)]
struct Events(usize);

impl IExplorerBrowserEvents_Impl for Events_Impl {
    fn OnNavigationPending(&self, _: *const ITEMIDLIST) -> Result<()> {
        unsafe { save_view(self.0) };
        Ok(())
    }
    fn OnViewCreated(&self, _: Ref<IShellView>) -> Result<()> {
        Ok(())
    }
    fn OnNavigationComplete(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        unsafe { on_nav(self.0, pidl) };
        Ok(())
    }
    fn OnNavigationFailed(&self, _: *const ITEMIDLIST) -> Result<()> {
        Ok(())
    }
}

// Site of each Explorer control. It asks the site for ICommDlgBrowser, whose IncludeObject is the name filter.
#[implement(IServiceProvider, ICommDlgBrowser)]
struct Host(usize);

impl IServiceProvider_Impl for Host_Impl {
    fn QueryService(&self, service: *const GUID, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        unsafe {
            *ppv = std::ptr::null_mut();
            // SID_SExplorerBrowserFrame is defined as IID_ICommDlgBrowser.
            if *service == ICommDlgBrowser::IID {
                return self.to_interface::<ICommDlgBrowser>().query(riid, ppv).ok();
            }
        }
        Err(E_NOINTERFACE.into())
    }
}

impl ICommDlgBrowser_Impl for Host_Impl {
    fn OnDefaultCommand(&self, _: Ref<IShellView>) -> Result<()> {
        Err(S_FALSE.into()) // let the view do its default action (open)
    }
    fn OnStateChange(&self, _: Ref<IShellView>, _: u32) -> Result<()> {
        Ok(())
    }
    fn IncludeObject(&self, sv: Ref<IShellView>, pidl: *const ITEMIDLIST) -> Result<()> {
        let spec = filter_of(self.0);
        if spec.is_empty() {
            return Ok(());
        }
        unsafe {
            let sf: IShellFolder = sv.ok()?.cast::<IFolderView>()?.GetFolder()?;
            let mut attr = SFGAO_FOLDER.0;
            _ = sf.GetAttributesOf(&[pidl], &mut attr);
            if attr & SFGAO_FOLDER.0 != 0 {
                return Ok(()); // folders stay, so a filtered pane can still be navigated
            }
            let mut sr = STRRET::default();
            let mut name = PWSTR::null();
            sf.GetDisplayNameOf(pidl, SHGDNF(SHGDN_INFOLDER.0 | SHGDN_FORPARSING.0), &mut sr)?;
            StrRetToStrW(&mut sr, Some(pidl), &mut name)?;
            let hit = PathMatchSpecW(PCWSTR(name.0), &HSTRING::from(spec)).as_bool();
            CoTaskMemFree(Some(name.0 as _));
            if hit {
                Ok(())
            } else {
                Err(S_FALSE.into())
            }
        }
    }
}

// ---------- shell tree (sidebar) ----------

#[implement(INameSpaceTreeControlEvents)]
struct TreeEvents;

impl INameSpaceTreeControlEvents_Impl for TreeEvents_Impl {
    fn OnItemClick(&self, psi: Ref<IShellItem>, hit: u32, click: u32) -> Result<()> {
        // Re-clicking the already selected folder fires no selection change; navigate here instead.
        let on_item = hit & NSTCEHT_ONITEM.0 as u32 != 0 && click & NSTCECT_LBUTTON.0 as u32 != 0;
        if let (true, Some(si), Some(t)) = (on_item, psi.as_ref(), app(|a| a.tree.clone())) {
            if unsafe { t.GetItemState(si, NSTCIS_SELECTED.0 as u32) }.unwrap_or(0) != 0 {
                unsafe { browse_item(app(|a| a.s.active), si) };
            }
        }
        Err(S_FALSE.into()) // let the tree do its default handling
    }
    fn OnSelectionChanged(&self, sel: Ref<IShellItemArray>) -> Result<()> {
        // The tree selects its root on its own at startup; only follow selections right after user input.
        // (Focus is no good here: on a click the selection changes before the tree takes focus.)
        let user = app(|a| std::mem::take(&mut a.tree_input));
        if let Some(item) = sel.as_ref().filter(|_| user).and_then(|s| unsafe { s.GetItemAt(0) }.ok()) {
            unsafe { browse_item(app(|a| a.s.active), &item) };
        }
        Ok(())
    }
    fn OnPropertyItemCommit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemStateChanging(&self, _: Ref<IShellItem>, _: u32, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemStateChanged(&self, _: Ref<IShellItem>, _: u32, _: u32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnKeyboardInput(&self, _: u32, _: WPARAM, _: LPARAM) -> Result<()> {
        Err(S_FALSE.into())
    }
    fn OnBeforeExpand(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnAfterExpand(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeginLabelEdit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnEndLabelEdit(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetToolTip(&self, _: Ref<IShellItem>, _: PWSTR, _: i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeItemDelete(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemAdded(&self, _: Ref<IShellItem>, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnItemDeleted(&self, _: Ref<IShellItem>, _: BOOL) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeContextMenu(&self, _: Ref<IShellItem>, _: *const GUID, _: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnAfterContextMenu(&self, _: Ref<IShellItem>, _: Ref<IContextMenu>, _: *const GUID, _: *mut *mut core::ffi::c_void) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeStateImageChange(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetDefaultIconIndex(&self, _: Ref<IShellItem>, _: *mut i32, _: *mut i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

unsafe fn create_tree(main: HWND) -> Result<(INameSpaceTreeControl, HWND)> {
    let t: INameSpaceTreeControl = CoCreateInstance(&NamespaceTreeControl, None, CLSCTX_INPROC_SERVER)?;
    let style = NSTCS_HASEXPANDOS.0
        | NSTCS_ROOTHASEXPANDO.0
        | NSTCS_FADEINOUTEXPANDOS.0
        | NSTCS_NOINFOTIP.0
        | NSTCS_ALLOWJUNCTIONS.0
        | NSTCS_SHOWSELECTIONALWAYS.0
        | NSTCS_FULLROWSELECT.0
        | NSTCS_AUTOHSCROLL.0;
    t.Initialize(main, &RECT::default(), style as u32)?;
    let root: IShellItem = SHGetKnownFolderItem(&FOLDERID_Desktop, KF_FLAG_DEFAULT, None)?;
    t.AppendRoot(&root, SHCONTF_FOLDERS.0 as u32, NSTCRS_EXPANDED.0 as u32, None)?;
    let ev: INameSpaceTreeControlEvents = TreeEvents.into();
    t.TreeAdvise(&ev)?;
    let h = t.cast::<IOleWindow>()?.GetWindow()?;
    Ok((t, h))
}

// ---------- panes ----------

unsafe fn child(parent: HWND, class: PCWSTR, text: PCWSTR, style: u32, ex: WINDOW_EX_STYLE, id: i32, font: HFONT) -> HWND {
    let style = WS_CHILD | WS_VISIBLE | WINDOW_STYLE(style);
    let menu = HMENU(id as isize as _);
    let h = CreateWindowExW(ex, class, text, style, 0, 0, 0, 0, Some(parent), Some(menu), None, None).unwrap_or_default();
    SendMessageW(h, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
    h
}

unsafe fn create_pane(i: usize) -> Result<Pane> {
    let (main, font) = app(|a| (a.main, a.font));
    let style = WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN;
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("QDirPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    let buttons = [
        (ID_BACK, w!("뒤로"), w!("뒤로  (Alt+←)")),
        (ID_FWD, w!("앞으로"), w!("앞으로  (Alt+→)")),
        (ID_UP, w!("상위 폴더"), w!("상위 폴더  (Alt+↑)")),
        (ID_OPEN_IN, w!("다른 패널에서 열기"), w!("다른 패널에서 열기")),
    ];
    for (id, name, tip) in buttons {
        add_tip(child(h, w!("BUTTON"), name, btn, plain, id, font), tip);
    }
    child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    let f = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);
    SendMessageW(f, EM_SETCUEBANNER, Some(WPARAM(1)), Some(LPARAM(w!("필터  Ctrl+F").as_ptr() as isize)));

    let eb: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
    let site: IServiceProvider = Host(i).into();
    IUnknown_SetSite(&eb, &site)?; // before Initialize, so the view picks up our ICommDlgBrowser
    let fs = FOLDERSETTINGS { ViewMode: FVM_DETAILS.0 as u32, fFlags: FWF_NOWEBVIEW.0 as u32 };
    eb.Initialize(h, &RECT::default(), Some(&fs))?;
    eb.SetOptions(EBO_NOBORDER)?;
    let ev: IExplorerBrowserEvents = Events(i).into();
    let cookie = eb.Advise(&ev)?;
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new() })
}

unsafe fn destroy_pane(i: usize) {
    save_view(i);
    if let Some(p) = app(|a| a.panes[i].take()) {
        _ = p.eb.Unadvise(p.cookie);
        _ = IUnknown_SetSite(&p.eb, None);
        _ = p.eb.Destroy();
        _ = DestroyWindow(p.hwnd);
    }
    if let Ok(mut f) = FILTERS.lock() {
        f[i].clear();
    }
}

// Only the panes visible in the current grid exist; hidden slots keep just their path.
unsafe fn sync_panes() {
    let (main, n) = app(|a| (a.main, a.s.rows * a.s.cols));
    if main.0.is_null() {
        return;
    }
    for i in 0..MAXP {
        let has = app(|a| a.panes[i].is_some());
        if i < n && !has {
            let Ok(p) = create_pane(i) else { continue };
            app(|a| a.panes[i] = Some(p));
            let path = app(|a| a.s.paths[i].clone());
            if path.is_empty() || !navigate(i, &path) {
                navigate(i, THIS_PC);
            }
        } else if i >= n && has {
            destroy_pane(i);
        }
    }
}

// Moves a batch of windows at once (one repaint, no copied bits).
unsafe fn move_all(moves: &[(HWND, RECT)], eb: Option<(&IExplorerBrowser, RECT)>) {
    let Ok(mut dwp) = BeginDeferWindowPos(moves.len() as i32 + 1) else { return };
    for (h, r) in moves.iter().filter(|(h, _)| !h.0.is_null()) {
        match DeferWindowPos(dwp, *h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, MOVE) {
            Ok(d) => dwp = d,
            Err(_) => return,
        }
    }
    if let Some((eb, r)) = eb {
        _ = eb.SetRect(Some(&mut dwp as *mut HDWP), r);
    }
    _ = EndDeferWindowPos(dwp);
}

unsafe fn layout_pane(h: HWND) {
    let Some((eb, dpi)) = app(|a| a.panes.iter().flatten().find(|p| p.hwnd == h).map(|p| (p.eb.clone(), a.dpi))) else {
        return;
    };
    let fh = app(|a| a.fh) + 2;
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let s = |v: i32| v * dpi / 96;
    let pg = pane_geo(rc.right, rc.bottom, dpi);
    // borderless edit, vertically centred inside its pill
    let inside = |p: RECT, l: i32, r: i32| {
        let top = p.top + (p.bottom - p.top - fh) / 2;
        RECT { left: p.left + l, top, right: (p.right - r).max(p.left + l), bottom: top + fh }
    };
    let ctl = |id: i32| GetDlgItem(Some(h), id).unwrap_or_default();
    let mut moves: Vec<(HWND, RECT)> = [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].iter().zip(pg.btn).map(|(&id, r)| (ctl(id), r)).collect();
    moves.push((ctl(ID_EDIT), inside(pg.addr, s(14), s(12))));
    moves.push((ctl(ID_FILTER), inside(pg.filt, s(32), s(12))));
    move_all(&moves, Some((&eb, pg.view)));
}

unsafe fn relayout() {
    sync_panes();
    let Some((main, moves, label)) = app(|a| {
        if a.main.0.is_null() {
            return None;
        }
        let g = geo(a);
        let s = |v: i32| v * a.dpi / 96;
        let sr = side_rect(&g);
        let tree = RECT { left: sr.left + s(6), top: sr.top + s(8), right: sr.right - s(6), bottom: sr.bottom - s(6) };
        let mut moves = vec![(a.tree_hwnd, tree)];
        let mut x = g.pad;
        for (id, w) in [(ID_TB_LAYOUT, s(92)), (ID_TB_SESS, s(76)), (ID_TB_VIEW, s(76))] {
            let b = unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
            moves.push((b, RECT { left: x, top: s(9), right: x + w, bottom: s(9) + s(32) }));
            x += w + s(4);
        }
        for (i, p) in a.panes.iter().enumerate().take(a.s.rows * a.s.cols) {
            if let Some(p) = p {
                moves.push((p.hwnd, cell_rect(&g, i, a.s.cols)));
            }
        }
        Some((a.main, moves, format!("{} × {}", a.s.rows, a.s.cols)))
    }) else {
        return;
    };
    move_all(&moves, None);
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        if text_of(b) != label {
            _ = SetWindowTextW(b, &HSTRING::from(label));
        }
    }
    _ = InvalidateRect(Some(main), None, true);
    // Paint now: WM_PAINT is only generated once the message queue is empty, which never happens
    // while the mouse keeps moving, so mid-drag the views would keep showing the previous frame.
    _ = RedrawWindow(Some(main), None, None, RDW_UPDATENOW | RDW_ALLCHILDREN);
}

// Full repaint of everything once a drag/resize/layout change is over, so nothing stale survives.
unsafe fn redraw_all() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = RedrawWindow(Some(main), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

unsafe fn set_active(i: usize) {
    let (old, hs, main) = app(|a| {
        let old = std::mem::replace(&mut a.s.active, i);
        (old, [old, i].map(|k| a.panes[k].as_ref().map(|p| p.hwnd)), a.main)
    });
    if old != i {
        for h in hs.into_iter().flatten().chain([main]) {
            _ = InvalidateRect(Some(h), None, false); // pane ring + the glow around it on the main window
        }
    }
}

unsafe fn cycle(d: i32) {
    let (a, n) = app(|a| (a.s.active as i32, (a.s.rows * a.s.cols) as i32));
    let i = (a + d).rem_euclid(n) as usize;
    set_active(i);
    focus_view(i);
}

unsafe fn set_layout(r: usize, c: usize) {
    app(|a| {
        (a.s.rows, a.s.cols, a.s.colr, a.s.rowr) = (r, c, even(c), even(r));
        a.s.active = a.s.active.min(r * c - 1);
    });
    relayout();
    redraw_all();
}

// Same switch as Explorer's "Hidden items" (global shell setting).
unsafe fn hidden() -> bool {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 & 1 != 0
}

unsafe fn toggle_hidden() {
    let mut ss = SHELLSTATEA::default();
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, false);
    ss._bitfield1 ^= 1;
    SHGetSetSettings(Some(&mut ss), SSF_SHOWALLOBJECTS, true);
    for i in 0..MAXP {
        if let Some(sv) = view_of::<IShellView>(i) {
            _ = sv.Refresh();
        }
    }
}

// ---------- sessions ----------

unsafe fn save_session() {
    let (main, n) = app(|a| (a.main, a.s.rows * a.s.cols));
    for i in 0..n {
        save_view(i);
    }
    let mut wp = WINDOWPLACEMENT { length: size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    _ = GetWindowPlacement(main, &mut wp);
    let ini = app(|a| {
        let r = wp.rcNormalPosition;
        let max = (wp.showCmd == SW_SHOWMAXIMIZED.0 as u32) as i32;
        a.s.win = Some([r.left, r.top, r.right, r.bottom, max]);
        a.s.to_ini()
    });
    _ = std::fs::write(ini_path(), ini);
}

// Named session = the save dialog does naming, overwrite prompts, rename and delete for us.
unsafe fn save_session_as() -> Result<()> {
    let (main, n) = app(|a| (a.main, a.s.rows * a.s.cols));
    for i in 0..n {
        save_view(i);
    }
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
    dlg.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: w!("Q-Dir 세션 (*.qdr)"), pszSpec: w!("*.qdr") }])?;
    dlg.SetDefaultExtension(w!("qdr"))?;
    dlg.SetTitle(w!("세션 저장"))?;
    if let Ok(f) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.to_string_lossy().as_ref()), None) {
        dlg.SetFolder(&f)?;
    }
    dlg.Show(Some(main))?; // Err on cancel
    let path = pwstr(dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?);
    std::fs::write(path, app(|a| a.s.to_ini())).map_err(|_| Error::from(E_FAIL))
}

unsafe fn load_session_file(path: &Path) {
    let Ok(text) = std::fs::read_to_string(path) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    for i in 0..MAXP {
        destroy_pane(i);
    }
    let other = Session::parse(&text);
    app(|a| a.s.take_layout(&other));
    relayout();
    redraw_all();
    focus_view(app(|a| a.s.active));
}

unsafe fn fill_sessions(menu: HMENU) {
    for _ in 3..GetMenuItemCount(Some(menu)) {
        _ = DeleteMenu(menu, 3, MF_BYPOSITION);
    }
    let entries = std::fs::read_dir(sessions_dir()).into_iter().flatten().flatten();
    let mut files: Vec<PathBuf> =
        entries.map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("qdr"))).collect();
    files.sort();
    files.truncate(100);
    for (k, f) in files.iter().enumerate() {
        let label = f.file_stem().unwrap_or_default().to_string_lossy();
        _ = AppendMenuW(menu, MF_STRING, ID_SESS + k, &HSTRING::from(label.as_ref()));
    }
    if files.is_empty() {
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, w!("(저장된 세션 없음)"));
    }
    app(|a| a.sess_files = files);
}

// Paths from the command line or a second instance: a .qdr loads, folders fill panes from the active one on.
unsafe fn open_args(args: Vec<String>) {
    if let Some(f) = qdr(&args) {
        return load_session_file(Path::new(f));
    }
    let (a, n) = app(|x| (x.s.active, x.s.rows * x.s.cols));
    for (k, p) in args.iter().take(n).enumerate() {
        navigate((a + k) % n, p);
    }
    focus_view(a);
}

// Window of a running copy of this same exe. Portable copies elsewhere have their own ini and stay separate.
unsafe fn running_instance() -> Option<HWND> {
    let me = std::env::current_exe().ok()?.to_string_lossy().to_lowercase();
    let mut h = None;
    loop {
        let w = FindWindowExW(None, h, w!("QDirMain"), PCWSTR::null()).ok()?;
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
struct CopyData {
    dw: usize,
    cb: u32,
    lp: *const core::ffi::c_void,
}

// ---------- status bar ----------

unsafe fn selected_size(fv: &IFolderView2) -> u64 {
    let Ok(arr) = fv.GetSelection(false) else { return 0 };
    // ponytail: sums at most 10k selected items; bigger selections show a partial size
    let n = arr.GetCount().unwrap_or(0).min(10_000);
    let paths = (0..n).filter_map(|k| arr.GetItemAt(k).ok()).filter_map(|it| it.GetDisplayName(SIGDN_FILESYSPATH).ok());
    paths.map(|p| std::fs::metadata(pwstr(p)).map(|m| if m.is_file() { m.len() } else { 0 }).unwrap_or(0)).sum()
}

// Polled by a 300ms timer; the expensive parts only run when the active pane's state changed.
unsafe fn update_status() {
    let (i, path, main) = app(|a| (a.s.active, a.s.paths[a.s.active].clone(), a.main));
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let n = fv.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
    let sel = fv.ItemCount(SVGIO_SELECTION).unwrap_or(0);
    let spec = filter_of(i);
    let key = format!("{i}|{path}|{n}|{sel}|{}|{spec}", fv.GetFocusedItem().unwrap_or(-1));
    if app(|a| a.status_key == key) {
        return;
    }
    let mut parts = vec![format!("항목 {n}개")];
    if sel > 0 {
        parts.push(match selected_size(&fv) {
            0 => format!("{sel}개 선택"),
            size => format!("{sel}개 선택 ({})", human(size)),
        });
    }
    if !spec.is_empty() {
        parts.push(format!("필터 {spec}"));
    }
    let drive = path.get(1..2) == Some(":");
    if drive || path.starts_with(r"\\") {
        // ponytail: a dead network share can stall this call once per change
        let (mut free, mut total) = (0, 0);
        if GetDiskFreeSpaceExW(&HSTRING::from(&path), Some(&mut free), Some(&mut total), None).is_ok() {
            let label = if drive { &path[..2] } else { "네트워크" };
            parts.push(format!("{label} 여유 {} / {}", human(free), human(total)));
        }
    }
    let rc = app(|a| {
        (a.status_key, a.status) = (key, parts.join("     ·     "));
        status_rect(&geo(a))
    });
    _ = InvalidateRect(Some(main), Some(&rc), true);
}

// ---------- painting ----------

// Ground, plate shadows, the sidebar plate, the active plate's glow and the status line.
// The pane plates themselves are child windows painted by paint_pane.
unsafe fn paint_main(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let (g, cards, active, status, font, dpi) = app(|a| {
        let g = geo(a);
        let n = a.s.rows * a.s.cols;
        let cards: Vec<RECT> = (0..n).filter(|&i| a.panes[i].is_some()).map(|i| cell_rect(&g, i, a.s.cols)).collect();
        let active = (a.s.active < n).then(|| cell_rect(&g, a.s.active, a.s.cols));
        (g, cards, active, a.status.clone(), a.font, a.dpi)
    });
    buffered(h, hdc, |dc, rc| {
        let rad = (12 * dpi / 96) as f32;
        fill(dc, rc, BG);
        let gfx = Gfx::new(dc);
        let side = side_rect(&g);
        gfx.shadow(side, rad);
        gfx.round(rf(side, 0.5), rad, Some(argb(CARD, 255)), Some((argb(LINE, 255), 1.0)));
        for c in &cards {
            gfx.shadow(*c, rad);
        }
        if let Some(c) = active {
            gfx.round(rf(c, -2.0), rad + 2.0, None, Some((argb(ACCENT, 0x38), 4.0)));
        }
        drop(gfx);
        let st = status_rect(&g);
        text(dc, RECT { left: g.pad + 4 * dpi / 96, ..st }, &status, font, SUB, DT_LEFT);
    });
    _ = EndPaint(h, &ps);
}

// One white rounded plate per pane: a single accent ring when active, pill fields in the top bar.
unsafe fn paint_pane(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let (active, dpi, icon_font) = app(|a| (a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h), a.dpi, a.icon_font));
    let focus = GetFocus();
    let focused = |id: i32| GetDlgItem(Some(h), id).is_ok_and(|e| e == focus);
    let (addr_on, filt_on) = (focused(ID_EDIT), focused(ID_FILTER));
    buffered(h, hdc, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let rad = s(12) as f32;
        let pg = pane_geo(rc.right, rc.bottom, dpi);
        fill(dc, rc, BG);
        let gfx = Gfx::new(dc);
        gfx.round(rf(rc, 0.5), rad, Some(argb(CARD, 255)), Some((argb(LINE, 255), 1.0)));
        if active {
            gfx.round(rf(rc, 1.0), rad - 0.5, None, Some((argb(ACCENT, 255), 2.0)));
        }
        for (p, on) in [(pg.addr, addr_on), (pg.filt, filt_on)] {
            let r = rf(p, 0.0);
            gfx.round(r, r.3 / 2.0, Some(argb(PILL, 255)), on.then_some((argb(ACCENT, 0xB0), 1.5)));
        }
        drop(gfx);
        fill(dc, RECT { left: pg.view.left + s(8), top: pg.view.top - 1, right: pg.view.right - s(8), bottom: pg.view.top }, LINE);
        let icon = RECT { left: pg.filt.left + s(10), right: pg.filt.left + s(28), ..pg.filt };
        text(dc, icon, ICON_SEARCH, icon_font, SUB, DT_CENTER);
    });
    _ = EndPaint(h, &ps);
}

// ---------- layout picker (hover a rows x cols block, click) ----------

fn grid_metrics(dpi: i32) -> (i32, i32, i32, i32) {
    let s = |v: i32| v * dpi / 96;
    let (pad, cell, gap) = (s(14), s(26), s(6));
    let w = 2 * pad + 4 * cell + 3 * gap;
    (w, w + s(18), cell, gap) // extra room below for the label
}

fn grid_cell(r: usize, c: usize, dpi: i32) -> RECT {
    let (_, _, cell, gap) = grid_metrics(dpi);
    let pad = 14 * dpi / 96;
    let (x, y) = (pad + c as i32 * (cell + gap), pad + r as i32 * (cell + gap));
    RECT { left: x, top: y, right: x + cell, bottom: y + cell }
}

unsafe fn open_grid() {
    let (main, dpi, open, closed, cur) =
        app(|a| (a.main, a.dpi, a.grid, a.grid_closed, (a.s.rows - 1, a.s.cols - 1)));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        _ = GetWindowRect(b, &mut rc);
    }
    let (w, h, _, _) = grid_metrics(dpi);
    app(|a| a.grid_hover = cur);
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(g) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("QDirGrid"), w!(""), style, rc.left, y, w, h, Some(main), None, None, None) else {
        return;
    };
    let round = DWMWCP_ROUND;
    _ = DwmSetWindowAttribute(g, DWMWA_WINDOW_CORNER_PREFERENCE, &round as *const _ as _, 4);
    app(|a| a.grid = g);
}

unsafe fn paint_grid(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let ((hr, hc), (cr, cc), dpi, font) = app(|a| (a.grid_hover, (a.s.rows - 1, a.s.cols - 1), a.dpi, a.font));
    buffered(h, hdc, |dc, rc| {
        fill(dc, rc, CARD);
        let gfx = Gfx::new(dc);
        for r in 0..4 {
            for c in 0..4 {
                let col = if r <= hr && c <= hc {
                    ACCENT
                } else if r <= cr && c <= cc {
                    ACCENT_SOFT
                } else {
                    PILL
                };
                gfx.round(rf(grid_cell(r, c, dpi), 0.0), (6 * dpi / 96) as f32, Some(argb(col, 255)), None);
            }
        }
        drop(gfx);
        let label = RECT { top: grid_cell(3, 0, dpi).bottom, bottom: rc.bottom - 4 * dpi / 96, ..rc };
        text(dc, label, &format!("{}행 × {}열", hr + 1, hc + 1), font, TEXT, DT_CENTER);
    });
    _ = EndPaint(h, &ps);
}

extern "system" fn grid_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let set = |r: usize, c: usize| {
            if app(|a| std::mem::replace(&mut a.grid_hover, (r.min(3), c.min(3)))) != (r.min(3), c.min(3)) {
                _ = InvalidateRect(Some(h), None, false);
            }
        };
        match msg {
            WM_PAINT => paint_grid(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let (dpi, (_, _, cell, gap)) = app(|a| (a.dpi, grid_metrics(a.dpi)));
                let at = |v: i32| ((v - 14 * dpi / 96).max(0) / (cell + gap)) as usize;
                set(at(y), at(x));
            }
            WM_LBUTTONUP => {
                let (r, c) = app(|a| a.grid_hover);
                _ = DestroyWindow(h);
                set_layout(r + 1, c + 1);
            }
            WM_KEYDOWN => {
                let (r, c) = app(|a| a.grid_hover);
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_LEFT => set(r, c.saturating_sub(1)),
                    VK_RIGHT => set(r, c + 1),
                    VK_UP => set(r.saturating_sub(1), c),
                    VK_DOWN => set(r + 1, c),
                    VK_RETURN => {
                        _ = DestroyWindow(h);
                        set_layout(r + 1, c + 1);
                    }
                    VK_ESCAPE => _ = DestroyWindow(h),
                    _ => {}
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.grid, a.grid_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

unsafe fn popup_menu(menu: HMENU, id: i32) {
    let main = app(|a| a.main);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), id) {
        _ = GetWindowRect(b, &mut rc);
    }
    _ = TrackPopupMenu(menu, TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom + 4, None, main, None);
}

// ---------- keyboard ----------

unsafe fn key(m: &MSG) -> bool {
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = app(|a| a.s.active);
    let addr = pane_ctl(a, ID_EDIT) == Some(m.hwnd);
    let filt = pane_ctl(a, ID_FILTER) == Some(m.hwnd);
    match VIRTUAL_KEY(m.wParam.0 as u16) {
        VK_TAB if ctrl => cycle(if shift { -1 } else { 1 }),
        VK_LEFT if alt => browse(a, SBSP_NAVIGATEBACK),
        VK_RIGHT if alt => browse(a, SBSP_NAVIGATEFORWARD),
        VK_UP if alt => browse(a, SBSP_PARENT),
        VK_L if ctrl => return focus_ctl(a, ID_EDIT),
        VK_D if alt => return focus_ctl(a, ID_EDIT),
        VK_F4 if !alt => return focus_ctl(a, ID_EDIT),
        VK_F if ctrl => return focus_ctl(a, ID_FILTER),
        VK_H if ctrl => toggle_hidden(),
        VK_RETURN if addr => go_edit(a),
        VK_RETURN if filt => {
            apply_filter(a);
            focus_view(a);
        }
        VK_ESCAPE if addr => {
            let disp = app(|x| x.panes[a].as_ref().map(|p| p.disp.clone())).unwrap_or_default();
            _ = SetWindowTextW(m.hwnd, &HSTRING::from(disp));
            focus_view(a);
        }
        VK_ESCAPE if filt => {
            _ = SetWindowTextW(m.hwnd, w!(""));
            apply_filter(a);
            focus_view(a);
        }
        _ if addr || filt => return false,
        _ => {
            // F2, Del, Ctrl+C/V ... are the shell view's accelerators: hand them over.
            let Some(sv) = view_of::<IShellView>(a) else { return false };
            let svh = sv.GetWindow().unwrap_or_default();
            return (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool()) && sv.TranslateAccelerator(m) == S_OK;
        }
    }
    true
}

unsafe fn pre_translate(m: &MSG) -> bool {
    if m.message == WM_LBUTTONDOWN || m.message == WM_KEYDOWN {
        app(|a| a.tree_input |= a.tree_hwnd == m.hwnd || IsChild(a.tree_hwnd, m.hwnd).as_bool());
    }
    match m.message {
        WM_MOUSEMOVE => {
            set_hover(if is_icon_button(m.hwnd) { m.hwnd } else { HWND::default() });
            false
        }
        WM_MOUSELEAVE => {
            if app(|a| a.hover == m.hwnd) {
                set_hover(HWND::default());
            }
            false
        }
        WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
            if let Some(i) = pane_of(m.hwnd) {
                set_active(i);
            }
            false
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => key(m),
        _ => false,
    }
}

// ---------- window procs ----------

fn xy(lp: LPARAM) -> (i32, i32) {
    ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32)
}

unsafe fn cursor_for(h: HWND) -> Option<Hit> {
    let mut pt = POINT::default();
    _ = GetCursorPos(&mut pt);
    _ = ScreenToClient(h, &mut pt);
    app(|a| hit(&geo(a), pt.x, pt.y))
}

extern "system" fn main_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_SIZE => {
                relayout();
                if !app(|a| a.sizing) {
                    redraw_all(); // maximize / restore
                }
            }
            WM_ENTERSIZEMOVE => app(|a| a.sizing = true),
            WM_EXITSIZEMOVE => {
                app(|a| a.sizing = false);
                redraw_all();
            }
            WM_PAINT => paint_main(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_main covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_TIMER if wp.0 == TIMER_STATUS => update_status(),
            WM_SETCURSOR if wp.0 == h.0 as usize && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let Some(d) = app(|a| a.drag).or_else(|| cursor_for(h)) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                let c = match d {
                    Hit::Side | Hit::Grid(Some(_), None) => IDC_SIZEWE,
                    Hit::Grid(None, Some(_)) => IDC_SIZENS,
                    _ => IDC_SIZEALL,
                };
                SetCursor(LoadCursorW(None, c).ok());
                return LRESULT(1);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| hit(&geo(a), x, y)) {
                    app(|a| a.drag = Some(d));
                    SetCapture(h);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let moved = app(|a| {
                    let d = a.drag?;
                    let g = geo(a);
                    let min = 60 * a.dpi / 96;
                    match d {
                        Hit::Side => a.s.side = x - g.gap / 2,
                        Hit::Grid(c, r) => {
                            if let Some(k) = c {
                                drag_ratio(&mut a.s.colr, k, x - g.gap / 2, g.gx, g.w - g.gx, g.gap, min);
                            }
                            if let Some(k) = r {
                                drag_ratio(&mut a.s.rowr, k, y - g.gap / 2, 0, g.h, g.gap, min);
                            }
                        }
                    }
                    a.s.side = geo(a).side;
                    Some(())
                });
                if moved.is_some() {
                    relayout();
                }
            }
            WM_LBUTTONUP => {
                let dragged = app(|a| a.drag.take()).is_some();
                _ = ReleaseCapture();
                if dragged {
                    redraw_all();
                }
            }
            WM_CAPTURECHANGED => app(|a| a.drag = None),
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                let d = app(|a| hit(&geo(a), x, y));
                app(|a| match d {
                    Some(Hit::Side) => a.s.side = 240 * a.dpi / 96,
                    Some(Hit::Grid(c, r)) => {
                        if c.is_some() {
                            a.s.colr = even(a.s.cols);
                        }
                        if r.is_some() {
                            a.s.rowr = even(a.s.rows);
                        }
                    }
                    None => {}
                });
                relayout();
                redraw_all();
            }
            WM_COMMAND => match wp.0 & 0xffff {
                id if id == ID_TB_LAYOUT as usize => open_grid(),
                id if id == ID_TB_SESS as usize => popup_menu(app(|a| a.sess_menu), ID_TB_SESS),
                id if id == ID_TB_VIEW as usize => popup_menu(app(|a| a.view_menu), ID_TB_VIEW),
                ID_HIDDEN => toggle_hidden(),
                ID_FONT => pick_font(),
                ID_KEYS => _ = MessageBoxW(Some(h), &HSTRING::from(KEYS_HELP), w!("단축키"), MB_OK),
                ID_SESS_SAVE => _ = save_session_as(),
                ID_SESS_DIR => {
                    let d = sessions_dir();
                    _ = std::fs::create_dir_all(&d);
                    navigate(app(|a| a.s.active), &d.to_string_lossy());
                }
                id @ ID_SESS..=1399 => {
                    if let Some(f) = app(|a| a.sess_files.get(id - ID_SESS).cloned()) {
                        load_session_file(&f);
                    }
                }
                _ => {}
            },
            WM_INITMENUPOPUP => {
                if HMENU(wp.0 as _) == app(|a| a.sess_menu) {
                    fill_sessions(HMENU(wp.0 as _));
                }
                let check = if hidden() { MF_CHECKED } else { MF_UNCHECKED };
                CheckMenuItem(app(|a| a.view_menu), ID_HIDDEN as u32, (MF_BYCOMMAND | check).0);
            }
            WM_COPYDATA => {
                let cd = &*(lp.0 as *const CopyData);
                if cd.dw != COPY_ID || cd.lp.is_null() {
                    return LRESULT(0);
                }
                // copy first: the sender's buffer is only valid during this call
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(cd.lp as *const u16, cd.cb as usize / 2));
                open_args(text.split('\n').map(String::from).collect());
                return LRESULT(1);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as i32;
                app(|a| {
                    a.s.side = a.s.side * dpi / a.dpi;
                    a.dpi = dpi;
                });
                apply_font();
                let r = &*(lp.0 as *const RECT);
                _ = SetWindowPos(h, None, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            WM_CLOSE => {
                save_session();
                _ = DestroyWindow(h);
            }
            WM_DESTROY => {
                _ = KillTimer(Some(h), TIMER_STATUS);
                for i in 0..MAXP {
                    destroy_pane(i);
                }
                app(|a| a.tree = None);
                PostQuitMessage(0);
            }
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

extern "system" fn pane_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let nav = |flags: u32| {
            if let Some(i) = pane_of(h) {
                set_active(i);
                browse(i, flags);
                focus_view(i);
            }
        };
        match msg {
            WM_SIZE => layout_pane(h),
            WM_PAINT => paint_pane(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_pane covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            // borderless edits sit on the pill colour
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(PILL));
                SetTextColor(dc, cref(TEXT));
                return LRESULT(app(|a| a.pill_brush).0 as isize);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
                // pill gets an accent outline while its edit has focus
                (ID_EDIT | ID_FILTER, EN_SETFOCUS | EN_KILLFOCUS) => _ = InvalidateRect(Some(h), None, false),
                (ID_BACK, _) => nav(SBSP_NAVIGATEBACK),
                (ID_FWD, _) => nav(SBSP_NAVIGATEFORWARD),
                (ID_UP, _) => nav(SBSP_PARENT),
                (ID_OPEN_IN, _) => {
                    if let Some(i) = pane_of(h) {
                        open_in_menu(h, i);
                    }
                }
                // typing in the filter box applies after a short pause
                (ID_FILTER, EN_CHANGE) => _ = SetTimer(Some(h), TIMER_FILTER, 250, None),
                _ => {}
            },
            WM_TIMER if wp.0 == TIMER_FILTER => {
                _ = KillTimer(Some(h), TIMER_FILTER);
                if let Some(i) = pane_of(h) {
                    apply_filter(i);
                }
            }
            // mouse side buttons
            WM_APPCOMMAND => match (lp.0 >> 16) & 0x0fff {
                1 => nav(SBSP_NAVIGATEBACK),
                2 => nav(SBSP_NAVIGATEFORWARD),
                _ => return DefWindowProcW(h, msg, wp, lp),
            },
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// Popups opened from the top toolbar (no classic menu bar).
unsafe fn make_menus() -> Result<(HMENU, HMENU)> {
    let sess = CreatePopupMenu()?;
    AppendMenuW(sess, MF_STRING, ID_SESS_SAVE, w!("현재 세션 저장..."))?;
    AppendMenuW(sess, MF_STRING, ID_SESS_DIR, w!("세션 폴더 열기"))?;
    AppendMenuW(sess, MF_SEPARATOR, 0, PCWSTR::null())?; // saved sessions are listed below on open
    let view = CreatePopupMenu()?;
    AppendMenuW(view, MF_STRING, ID_HIDDEN, w!("숨김 파일 표시\tCtrl+H"))?;
    AppendMenuW(view, MF_STRING, ID_FONT, w!("글꼴..."))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    AppendMenuW(view, MF_STRING, ID_KEYS, w!("단축키..."))?;
    Ok((sess, view))
}

fn main() -> Result<()> {
    unsafe {
        let args: Vec<String> = std::env::args().skip(1).map(resolve).collect();
        // Single instance: hand the arguments to the running window and leave.
        if let Some(prev) = running_instance() {
            if !args.is_empty() {
                let data: Vec<u16> = args.join("\n").encode_utf16().collect();
                let cd = CopyData { dw: COPY_ID, cb: (data.len() * 2) as u32, lp: data.as_ptr() as _ };
                let lp = LPARAM(&cd as *const CopyData as isize);
                SendMessageTimeoutW(prev, WM_COPYDATA, WPARAM(0), lp, SMTO_ABORTIFHUNG, 5000, None);
            }
            if IsIconic(prev).as_bool() {
                _ = ShowWindow(prev, SW_RESTORE);
            }
            _ = SetForegroundWindow(prev);
            return Ok(());
        }

        OleInitialize(None)?; // OLE (not just COM) for drag & drop
        let mut s = std::fs::read_to_string(ini_path()).map(|t| Session::parse(&t)).unwrap_or_default();
        match qdr(&args).and_then(|f| std::fs::read_to_string(f).ok()) {
            Some(t) => s.take_layout(&Session::parse(&t)),
            None => apply_args(&mut s, args),
        }

        let mut token = 0usize;
        GdiplusStartup(&mut token, &GdiplusStartupInput { GdiplusVersion: 1, ..Default::default() }, std::ptr::null_mut());
        _ = BufferedPaintInit();
        load_private_fonts();

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("QDirMain"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW, // the plate is drawn to the window size
            lpfnWndProc: Some(pane_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("QDirPane"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(grid_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("QDirGrid"),
            ..Default::default()
        });
        let (sess_menu, view_menu) = make_menus()?;
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("QDirMain"),
            w!("Q-Dir.rs"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1400,
            900,
            None,
            None,
            Some(hinst),
            None,
        )?;
        // caption in the ground colour, so title bar and toolbar read as one surface
        let caption = cref(BG);
        _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
        let dpi = GetDpiForWindow(main) as i32;
        if s.side <= 0 {
            s.side = 240 * dpi / 96;
        }
        let win = s.win;
        let tip_style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02); // TTS_ALWAYSTIP | TTS_NOPREFIX
        let tip = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, tip_style, 0, 0, 0, 0, Some(main), None, None, None)?;
        app(|a| {
            (a.s, a.main, a.dpi, a.sess_menu, a.view_menu, a.tip) = (s, main, dpi, sess_menu, view_menu, tip);
            a.pill_brush = CreateSolidBrush(cref(PILL));
        });
        let tb = [(ID_TB_LAYOUT, w!(""), w!("레이아웃 (행×열)")), (ID_TB_SESS, w!("세션"), w!("세션 저장·불러오기")), (ID_TB_VIEW, w!("보기"), w!("보기·글꼴·단축키"))];
        for (id, label, hint) in tb {
            add_tip(child(main, w!("BUTTON"), label, BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), id, HFONT::default()), hint);
        }
        if let Ok((t, th)) = create_tree(main) {
            app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
        }
        apply_font();
        match win {
            Some([l, t, r, b, max]) => {
                let wp = WINDOWPLACEMENT {
                    length: size_of::<WINDOWPLACEMENT>() as u32,
                    showCmd: if max != 0 { SW_SHOWMAXIMIZED.0 as u32 } else { SW_SHOWNORMAL.0 as u32 },
                    rcNormalPosition: RECT { left: l, top: t, right: r, bottom: b },
                    ..Default::default()
                };
                _ = SetWindowPlacement(main, &wp);
            }
            None => _ = ShowWindow(main, SW_SHOWNORMAL),
        }
        relayout();
        focus_view(app(|a| a.s.active));
        SetTimer(Some(main), TIMER_STATUS, 300, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !pre_translate(&msg) {
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_layout_args() {
        let mut s = Session { rows: 2, cols: 3, colr: vec![0.25, 0.25, 0.5], side: 200, active: 4, font_pt: 11, ..Default::default() };
        s.font = "Pretendard".into();
        s.win = Some([1, 2, 300, 400, 1]);
        s.paths[1] = r"C:\a=b\폴더".into();
        s.views[1] = (4, 16);
        assert_eq!(Session::parse(&s.to_ini()), s);
        // garbage in the ini falls back to sane values
        let bad = Session::parse("grid=9,0\ncolr=1,-1\nactive=99\nv3=x\np99=C:\\");
        assert_eq!((bad.rows, bad.cols, bad.colr.clone(), bad.active), (4, 1, vec![1.0], 0));
        // a named session brings grid + folders but not the window
        let mut cur = Session { side: 300, win: Some([0, 0, 9, 9, 0]), ..Default::default() };
        cur.take_layout(&s);
        assert_eq!((cur.rows, cur.cols, cur.paths[1].as_str(), cur.side, cur.win), (2, 3, s.paths[1].as_str(), 300, Some([0, 0, 9, 9, 0])));

        // cells tile the length exactly, with `gap` between neighbours
        let c = cells(10, 305, &[0.25, 0.25, 0.5], 5);
        assert_eq!((c[0].0, c[2].1), (10, 315));
        assert!(c.windows(2).all(|w| w[0].1 + 5 == w[1].0));

        // dragging respects the minimum size and keeps the pair's total
        let mut r = vec![0.5, 0.5];
        drag_ratio(&mut r, 0, -1000, 0, 205, 5, 20);
        assert_eq!(r[0], 0.1);
        assert!((r[0] + r[1] - 1.0).abs() < 1e-12);

        let mut s = Session::default();
        apply_args(&mut s, (0..5).map(|i| format!("::{{x{i}}}")).collect());
        assert_eq!((s.rows, s.cols, s.paths[4].as_str()), (2, 3, "::{x4}"));
        assert_eq!(qdr(&["a.QDR".into()]), Some("a.QDR"));
        assert_eq!(qdr(&["a.qdr".into(), "b".into()]), None);

        assert_eq!(filter_spec(" pdf; *.TXT ;"), "*pdf*;*.TXT");
        assert_eq!((human(10), human(1536), human(5 << 30)), ("10 B".into(), "1.5 KB".into(), "5.0 GB".into()));

        // pane top bar: back, fwd, up, address, ⇄, filter left to right, nothing overlapping, inside the plate
        for (w, dpi) in [(800, 96), (260, 96), (1200, 144)] {
            let p = pane_geo(w, 500, dpi);
            let row = [p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt];
            assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
            assert!(p.filt.right <= w && p.view.top >= p.btn[0].bottom);
        }
    }
}
