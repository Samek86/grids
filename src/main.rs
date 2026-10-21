// Grids: multi-pane file manager. Shell tree on the left, up to 4 columns of up to 4 Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWINDOWATTRIBUTE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::SFGAO_FOLDER;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Controls::Dialogs::{ChooseColorW, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW};
use windows::Win32::UI::Controls::{
    SetWindowTheme, BeginBufferedPaint, BufferedPaintInit, EndBufferedPaint, BPBF_TOPDOWNDIB, DRAWITEMSTRUCT, ODS_SELECTED, TOOLTIPS_CLASSW,
    CDDS_ITEMPREPAINT, CDDS_PREPAINT, CDDS_SUBITEM, CDIS_HOT, CDIS_SELECTED, CDRF_NOTIFYITEMDRAW, CDRF_NOTIFYSUBITEMDRAW, CDRF_SKIPDEFAULT,
    HDF_CENTER, HDF_RIGHT, HDF_SORTDOWN, HDF_SORTUP, HDITEMW, HDI_FORMAT, HDI_TEXT, HDM_GETITEMW, HDN_BEGINTRACKW, HDN_ITEMCHANGINGW, HDN_TRACKW, HDI_WIDTH, NMHEADERW, LVCF_MINWIDTH, LVCOLUMNW, LVM_GETCOLUMNW, LVM_SETCOLUMNW,
    LVM_GETCOLUMNWIDTH, LVM_SETCOLUMNWIDTH, NMCUSTOMDRAW, NMHDR, NMLVCUSTOMDRAW, NM_CUSTOMDRAW,
    TTF_IDISHWND, TTF_SUBCLASS, TTM_ADDTOOLW, TTM_UPDATETIPTEXTW, TTTOOLINFOW, WM_MOUSELEAVE,
};
use windows::Win32::UI::HiDpi::{GetDpiForWindow, SystemParametersInfoForDpi};
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
const ID_FAV: i32 = 107; // ☆+ / ★: add or remove the pane's folder from favourites
// top toolbar buttons (children of the main window)
const ID_TB_LAYOUT: i32 = 201;
const ID_TB_SAVE: i32 = 202;
const ID_TB_LOAD: i32 = 204;
const ID_TB_THEME: i32 = 205;
const ID_TB_FAV: i32 = 206;
const ID_TB_VIEW: i32 = 203;
// menu ids
const ID_HIDDEN: usize = 1100;
const ID_KEYS: usize = 1101;
const ID_DBLUP: usize = 1103;
const ID_LANG: usize = 1104;
const ID_HELP: usize = 1105;
const ID_ABOUT: usize = 1106;
const HOME: &str = "https://github.com/efww/grids";
const ID_ACCENT: usize = 1400; // + preset index
const ID_ACCENT_PICK: usize = 1410;
const EM_SETSEL: u32 = 0xB1;
const EN_CHANGE: u32 = 0x0300;
const TIMER_STATUS: usize = 1;
const TIMER_FILTER: usize = 2;
const TIMER_REVEAL: usize = 3;
const TIMER_SETTLE: usize = 4; // on a pane: narrow column widths going back on after a folder change
const TIMER_NEWFILE: usize = 5; // on a pane: select and rename a file just created
// tree-view messages (the sidebar's SysTreeView32)
const TVM_GETNEXTITEM: u32 = 0x110A;
const TVM_ENSUREVISIBLE: u32 = 0x1114;
const TVM_GETITEMRECT: u32 = 0x1104;
const TVGN_CARET: usize = 9;
const TVM_SETBKCOLOR: u32 = 0x111D;
// list-view messages (the Explorer list inside each pane)
const LVM_SETBKCOLOR: u32 = 0x1001;
const LVM_GETHEADER: u32 = 0x101F;
const LVM_GETCOLUMNORDERARRAY: u32 = 0x103B;
const LVM_SETEXTENDEDLISTVIEWSTYLE: u32 = 0x1036;
const LVM_GETEXTENDEDLISTVIEWSTYLE: u32 = 0x1037;
const LVS_EX_AUTOSIZECOLUMNS: usize = 0x1000_0000;
const LVS_EX_COLUMNSNAPPOINTS: usize = 0x4000_0000;
const HDM_GETITEMCOUNT: u32 = 0x1200;
const LVM_SETTEXTCOLOR: u32 = 0x1024;
const LVM_SETTEXTBKCOLOR: u32 = 0x1026;
const LVM_SETSELECTEDCOLUMN: u32 = 0x108C;
const LVM_GETSELECTEDCOLUMN: u32 = 0x10AE;
const TVM_SETTEXTCOLOR: u32 = 0x111E;
const COPY_ID: usize = 0x4752_4944; // "GRID": WM_COPYDATA from a second instance
// Move without copying old pixels: copied bits are what left ghost borders/buttons while resizing.
const MOVE: SET_WINDOW_POS_FLAGS = SET_WINDOW_POS_FLAGS(SWP_NOZORDER.0 | SWP_NOACTIVATE.0 | SWP_NOCOPYBITS.0);

// Themes, 0xRRGGBB. Light: an ivory frame around white plates; dark: charcoal. Each theme has its own
// selection colours (same names, tuned to the ground): pastels on light, muted mid tones on dark.
struct Palette {
    bg: u32,   // frame: window, toolbar, gaps, status line
    card: u32, // plates and lists
    line: u32,
    pill: u32, // fields
    text: u32,
    sub: u32,
    accents: [(&'static str, u32); 6],
}

const LIGHT: Palette = Palette {
    bg: 0xF3F1EC,
    card: 0xFFFFFF,
    line: 0xE4E0D8,
    pill: 0xF4F2EE,
    text: 0x2A2825,
    sub: 0x7C776F,
    accents: [("하늘", 0x8DB5E8), ("그레이", 0xC5C5C5), ("라벤더", 0xA99BE0), ("세이지", 0x8CC5A8), ("피치", 0xEFAE92), ("로즈", 0xE7A1B5)],
};

const DARK: Palette = Palette {
    bg: 0x1C1D20,
    card: 0x26282C,
    line: 0x36393E,
    pill: 0x303338,
    text: 0xE6E8EB,
    sub: 0x9AA0A8,
    accents: [("하늘", 0x4C6FA8), ("그레이", 0x4B4B4B), ("라벤더", 0x6A5AA8), ("세이지", 0x3E8676), ("피치", 0xA8624A), ("로즈", 0xA65A78)],
};

static DARK_ON: AtomicBool = AtomicBool::new(false);
static KO: AtomicBool = AtomicBool::new(false);

/// UI text: English unless Korean is picked in the view menu (the Explorer parts follow Windows).
fn tr(en: &'static str, ko: &'static str) -> &'static str {
    if KO.load(Ordering::Relaxed) {
        ko
    } else {
        en
    }
}

const ACCENT_EN: [&str; 6] = ["Sky", "Gray", "Lavender", "Sage", "Peach", "Rose"];

fn pal() -> &'static Palette {
    if DARK_ON.load(Ordering::Relaxed) {
        &DARK
    } else {
        &LIGHT
    }
}

const INK: u32 = 0x2A2825; // dark text on light selection colours
const GREEN: u32 = 0x1FA463; // unsaved layout changes
fn blend(a: u32, b: u32, t: f32) -> u32 {
    let ch = |sh: u32| {
        let (x, y) = ((a >> sh & 0xFF) as f32, (b >> sh & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    ch(16) | ch(8) | ch(0)
}

// Text/icons on a coloured fill: near-black on light colours, white on dark ones (black accent -> white text).
fn on_color(bg: u32) -> u32 {
    let l = 0.2126 * (bg >> 16 & 0xFF) as f32 + 0.7152 * (bg >> 8 & 0xFF) as f32 + 0.0722 * (bg & 0xFF) as f32;
    if l > 160.0 {
        INK
    } else {
        0xFFFFFF
    }
}
// Segoe Fluent Icons / MDL2 Assets code points
const ICON_SEARCH: &str = "\u{E721}";

fn glyph(id: i32) -> &'static str {
    match id {
        ID_BACK => "\u{E72B}",
        ID_FWD => "\u{E72A}",
        ID_UP => "\u{E74A}",
        ID_OPEN_IN => "\u{E8AB}",
        ID_TB_LAYOUT => "\u{E8A9}",
        ID_TB_SAVE => "\u{E74E}",
        ID_TB_LOAD => "\u{E838}",
        ID_TB_FAV => "\u{E734}",
        ID_TB_THEME if DARK_ON.load(Ordering::Relaxed) => "\u{E706}", // sun: back to light
        ID_TB_THEME => "\u{E708}",                                     // moon: go dark
        _ => "\u{E890}",
    }
}

const KEYS_EN: &str = "Ctrl+Tab / Ctrl+Shift+Tab\tNext / previous pane\n\
Alt+← / Alt+→\tBack / forward\n\
Alt+↑\tParent folder\n\
Ctrl+L, Alt+D, F4\tAddress bar\n\
Ctrl+F\tFilter (*.pdf;*.txt or part of a name; folders always show)\n\
Enter / Esc\tApply / cancel the address or filter\n\
Ctrl+H\tShow hidden files\n\
Ctrl+Shift+M\tNew text file (then rename)\n\
F1\tHelp (opens the README on GitHub)\n\
F2, Del, Ctrl+C/X/V, F5\tSame as Explorer\n\n\
Layout button: click for an even grid, press and drag for rows per column (arrow keys + Enter work too)\n\
⇄ button: open the selected folder (or the current one) in another pane, with this pane's columns\n\
Save (Ctrl+S): overwrite the current session file; green dot = unsaved layout changes (not folder moves)\n\
Load: pick a saved session by its layout preview\n\
Double-click a splitter: split evenly\n\
Arguments: grids.exe C:\\a D:\\b ... or grids.exe work.grids\n\
If Grids is already running, they open in that window.";

const KEYS_KO: &str = "Ctrl+Tab / Ctrl+Shift+Tab\t다음 / 이전 패널\n\
Alt+← / Alt+→\t뒤로 / 앞으로\n\
Alt+↑\t상위 폴더\n\
Ctrl+L, Alt+D, F4\t주소창\n\
Ctrl+F\t필터 (*.pdf;*.txt 또는 이름 일부, 폴더는 항상 표시)\n\
Enter / Esc\t주소창·필터 적용 / 취소\n\
Ctrl+H\t숨김 파일 표시\n\
Ctrl+Shift+M\t새 텍스트 문서 (이어서 이름 바꾸기)\n\
F1\t도움말 (GitHub README 열기)\n\
F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n\
레이아웃 버튼: 클릭 = 균등 배치, 누른 채 끌기 = 열마다 칸 수 (방향키+Enter도 가능)\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기, 열 순서·폭·정렬도 함께\n\
저장 (Ctrl+S): 지금 세션 파일에 바로 덮어쓰기, 초록 점 = 저장 안 된 배치 변경(폴더 이동은 제외)\n\
불러오기: 저장한 세션을 배치 미리보기로 보고 한 번 클릭\n\
스플리터 더블클릭: 균등 분할\n\
실행 인자: grids.exe C:\\a D:\\b ... 또는 grids.exe 작업.grids\n\
이미 실행 중이면 그 창에서 엽니다.";

// ---------- session (grids.ini next to the exe; named sessions are the same format as .grids) ----------

#[derive(Debug, PartialEq, Clone)]
struct Session {
    win: Option<[i32; 5]>, // normal rect l,t,r,b + maximized flag
    side: i32,             // sidebar width in px
    heights: Vec<usize>,   // panes per column, left to right (1..=4 columns of 1..=4)
    colr: Vec<f64>,        // column width ratios
    rowr: Vec<Vec<f64>>,   // row height ratios, per column
    active: usize,
    paths: Vec<String>,     // parsing names per pane slot (numbered down each column, left to right)
    views: Vec<(i32, i32)>, // (FOLDERVIEWMODE, icon size); 0 = shell default
    accent: u32,            // selection colour 0xRRGGBB, light theme
    accent_dark: u32,       // selection colour, dark theme
    dark: bool,
    dblup: bool,            // double-click on empty space goes to the parent folder
    file: String,           // named session (.grids) the layout was last loaded from / saved to
    favs: Vec<String>,      // favourite folders (parsing names), app-wide
    ko: bool,               // Korean UI
    cols: Vec<String>,      // per pane: columns and sort of its view (cols_text)
}

impl Default for Session {
    fn default() -> Self {
        Session {
            win: None,
            side: 0,
            heights: vec![2, 2],
            colr: even(2),
            rowr: vec![even(2); 2],
            active: 0,
            paths: vec![String::new(); MAXP],
            views: vec![(0, 0); MAXP],
            accent: LIGHT.accents[1].1, // gray
            accent_dark: DARK.accents[1].1,
            dark: false,
            dblup: true,
            file: String::new(),
            favs: Vec::new(),
            ko: false,
            cols: vec![String::new(); MAXP],
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

// "2 × 2" for an even grid, "1 | 3" when the columns differ.
fn layout_label(h: &[usize]) -> String {
    match h.iter().all(|&n| n == h[0]) {
        true => format!("{} × {}", h[0], h.len()),
        false => h.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(" | "),
    }
}

impl Session {
    fn accent_now(&self) -> u32 {
        if self.dark {
            self.accent_dark
        } else {
            self.accent
        }
    }

    fn panes(&self) -> usize {
        self.heights.iter().sum()
    }

    /// New layout with even splits.
    fn set_heights(&mut self, h: Vec<usize>) {
        self.colr = even(h.len());
        self.rowr = h.iter().map(|&n| even(n)).collect();
        self.heights = h;
        self.active = self.active.min(self.panes() - 1);
    }

    fn parse(text: &str) -> Session {
        let mut s = Session::default();
        let mut rowr: Vec<Vec<f64>> = Vec::new();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let slot = |p: char| k.strip_prefix(p).and_then(|n| n.parse::<usize>().ok()).filter(|&i| i < MAXP);
            match k {
                "win" => s.win = nums(v).try_into().ok(),
                "side" => s.side = v.parse().unwrap_or(0),
                "cols" => {
                    let h: Vec<usize> = nums::<usize>(v).into_iter().take(4).map(|n| n.clamp(1, 4)).collect();
                    if !h.is_empty() {
                        s.heights = h;
                    }
                }
                // older ini: even rows x cols grid
                "grid" => {
                    if let [r, c] = nums::<usize>(v)[..] {
                        s.heights = vec![r.clamp(1, 4); c.clamp(1, 4)];
                    }
                }
                "colr" => s.colr = nums(v),
                "rowr" => rowr = v.split('|').map(nums).collect(),
                "active" => s.active = v.parse().unwrap_or(0),
                "accent" => s.accent = u32::from_str_radix(v.trim().trim_start_matches('#'), 16).unwrap_or(s.accent) & 0xFFFFFF,
                "accent_dark" => s.accent_dark = u32::from_str_radix(v.trim(), 16).unwrap_or(s.accent_dark) & 0xFFFFFF,
                "dark" => s.dark = v.trim() == "1",
                "dblup" => s.dblup = v.trim() != "0",
                "lang" => s.ko = v.trim() == "ko",
                "session" => s.file = v.trim().to_string(),
                "fav" if !v.trim().is_empty() => s.favs.push(v.trim().to_string()),
                _ => {
                    if let Some(i) = slot('p') {
                        s.paths[i] = v.to_string();
                    } else if let (Some(i), [m, z]) = (slot('v'), &nums::<i32>(v)[..]) {
                        s.views[i] = (*m, *z);
                    } else if let Some(i) = slot('c') {
                        s.cols[i] = v.trim().to_string();
                    }
                }
            }
        }
        s.colr = fix(std::mem::take(&mut s.colr), s.heights.len());
        // one group per column; an older single group applies to every column
        let group = |c: usize| if rowr.len() == s.heights.len() { rowr.get(c) } else { rowr.first() };
        s.rowr = s.heights.iter().enumerate().map(|(c, &n)| fix(group(c).cloned().unwrap_or_default(), n)).collect();
        if s.active >= s.panes() {
            s.active = 0;
        }
        s
    }

    fn to_ini(&self) -> String {
        let mut o = String::new();
        if let Some(w) = self.win {
            o += &format!("win={}\n", join(&w));
        }
        let rowr = self.rowr.iter().map(|r| join(r)).collect::<Vec<_>>().join("|");
        o += &format!("side={}\ncols={}\n", self.side, join(&self.heights));
        o += &format!("colr={}\nrowr={rowr}\nactive={}\n", join(&self.colr), self.active);
        o += &format!("accent={:06X}\naccent_dark={:06X}\ndark={}\n", self.accent, self.accent_dark, self.dark as u8);
        o += &format!("dblup={}\nlang={}\n", self.dblup as u8, if self.ko { "ko" } else { "en" });
        if !self.file.is_empty() {
            o += &format!("session={}\n", self.file);
        }
        for f in &self.favs {
            o += &format!("fav={f}\n");
        }
        for i in 0..MAXP {
            if !self.paths[i].is_empty() {
                o += &format!("p{i}={}\n", self.paths[i]);
            }
            if self.views[i].0 != 0 {
                o += &format!("v{i}={},{}\n", self.views[i].0, self.views[i].1);
            }
            if !self.cols[i].is_empty() {
                o += &format!("c{i}={}\n", self.cols[i]);
            }
        }
        o
    }

    fn is_fav(&self, path: &str) -> bool {
        !path.is_empty() && self.favs.iter().any(|f| f.eq_ignore_ascii_case(path))
    }

    /// What goes into a named session file: no window position, no pointer to itself, no favourites.
    fn snapshot(&self) -> String {
        Session { win: None, file: String::new(), favs: Vec::new(), ..self.clone() }.to_ini()
    }

    /// Takes grid + folders from a named session; window position and sidebar width stay as they are.
    fn take_layout(&mut self, o: &Session) {
        (self.heights, self.colr, self.rowr) = (o.heights.clone(), o.colr.clone(), o.rowr.clone());
        (self.active, self.paths, self.views, self.cols) = (o.active, o.paths.clone(), o.views.clone(), o.cols.clone());
    }
}

/// Layout changes worth a save prompt: grid, split ratios, view modes. Folder changes don't count.
fn layout_differs(cur: &Session, base: &Session) -> bool {
    let near = |x: &[f64], y: &[f64]| x.len() == y.len() && x.iter().zip(y).all(|(a, b)| (a - b).abs() < 0.005);
    cur.heights != base.heights
        || !near(&cur.colr, &base.colr)
        || cur.rowr.iter().zip(&base.rowr).any(|(a, b)| !near(a, b))
        || (0..cur.panes()).any(|i| base.views[i].0 != 0 && cur.views[i] != base.views[i])
        || (0..cur.panes()).any(|i| !base.cols[i].is_empty() && cur.cols[i] != base.cols[i])
}

// Drives, folders and shares; not shell places like This PC or the Recycle Bin, whose columns are their own.
fn is_fs(path: &str) -> bool {
    path.get(1..3) == Some(":\\") || path.starts_with(r"\\")
}

// A view's columns: visible ones in order with their widths, then the sort columns, each property as
// "format GUID/property id": "GUID/pid=width,...;GUID/pid=direction,...".
fn cols_text(cols: &[(PROPERTYKEY, u32)], sort: &[(PROPERTYKEY, i32)]) -> String {
    let key = |k: &PROPERTYKEY| format!("{:?}/{}", k.fmtid, k.pid);
    let cols: Vec<String> = cols.iter().map(|(k, w)| format!("{}={w}", key(k))).collect();
    let sort: Vec<String> = sort.iter().map(|(k, d)| format!("{}={d}", key(k))).collect();
    format!("{};{}", cols.join(","), sort.join(","))
}

fn parse_cols(t: &str) -> (Vec<(PROPERTYKEY, u32)>, Vec<(PROPERTYKEY, i32)>) {
    fn items<T: std::str::FromStr>(s: &str) -> Vec<(PROPERTYKEY, T)> {
        let item = |e: &str| {
            let (k, v) = e.split_once('=')?;
            let (g, pid) = k.split_once('/')?;
            Some((PROPERTYKEY { fmtid: GUID::try_from(g).ok()?, pid: pid.parse().ok()? }, v.parse().ok()?))
        };
        s.split(',').filter_map(item).collect()
    }
    let (cols, sort) = t.split_once(';').unwrap_or((t, ""));
    (items(cols), items(sort))
}

// Existing relative paths become absolute; shell names like ::{GUID} pass through.
fn resolve(p: String) -> String {
    match Path::new(&p).exists() {
        true => std::path::absolute(&p).map(|x| x.to_string_lossy().into_owned()).unwrap_or(p),
        false => p,
    }
}

fn is_session_file(p: &Path) -> bool {
    // .qdr: sessions saved before the rename
    p.extension().is_some_and(|x| x.eq_ignore_ascii_case("grids") || x.eq_ignore_ascii_case("qdr"))
}

// A lone session-file argument means "load this session".
fn session_arg(args: &[String]) -> Option<&str> {
    match args {
        [f] if is_session_file(Path::new(f)) => Some(f),
        _ => None,
    }
}

// `grids.exe A B C` opens the paths in pane order, growing the grid if needed.
fn apply_args(s: &mut Session, args: Vec<String>) {
    if args.is_empty() {
        return;
    }
    let n = args.len().min(MAXP);
    for (i, p) in args.into_iter().take(MAXP).enumerate() {
        s.paths[i] = p;
    }
    if n > s.panes() {
        let c = (n as f64).sqrt().ceil() as usize;
        s.set_heights(vec![n.div_ceil(c); c]);
    }
    s.active = 0;
}

fn ini_path() -> PathBuf {
    std::env::current_exe().unwrap_or_default().with_file_name("grids.ini")
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
    Col(usize),        // boundary after column k
    Row(usize, usize), // (column, boundary after row k in it)
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
    ys: Vec<Vec<(i32, i32)>>, // per column
}

fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let (pad, gap, top) = (s(8), s(8), s(34));
    let bot = (rc.bottom - s(24)).max(top);
    let side = a.s.side.clamp(s(120), (rc.right - 2 * pad - gap - s(240)).max(s(120)));
    let gx = pad + side + gap;
    Geo {
        xs: cells(gx, rc.right - pad - gx, &a.s.colr, gap),
        ys: a.s.rowr.iter().map(|r| cells(top, bot - top, r, gap)).collect(),
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

/// Pane slot -> (column, row): panes are numbered down each column, left to right.
fn slot(h: &[usize], mut i: usize) -> (usize, usize) {
    for (c, &n) in h.iter().enumerate() {
        if i < n {
            return (c, i);
        }
        i -= n;
    }
    (h.len() - 1, h[h.len() - 1] - 1)
}

fn cell_rect(g: &Geo, h: &[usize], i: usize) -> RECT {
    let (c, r) = slot(h, i);
    let ((x0, x1), (y0, y1)) = (g.xs[c], g.ys[c][r]);
    RECT { left: x0, top: y0, right: x1, bottom: y1 }
}

// Inside a pane plate: icon buttons, address pill, ⇄, filter pill on top; the Explorer view below.
struct PaneGeo {
    btn: [RECT; 4], // back, forward, up, open-in
    addr: RECT,
    filt: RECT,
    fav: RECT,
    view: RECT,
}

fn pane_geo(w: i32, h: i32, dpi: i32) -> PaneGeo {
    let s = |v: i32| v * dpi / 96;
    let (bar, bh, b, pad) = (s(30), s(22), s(24), s(4));
    let y = (bar - bh) / 2;
    let r = |x: i32, wd: i32| RECT { left: x, top: y, right: x + wd.max(0), bottom: y + bh };
    let fav = r(w - pad - b, b);
    let fw = s(150).min((w - 2 * pad - 5 * b) / 3).max(0);
    let filt = r(fav.left - s(2) - fw, fw);
    let open = r(filt.left - s(2) - b, b);
    let addr_x = pad + 3 * b + s(4);
    PaneGeo {
        btn: [r(pad, b), r(pad + b, b), r(pad + 2 * b, b), open],
        addr: r(addr_x, open.left - s(4) - addr_x),
        filt,
        fav,
        // 2px in, so the active ring stays visible around the list
        view: RECT { left: s(2), top: bar, right: w - s(2), bottom: h - s(2) },
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
    if let Some(k) = between(&g.xs, x) {
        return Some(Hit::Col(k));
    }
    // row gaps belong to one column: rows no longer line up across columns
    let c = g.xs.iter().position(|&(a, b)| x >= a && x < b)?;
    between(&g.ys[c], y).map(|k| Hit::Row(c, k))
}

// ---------- app state ----------
// Single UI thread. Rule: never call into Win32/COM that can re-enter our window procs while inside app().

struct Pane {
    hwnd: HWND,
    eb: IExplorerBrowser,
    cookie: u32,
    disp: String,
    pending: Option<String>, // columns to put on when the next folder shows (session restore, open in)
    settle: Option<(String, u8)>, // columns just put on, and passes left to hold narrow widths (TIMER_SETTLE)
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
    dpi: i32,
    panes: Vec<Option<Pane>>,
    drag: Option<Hit>,
    sizing: bool, // inside the window's own move/size loop
    status: String,
    status_key: String,
    view_menu: HMENU,
    base: Option<Session>, // last loaded/saved (or startup) state; differs -> green dot
    dirty: bool,
    small_font: HFONT,
    load: HWND, // load popup while open
    load_closed: Option<Instant>,
    load_hot: Option<usize>, // card index, or LOAD_SAVE_AS / LOAD_FOLDER
    fav_popup: HWND,
    fav_closed: Option<Instant>,
    fav_hot: Option<(usize, bool)>, // (row, on its ✕)
    cards: Vec<(PathBuf, Session, String)>,
    hover: HWND, // owner-drawn button under the mouse
    tip: HWND,
    bars: [HWND; 2], // splitter guide bars (vertical, horizontal)
    grid: HWND,      // layout picker popup while open
    reveal_tries: u8, // sidebar reveal passes left
    grid_preview: Vec<usize>,       // layout shown in the picker
    grid_drag: Option<Vec<usize>>, // press-and-drag in progress
    grid_preset: Option<usize>,    // preset tile under the mouse
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
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        sizing: false,
        status: String::new(),
        status_key: String::new(),
        view_menu: HMENU::default(),
        base: None,
        dirty: false,
        small_font: HFONT::default(),
        load: HWND::default(),
        load_closed: None,
        load_hot: None,
        fav_popup: HWND::default(),
        fav_closed: None,
        fav_hot: None,
        cards: Vec::new(),
        hover: HWND::default(),
        tip: HWND::default(),
        bars: [HWND::default(); 2],
        grid: HWND::default(),
        reveal_tries: 0,
        grid_preview: Vec::new(),
        grid_drag: None,
        grid_preset: None,
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

/// Pane that commands act on. A tab strip would return the active tab's pane from here.
fn active_pane() -> usize {
    app(|a| a.s.active)
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

// Windows' own UI font at this DPI (no custom fonts: they only reached our chrome and cost paint time),
// plus the MDL2 icon font for the buttons. Pushed to every edit; also used on DPI change.
unsafe fn apply_font() {
    let dpi = app(|a| a.dpi);
    let mut ncm = NONCLIENTMETRICSW { cbSize: size_of::<NONCLIENTMETRICSW>() as u32, ..Default::default() };
    let sys = SystemParametersInfoForDpi(SPI_GETNONCLIENTMETRICS.0, ncm.cbSize, Some(&mut ncm as *mut _ as _), 0, dpi as u32);
    let mut lf = if sys.is_ok() { ncm.lfMessageFont } else { logfont("Segoe UI", 12 * dpi / 96) };
    lf.lfFaceName = logfont("Segoe UI", 0).lfFaceName; // system size, but the Korean UI font draws '\\' in paths as '₩'
    let font = CreateFontIndirectW(&lf);
    let small = CreateFontIndirectW(&LOGFONTW { lfHeight: lf.lfHeight * 85 / 100, ..lf });
    let icon_font = CreateFontIndirectW(&logfont("Segoe MDL2 Assets", 14 * dpi / 96));
    let dc = GetDC(None);
    let old = SelectObject(dc, font.into());
    let mut tm = TEXTMETRICW::default();
    _ = GetTextMetricsW(dc, &mut tm);
    SelectObject(dc, old);
    ReleaseDC(None, dc);
    let (old, old_icon, panes) = app(|a| {
        a.fh = tm.tmHeight;
        let old_small = std::mem::replace(&mut a.small_font, small);
        if !old_small.is_invalid() {
            unsafe { _ = DeleteObject(old_small.into()) };
        }
        let old = std::mem::replace(&mut a.font, font);
        let old_icon = std::mem::replace(&mut a.icon_font, icon_font);
        (old, old_icon, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>())
    });
    for p in panes {
        for id in [ID_EDIT, ID_FILTER] {
            if let Ok(c) = GetDlgItem(Some(p), id) {
                SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
            }
        }
    }
    for f in [old, old_icon].into_iter().filter(|f| !f.is_invalid()) {
        _ = DeleteObject(f.into());
    }
    relayout();
    redraw_all();
}

// ---------- drawing (plain GDI: square shapes need no anti-aliasing, and fills are cheap) ----------

fn cref(rgb: u32) -> COLORREF {
    COLORREF((rgb & 0xFF) << 16 | (rgb & 0xFF00) | (rgb >> 16 & 0xFF))
}

unsafe fn fill(dc: HDC, r: RECT, rgb: u32) {
    SetDCBrushColor(dc, cref(rgb));
    FillRect(dc, &r, HBRUSH(GetStockObject(DC_BRUSH).0));
}

unsafe fn frame(dc: HDC, r: RECT, w: i32, rgb: u32) {
    let sides = [RECT { bottom: r.top + w, ..r }, RECT { top: r.bottom - w, ..r }, RECT { right: r.left + w, ..r }, RECT { left: r.right - w, ..r }];
    for side in sides {
        fill(dc, side, rgb);
    }
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

// Paints only the invalid area into an off-screen buffer and blits once (no flicker mid-drag).
unsafe fn buffered(h: HWND, draw: impl FnOnce(HDC, RECT)) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let mut mem = HDC::default();
    let pb = BeginBufferedPaint(hdc, &ps.rcPaint, BPBF_TOPDOWNDIB, None, &mut mem);
    if pb == 0 {
        draw(hdc, rc);
    } else {
        draw(mem, rc);
        _ = EndBufferedPaint(pb, true);
    }
    _ = EndPaint(h, &ps);
}

// Pane header colours (background, text/icons): tinted with the selection colour while active.
// The address and filter fields keep their own colours either way.
fn header_colors(pane: HWND) -> (u32, u32) {
    app(|a| {
        let active = a.panes.get(a.s.active).and_then(|p| p.as_ref()).is_some_and(|p| p.hwnd == pane);
        if active {
            (a.s.accent_now(), on_color(a.s.accent_now()))
        } else {
            (pal().card, pal().text)
        }
    })
}

unsafe fn draw_button(d: &DRAWITEMSTRUCT) {
    let (hover, dpi, font, icon_font, main) = app(|a| (a.hover, a.dpi, a.font, a.icon_font, a.main));
    let parent = GetParent(d.hwndItem).unwrap_or_default();
    let on_main = parent == main;
    let (dc, rc, id) = (d.hDC, d.rcItem, d.CtlID as i32);
    let pressed = d.itemState.0 & ODS_SELECTED.0 != 0;
    let hot = hover == d.hwndItem;
    let (bg, ink) = if on_main { (pal().bg, pal().text) } else { header_colors(parent) };
    let tinted = bg != pal().bg && bg != pal().card;
    fill(dc, rc, bg);
    if pressed || hot {
        let t = match (tinted, pressed) {
            (true, true) => 0.3,
            (true, false) => 0.18,
            (false, true) => 0.13,
            (false, false) => 0.07,
        };
        fill(dc, rc, blend(bg, ink, t));
    }
    let fg = if tinted || hot || pressed { ink } else { pal().sub };
    if id == ID_FAV {
        // ★ filled (gray, or the header ink when tinted) in a favourite folder, otherwise ☆ with a small +
        let fav = app(|a| a.panes.iter().position(|p| p.as_ref().is_some_and(|p| p.hwnd == parent)).is_some_and(|i| a.s.is_fav(&a.s.paths[i])));
        if fav {
            return draw_star(dc, rc, if tinted { ink } else { pal().sub }, Star::Filled, dpi);
        }
        // ☆ exactly where ★ goes, with a + at its exact centre
        return draw_star(dc, rc, fg, Star::Plus, dpi);
    }
    if !on_main {
        return text(dc, rc, glyph(id), icon_font, fg, DT_CENTER);
    }
    let s = |v: i32| v * dpi / 96;
    if id == ID_TB_LAYOUT {
        // the one label in the chrome: the current grid
        let icon = RECT { left: rc.left + s(4), right: rc.left + s(22), ..rc };
        text(dc, icon, glyph(id), icon_font, fg, DT_CENTER);
        return text(dc, RECT { left: icon.right + s(4), ..rc }, &text_of(d.hwndItem), font, pal().text, DT_LEFT);
    }
    // icon only, names live in the tooltips; save turns green with a dot while the layout differs
    if id == ID_TB_FAV {
        return draw_star(dc, rc, fg, Star::Outline, dpi);
    }
    let dirty = id == ID_TB_SAVE && app(|a| a.dirty);
    text(dc, rc, glyph(id), icon_font, if dirty { GREEN } else { fg }, DT_CENTER);
    if dirty {
        fill(dc, RECT { left: rc.right - s(10), top: rc.top + s(4), right: rc.right - s(4), bottom: rc.top + s(10) }, GREEN);
    }
}

// A chubby five-point star (inner radius 0.56 of the outer one) with softly rounded tips, as 0..=255
// coverage per pixel of an n×n box. Font stars are too sharp, so it's computed: 4×4 samples per pixel.
#[derive(Clone, Copy, PartialEq)]
enum Star {
    Outline,
    Filled,
    Plus, // outline with a + in its exact centre (add to favourites)
}

fn star_coverage(n: i32, kind: Star) -> Vec<u8> {
    let filled = kind == Star::Filled;
    let (c, r_out) = (n as f32 / 2.0, n as f32 / 2.0 - 0.5);
    let pt = |k: i32, r: f32| {
        let a = (-90.0 + 36.0 * k as f32).to_radians();
        (c + r * a.cos(), c + r * a.sin())
    };
    let poly: Vec<(f32, f32)> = (0..10).map(|k| pt(k, if k % 2 == 0 { r_out } else { r_out * 0.56 })).collect();
    let (stroke, round) = ((n as f32 * 0.11).max(1.2), r_out * 0.17);
    // each tip is replaced by a circle tangent to both of its edges
    let tips: Vec<((f32, f32), f32)> = (0..5)
        .map(|i| {
            let (t, v1, v2) = (poly[2 * i], poly[(2 * i + 9) % 10], poly[2 * i + 1]);
            let (a, b) = ((v1.0 - t.0, v1.1 - t.1), (v2.0 - t.0, v2.1 - t.1));
            let half = ((a.0 * b.0 + a.1 * b.1) / (a.0.hypot(a.1) * b.0.hypot(b.1))).acos() / 2.0; // half the tip angle
            let d = round / half.sin();
            let (ux, uy) = ((t.0 - c) / r_out, (t.1 - c) / r_out);
            ((t.0 - ux * d, t.1 - uy * d), d)
        })
        .collect();
    let inside_dist = |x: f32, y: f32| -> Option<f32> {
        for (i, &((cx, cy), d)) in tips.iter().enumerate() {
            let (ux, uy) = ((poly[2 * i].0 - c) / r_out, (poly[2 * i].1 - c) / r_out);
            if (x - cx) * ux + (y - cy) * uy > 0.0 && (x - poly[2 * i].0).hypot(y - poly[2 * i].1) < 2.0 * d {
                let e = round - (x - cx).hypot(y - cy);
                return (e >= 0.0).then_some(e);
            }
        }
        let mut inside = false;
        let mut edge = f32::MAX;
        for k in 0..10 {
            let ((x1, y1), (x2, y2)) = (poly[k], poly[(k + 1) % 10]);
            if (y1 > y) != (y2 > y) && x < x1 + (y - y1) * (x2 - x1) / (y2 - y1) {
                inside = !inside;
            }
            let (dx, dy) = (x2 - x1, y2 - y1);
            let t = (((x - x1) * dx + (y - y1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            edge = edge.min((x - x1 - t * dx).hypot(y - y1 - t * dy));
        }
        inside.then_some(edge)
    };
    let mut out = vec![0u8; (n * n) as usize];
    for py in 0..n {
        for px in 0..n {
            let mut hit = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, y) = (px as f32 + (sx as f32 + 0.5) / 4.0, py as f32 + (sy as f32 + 0.5) / 4.0);
                    hit += inside_dist(x, y).is_some_and(|e| filled || e < stroke) as u32;
                }
            }
            out[(py * n + px) as usize] = (hit * 255 / 16) as u8;
        }
    }
    if kind == Star::Plus {
        // whole pixels, with the same parity as n so the + is centred exactly on the star's centre,
        // and arms short enough to float in the hollow without touching the outline
        let mut t = (n / 12).max(1);
        if (n - t) % 2 != 0 {
            t += 1;
        }
        let hollow = r_out * 0.516 - stroke; // centre to the inside of the outline, at its nearest
        let len = t + 2 * ((hollow - 0.4 - t as f32 / 2.0) as i32).max(1);
        let (a, b) = ((n - t) / 2, (n - len) / 2);
        for i in b..b + len {
            for j in a..a + t {
                out[(j * n + i) as usize] = 255;
                out[(i * n + j) as usize] = 255;
            }
        }
    }
    out
}

unsafe fn draw_star(dc: HDC, rc: RECT, rgb: u32, kind: Star, dpi: i32) {
    let n = 15 * dpi / 96;
    let cov = star_coverage(n, kind);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: n,
            biHeight: -n,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = std::ptr::null_mut();
    let Ok(bmp) = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else { return };
    let px = std::slice::from_raw_parts_mut(bits as *mut u8, (n * n * 4) as usize);
    for (i, &a) in cov.iter().enumerate() {
        let pre = |ch: u32| ((ch & 0xFF) * a as u32 / 255) as u8; // premultiplied BGRA
        px[i * 4..i * 4 + 4].copy_from_slice(&[pre(rgb), pre(rgb >> 8), pre(rgb >> 16), a]);
    }
    let mem = CreateCompatibleDC(Some(dc));
    let old = SelectObject(mem, bmp.into());
    let (x, y) = ((rc.left + rc.right - n) / 2, (rc.top + rc.bottom - n) / 2);
    let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
    _ = AlphaBlend(dc, x, y, n, n, mem, 0, 0, n, n, blend);
    SelectObject(mem, old);
    _ = DeleteDC(mem);
    _ = DeleteObject(bmp.into());
}

// Repaints the save icon and puts the session name / unsaved state into its tooltip.
unsafe fn refresh_save_button() {
    let (main, tip, file, dirty) = app(|a| (a.main, a.tip, a.s.file.clone(), a.dirty));
    let Ok(b) = GetDlgItem(Some(main), ID_TB_SAVE) else { return };
    _ = InvalidateRect(Some(b), None, false);
    let mut hint = match Path::new(&file).file_stem() {
        Some(n) if KO.load(Ordering::Relaxed) => format!("'{}'에 저장 (Ctrl+S)", n.to_string_lossy()),
        Some(n) => format!("Save to '{}' (Ctrl+S)", n.to_string_lossy()),
        None => tr("Save session (Ctrl+S)", "세션 저장 (Ctrl+S)").into(),
    };
    if dirty {
        hint += tr(" · unsaved layout changes", " · 저장 안 된 배치 변경");
    }
    let hint = HSTRING::from(hint);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND,
        hwnd: main,
        uId: b.0 as usize,
        lpszText: PWSTR(hint.as_ptr() as *mut u16),
        ..Default::default()
    };
    SendMessageW(tip, TTM_UPDATETIPTEXTW, None, Some(LPARAM(&ti as *const _ as isize)));
}

// Polled with the status timer: view modes are only observable by asking the views.
unsafe fn check_dirty() {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
    let changed = app(|a| {
        if let Some(b) = a.base.as_mut() {
            for i in 0..a.s.panes() {
                if b.cols[i].is_empty() {
                    b.cols[i] = a.s.cols[i].clone();
                }
            }
        }
        let d = a.base.as_ref().is_some_and(|b| layout_differs(&a.s, b));
        std::mem::replace(&mut a.dirty, d) != d
    });
    if changed {
        refresh_save_button();
    }
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

unsafe fn add_tip(btn: HWND, tip: &str) {
    tip_text(btn, tip, TTM_ADDTOOLW);
}

// TTM_ADDTOOLW to add a control's tooltip, TTM_UPDATETIPTEXTW to change its text.
unsafe fn tip_text(btn: HWND, tip: &str, msg: u32) {
    let tip = HSTRING::from(tip);
    let ti = TTTOOLINFOW {
        cbSize: size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
        hwnd: GetParent(btn).unwrap_or_default(),
        uId: btn.0 as usize,
        lpszText: PWSTR(tip.as_ptr() as *mut u16),
        ..Default::default()
    };
    SendMessageW(app(|a| a.tip), msg, None, Some(LPARAM(&ti as *const _ as isize)));
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
        // the target takes this pane's columns too (order, widths, sort): set one up, copy it around
        save_view(src);
        app(|a| {
            let cols = a.s.cols[src].clone();
            if let Some(p) = a.panes[dst].as_mut() {
                p.pending = Some(cols);
            }
        });
        browse_item(dst, &item);
        set_active(dst);
        focus_view(dst);
    }
}

unsafe fn open_in_menu(h: HWND, src: usize) {
    let items = app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.panes());
        live.filter(|(k, _)| *k != src).filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone()))).collect::<Vec<_>>()
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Open the selected folder (or this one) in:", "선택한 폴더(없으면 현재 폴더)를 열 패널:")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (k, disp) in &items {
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("{} {}    {disp}", tr("Pane", "패널"), k + 1)));
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

// Pane-sticky view mode and columns (order, widths, sort): remember them before leaving a folder,
// re-apply after arriving. Columns seen in shell places (This PC ...) are theirs, not the pane's.
unsafe fn save_view(i: usize) {
    if app(|a| a.panes[i].as_ref().is_none_or(|p| p.pending.is_some() || p.settle.is_some())) {
        return; // columns are about to be put on, or still settling: don't overwrite them
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    free_min_widths(view_list(&fv)); // a list made after the folder change gets it here
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    if fv.GetViewModeAndIconSize(&mut m, &mut z).is_ok() {
        app(|a| a.s.views[i] = (m.0, z));
    }
    if let Some(c) = read_cols(&fv, view_list(&fv)).filter(|_| app(|a| is_fs(&a.s.paths[i]))) {
        app(|a| a.s.cols[i] = c);
    }
}

// Column names in on-screen order, from the pane's list header: dragging a column to a new place
// changes only the header's order, which the view's own column list doesn't follow.
unsafe fn header_names(lv: HWND) -> Vec<String> {
    let n = list_titles(lv).len();
    let mut order = vec![0i32; n];
    SendMessageW(lv, LVM_GETCOLUMNORDERARRAY, Some(WPARAM(n)), Some(LPARAM(order.as_mut_ptr() as isize)));
    let titles = list_titles(lv);
    order.iter().filter_map(|&k| titles.get(k as usize).cloned()).collect()
}

// Column titles of a list, by column index.
unsafe fn list_titles(lv: HWND) -> Vec<String> {
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    let n = SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize;
    (0..n)
        .map(|k| {
            let mut text = [0u16; 128];
            let mut item = HDITEMW { mask: HDI_TEXT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
            SendMessageW(header, HDM_GETITEMW, Some(WPARAM(k)), Some(LPARAM(&mut item as *mut _ as isize)));
            String::from_utf16_lossy(&text[..text.iter().position(|&c| c == 0).unwrap_or(0)])
        })
        .collect()
}

// The list inside a view. During a folder change the old view's list is still around, so it is
// found from the view itself, not by looking through the pane.
unsafe fn view_list(fv: &IFolderView2) -> HWND {
    let view = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
    FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default()
}

// Explorer puts a floor under some columns (Size stops at 90 px) so numbers never get cut, and puts it
// back whenever a column changes (through an interface, not messages we could catch). In a pane the
// user decides: the floor is lowered to a sliver right before each resize, a drag on the header or a
// width set on the list.
unsafe fn free_min_widths(lv: HWND) {
    if lv.0.is_null() {
        return;
    }
    let mut was = 0usize;
    let first = !GetWindowSubclass(lv, Some(list_floor_proc), 3, Some(&mut was as *mut _)).as_bool();
    _ = SetWindowSubclass(lv, Some(list_floor_proc), 3, 0);
    if first {
        // the view puts its floor back on the first width change a list sees, and only then: spend that
        // change here (a pixel wider and back) so the user's first drag already goes below it
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
            let w = SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0;
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w + 1)));
            SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w)));
        }
    }
    // and no snapping to the floor or ideal width while dragging, no automatic sizing: the user's width
    let snap = LVS_EX_COLUMNSNAPPOINTS | LVS_EX_AUTOSIZECOLUMNS;
    if SendMessageW(lv, LVM_GETEXTENDEDLISTVIEWSTYLE, None, None).0 as usize & snap != 0 {
        SendMessageW(lv, LVM_SETEXTENDEDLISTVIEWSTYLE, Some(WPARAM(snap)), Some(LPARAM(0)));
    }
}

unsafe fn lower_floors(lv: HWND) {
    let floor = 20 * GetDpiForWindow(lv) as i32 / 96;
    let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
    for c in 0..SendMessageW(header, HDM_GETITEMCOUNT, None, None).0.max(0) as usize {
        let mut col = LVCOLUMNW { mask: LVCF_MINWIDTH, ..Default::default() };
        if SendMessageW(lv, LVM_GETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&mut col as *mut _ as isize))).0 != 0 && col.cxMin > floor {
            let lower = LVCOLUMNW { mask: LVCF_MINWIDTH, cxMin: floor, ..Default::default() };
            SendMessageW(lv, LVM_SETCOLUMNW, Some(WPARAM(c)), Some(LPARAM(&lower as *const _ as isize)));
        }
    }
}

unsafe extern "system" fn list_floor_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let resize = match msg {
        LVM_SETCOLUMNWIDTH => true,
        WM_NOTIFY => matches!((*(lp.0 as *const NMHDR)).code, HDN_BEGINTRACKW | HDN_TRACKW | HDN_ITEMCHANGINGW),
        _ => false,
    };
    if resize {
        lower_floors(h);
    }
    // the view stops a column coming down from above its floor at the floor (through its own channel,
    // not a message): the narrowing asked for goes on again right after, from the floor, where it isn't stopped
    if msg == WM_NOTIFY && (*(lp.0 as *const NMHDR)).code == HDN_ITEMCHANGINGW {
        let nm = &*(lp.0 as *const NMHEADERW);
        if !nm.pitem.is_null() && (*nm.pitem).mask.0 & HDI_WIDTH.0 != 0 {
            let floor = 20 * GetDpiForWindow(h) as i32 / 96; // never all the way to nothing
            let (col, want) = (nm.iItem, (*nm.pitem).cxy.max(floor));
            let now = SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col as usize)), None).0 as i32;
            let ask = if want < now { (col as i64) << 32 | want as i64 } else { -1 };
            NARROWING.store(ask, Ordering::Relaxed); // a newer change (or a widening) cancels an older one
            if ask >= 0 {
                _ = PostMessageW(Some(h), WM_NARROW_AGAIN, WPARAM(col as usize), LPARAM(want as isize));
            }
        }
    }
    if msg == WM_NARROW_AGAIN {
        let (col, want) = (wp.0, lp.0 as i32);
        if NARROWING.load(Ordering::Relaxed) == ((col as i64) << 32 | want as i64)
            && SendMessageW(h, LVM_GETCOLUMNWIDTH, Some(WPARAM(col)), None).0 as i32 != want // stopped at the floor, or let through to nothing
        {
            NARROWING.store(-1, Ordering::Relaxed);
            SendMessageW(h, LVM_SETCOLUMNWIDTH, Some(WPARAM(col)), Some(LPARAM(want as isize)));
        }
        return LRESULT(0);
    }
    DefSubclassProc(h, msg, wp, lp)
}

const WM_NARROW_AGAIN: u32 = WM_APP + 41;
static NARROWING: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // column << 32 | width


// The view's visible columns with widths, put in on-screen order by name, then its sort columns.
// Widths come from the list: one narrowed below the view's floor is only there (the view still says 90).
unsafe fn read_cols(fv: &IFolderView2, lv: HWND) -> Option<String> {
    let cm: IColumnManager = fv.cast().ok()?;
    let (titles, shown) = (list_titles(lv), header_names(lv));
    let mut keys = vec![PROPERTYKEY::default(); cm.GetColumnCount(CM_ENUM_VISIBLE).ok()? as usize];
    cm.GetColumns(CM_ENUM_VISIBLE, &mut keys).ok()?;
    let info = |k: &PROPERTYKEY| {
        let mask = (CM_MASK_WIDTH.0 | CM_MASK_NAME.0) as u32;
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: mask, ..Default::default() };
        _ = cm.GetColumnInfo(k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        let on_list = titles.iter().position(|t| *t == name).map(|c| SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 as u32);
        (*k, on_list.filter(|&w| w > 0).unwrap_or(ci.uWidth), name)
    };
    let mut left: Vec<_> = keys.iter().map(info).collect();
    let mut cols = Vec::new();
    for name in &shown {
        if let Some(at) = left.iter().position(|c| c.2 == *name) {
            cols.push(left.remove(at));
        }
    }
    cols.extend(left); // any the header didn't name keep the view's order
    let cols: Vec<_> = cols.into_iter().map(|(k, w, _)| (k, w)).collect();
    let mut sort = vec![SORTCOLUMN::default(); fv.GetSortColumnCount().ok()?.max(0) as usize];
    fv.GetSortColumns(&mut sort).ok()?;
    Some(cols_text(&cols, &sort.iter().map(|c| (c.propkey, c.direction.0)).collect::<Vec<_>>()))
}

// The view keeps its own floor (Size: 90 px) under widths set through it, also when it builds a new
// folder's columns: narrower ones go straight onto the list, which lets them through (list_floor_proc).
unsafe fn put_widths(fv: &IFolderView2, text: &str, lv: HWND) {
    let Ok(cm) = fv.cast::<IColumnManager>() else { return };
    let titles = list_titles(lv);
    for (k, w) in parse_cols(text).0 {
        let mut ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_NAME.0 as u32, ..Default::default() };
        _ = cm.GetColumnInfo(&k, &mut ci);
        let name = String::from_utf16_lossy(&ci.wszName[..ci.wszName.iter().position(|&c| c == 0).unwrap_or(0)]);
        if let Some(c) = titles.iter().position(|t| *t == name) {
            if SendMessageW(lv, LVM_GETCOLUMNWIDTH, Some(WPARAM(c)), None).0 != w as isize {
                SendMessageW(lv, LVM_SETCOLUMNWIDTH, Some(WPARAM(c)), Some(LPARAM(w as isize)));
            }
        }
    }
}

unsafe fn apply_cols(fv: &IFolderView2, text: &str, lv: HWND) {
    let (cols, sort) = parse_cols(text);
    if let (false, Ok(cm)) = (cols.is_empty(), fv.cast::<IColumnManager>()) {
        if cm.SetColumns(&cols.iter().map(|c| c.0).collect::<Vec<_>>()).is_ok() {
            for (k, w) in &cols {
                let ci = CM_COLUMNINFO { cbSize: size_of::<CM_COLUMNINFO>() as u32, dwMask: CM_MASK_WIDTH.0 as u32, uWidth: *w, ..Default::default() };
                _ = cm.SetColumnInfo(k, &ci);
            }
            put_widths(fv, text, lv);
        }
    }
    if !sort.is_empty() {
        _ = fv.SetSortColumns(&sort.iter().map(|&(propkey, d)| SORTCOLUMN { propkey, direction: SORTDIRECTION(d) }).collect::<Vec<_>>());
    }
}

// A few passes after a folder change (50, 150, 400, 1000 ms) put narrow widths back as the view builds
// its columns at its own floor; then the pane's columns are read from the view again.
unsafe fn settle_cols(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SETTLE);
    let Some(i) = pane_of(h) else { return };
    let Some((cols, left)) = app(|a| a.panes[i].as_ref().and_then(|p| p.settle.clone())) else { return };
    if let Some(fv) = view_of::<IFolderView2>(i) {
        let lv = view_list(&fv); // the new folder's list exists by now
        free_min_widths(lv);
        put_widths(&fv, &cols, lv);
    }
    let left = left - 1;
    app(|a| {
        if let Some(p) = a.panes[i].as_mut() {
            p.settle = (left > 0).then(|| (cols, left));
        }
    });
    if left > 0 {
        SetTimer(Some(h), TIMER_SETTLE, [1000, 400, 150][left as usize - 1], None);
    }
}

unsafe fn on_nav(i: usize, pidl: *const ITEMIDLIST) {
    let parse = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let disp = name(pidl, SIGDN_DESKTOPABSOLUTEEDITING);
    let Some((h, eb, view, cols)) = app(|a| {
        let fs = is_fs(&parse);
        a.s.paths[i] = parse;
        let (v, own) = (a.s.views[i], a.s.cols[i].clone());
        a.panes[i].as_mut().map(|p| {
            p.disp = disp.clone();
            // waiting columns (session, open in) first, else the pane's own; none in shell places
            let cols = p.pending.take().unwrap_or(own);
            (p.hwnd, p.eb.clone(), v, Some(cols).filter(|c| fs && !c.is_empty()))
        })
    }) else {
        return;
    };
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp));
    }
    if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
        _ = InvalidateRect(Some(b), None, false);
    }
    if let Ok(fv) = eb.GetCurrentView::<IFolderView2>() {
        if view.0 != 0 {
            _ = fv.SetViewModeAndIconSize(FOLDERVIEWMODE(view.0), view.1);
        }
        let lv = view_list(&fv);
        free_min_widths(lv);
        if let Some(cols) = cols {
            apply_cols(&fv, &cols, lv);
            app(|a| {
                if let Some(p) = a.panes[i].as_mut() {
                    p.settle = Some((cols, 4));
                }
            });
            SetTimer(Some(h), TIMER_SETTLE, 50, None);
        }
        // No full-row select: a drag that starts in the size/date cells draws a selection box
        // (and selects the names it covers) instead of grabbing that row's file.
        _ = fv.SetCurrentFolderFlags(FWF_FULLROWSELECT.0 as u32, 0);
    }
    if DARK_ON.load(Ordering::Relaxed) {
        dark_list(h);
    }
    if app(|a| a.s.active == i) {
        if let Ok(item) = SHCreateItemFromIDList::<IShellItem>(pidl) {
            sync_tree(&item);
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
        | NSTCS_HORIZONTALSCROLL.0; // a real scrollbar, so a deep folder can be scrolled into view (AUTOHSCROLL has none)
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
    let h = CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsPane"), w!(""), style, 0, 0, 0, 0, Some(main), None, None, None)?;
    let (btn, plain) = (BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0));
    let buttons = [
        (ID_BACK, tr("Back", "뒤로"), tr("Back  (Alt+←)", "뒤로  (Alt+←)")),
        (ID_FWD, tr("Forward", "앞으로"), tr("Forward  (Alt+→)", "앞으로  (Alt+→)")),
        (ID_UP, tr("Parent folder", "상위 폴더"), tr("Parent folder  (Alt+↑)", "상위 폴더  (Alt+↑)")),
        (ID_OPEN_IN, tr("Open in another pane", "다른 패널에서 열기"), tr("Open in another pane", "다른 패널에서 열기")),
        (ID_FAV, tr("Favourite", "즐겨찾기"), tr("Add to favourites (filled star: remove)", "즐겨찾기에 추가 (채워진 별: 빼기)")),
    ];
    for (id, name, tip) in buttons {
        add_tip(child(h, w!("BUTTON"), PCWSTR(HSTRING::from(name).as_ptr()), btn, plain, id, font), tip);
    }
    child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_EDIT, font);
    child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, plain, ID_FILTER, font);

    let eb: IExplorerBrowser = CoCreateInstance(&ExplorerBrowser, None, CLSCTX_INPROC_SERVER)?;
    let site: IServiceProvider = Host(i).into();
    IUnknown_SetSite(&eb, &site)?; // before Initialize, so the view picks up our ICommDlgBrowser
    let fs = FOLDERSETTINGS { ViewMode: FVM_DETAILS.0 as u32, fFlags: FWF_NOWEBVIEW.0 as u32 };
    eb.Initialize(h, &RECT::default(), Some(&fs))?;
    eb.SetOptions(EBO_NOBORDER)?;
    let ev: IExplorerBrowserEvents = Events(i).into();
    let cookie = eb.Advise(&ev)?;
    let pending = Some(app(|a| a.s.cols[i].clone())); // the session's, for the first folder
    Ok(Pane { hwnd: h, eb, cookie, disp: String::new(), pending, settle: None })
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
    let (main, n) = app(|a| (a.main, a.s.panes()));
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
    moves.push((ctl(ID_EDIT), inside(pg.addr, s(8), s(6))));
    moves.push((ctl(ID_FILTER), inside(pg.filt, s(24), s(6))));
    moves.push((ctl(ID_FAV), pg.fav));
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
        let tree = RECT { left: sr.left + s(2), top: sr.top + s(4), right: sr.right - s(2), bottom: sr.bottom - s(2) };
        let mut moves = vec![(a.tree_hwnd, tree)];
        let mut x = g.pad;
        let tb = |id: i32| unsafe { GetDlgItem(Some(a.main), id) }.unwrap_or_default();
        for (id, w) in [(ID_TB_LAYOUT, s(70)), (ID_TB_SAVE, s(32)), (ID_TB_LOAD, s(32)), (ID_TB_VIEW, s(32)), (ID_TB_FAV, s(32))] {
            moves.push((tb(id), RECT { left: x, top: s(5), right: x + w, bottom: s(5) + s(24) }));
            x += w + s(4);
        }
        let right = g.w - g.pad; // the theme toggle sits at the far right
        moves.push((tb(ID_TB_THEME), RECT { left: right - s(32), top: s(5), right, bottom: s(5) + s(24) }));
        for (i, p) in a.panes.iter().enumerate().take(a.s.panes()) {
            if let Some(p) = p {
                moves.push((p.hwnd, cell_rect(&g, &a.s.heights, i)));
            }
        }
        Some((a.main, moves, layout_label(&a.s.heights)))
    }) else {
        return;
    };
    move_all(&moves, None);
    start_reveal(main); // sidebar size may have changed: keep its selection in view
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

// Splitter drag shows translucent guide bars where the boundaries will land; the panes move once, on release.
unsafe fn show_bars() {
    let Some((main, bars, rects)) = app(|a| {
        let d = a.drag?;
        let g = geo(a);
        let t = 3 * a.dpi / 96;
        let vbar = |x: i32| RECT { left: x - t / 2, top: g.top, right: x - t / 2 + t, bottom: g.bot };
        let hbar = |(x0, x1): (i32, i32), y: i32| RECT { left: x0, top: y - t / 2, right: x1, bottom: y - t / 2 + t };
        let rects = match d {
            Hit::Side => [Some(vbar(g.pad + g.side + g.gap / 2)), None],
            Hit::Col(k) => [Some(vbar(g.xs[k].1 + g.gap / 2)), None],
            Hit::Row(c, k) => [None, Some(hbar(g.xs[c], g.ys[c][k].1 + g.gap / 2))],
        };
        Some((a.main, a.bars, rects))
    }) else {
        return;
    };
    for (b, r) in bars.into_iter().zip(rects) {
        match r {
            Some(r) => {
                let mut p = POINT { x: r.left, y: r.top };
                _ = ClientToScreen(main, &mut p);
                _ = SetWindowPos(b, None, p.x, p.y, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
            }
            None => _ = ShowWindow(b, SW_HIDE),
        }
    }
}

extern "system" fn bar_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { DefWindowProcW(h, msg, wp, lp) }
}

unsafe fn end_drag() {
    let Some(bars) = app(|a| a.drag.take().map(|_| a.bars)) else { return };
    for b in bars {
        _ = ShowWindow(b, SW_HIDE);
    }
    relayout();
    redraw_all();
}

// Full repaint of everything once a drag/resize/layout change is over, so nothing stale survives.
unsafe fn redraw_all() {
    let main = app(|a| a.main);
    if !main.0.is_null() {
        _ = RedrawWindow(Some(main), None, None, RDW_INVALIDATE | RDW_ERASE | RDW_ALLCHILDREN);
    }
}

unsafe fn set_active(i: usize) {
    let (old, hs) = app(|a| {
        let old = std::mem::replace(&mut a.s.active, i);
        (old, [old, i].map(|k| a.panes[k].as_ref().map(|p| p.hwnd)))
    });
    if old != i {
        for h in hs.into_iter().flatten() {
            repaint_header(h);
        }
        if let Some(item) = view_of::<IFolderView2>(i).and_then(|fv| fv.GetFolder::<IShellItem>().ok()) {
            sync_tree(&item);
        }
    }
}

unsafe fn repaint_header(pane: HWND) {
    _ = InvalidateRect(Some(pane), None, false);
    for id in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN, ID_FAV, ID_EDIT, ID_FILTER] {
        if let Ok(c) = GetDlgItem(Some(pane), id) {
            _ = InvalidateRect(Some(c), None, true);
        }
    }
}

// The sidebar follows the active pane: expand down to its folder and select it.
// (Programmatic selection doesn't navigate: OnSelectionChanged only follows user input.)
unsafe fn sync_tree(item: &IShellItem) {
    let Some((t, main, th)) = app(|a| a.tree.clone().map(|t| (t, a.main, a.tree_hwnd))) else { return };
    // the tree scrolls on its own when it selects; hide that and paint once, after the reveal
    let tv = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()).ok();
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    }
    _ = t.EnsureItemVisible(item);
    _ = t.SetItemState(item, NSTCIS_SELECTED.0 as u32, NSTCIS_SELECTED.0 as u32);
    reveal_tree_selection(); // turns redraw back on
    if let Some(tv) = tv {
        SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
        _ = InvalidateRect(Some(tv), None, true);
    }
    start_reveal(main); // and again while the tree finishes expanding
}

// Scrolls the tree so the selected folder is on screen: vertically, and sideways when deep nesting
// pushes its name past a narrow sidebar.
unsafe fn reveal_tree_selection() {
    let (th, dpi) = app(|a| (a.tree_hwnd, a.dpi));
    let Ok(tv) = FindWindowExW(Some(th), None, w!("SysTreeView32"), PCWSTR::null()) else { return };
    let item = SendMessageW(tv, TVM_GETNEXTITEM, Some(WPARAM(TVGN_CARET)), None).0;
    if item == 0 {
        return;
    }
    let label = || {
        let mut r = RECT::default();
        *(&mut r as *mut RECT as *mut isize) = item; // TVM_GETITEMRECT takes the item in the RECT
        let ok = SendMessageW(tv, TVM_GETITEMRECT, Some(WPARAM(1)), Some(LPARAM(&mut r as *mut _ as isize))).0 != 0;
        ok.then_some(r)
    };
    let mut c = RECT::default();
    _ = GetClientRect(tv, &mut c);
    if c.right == 0 {
        return; // not laid out yet; relayout asks again
    }
    let margin = 16 * dpi / 96;
    // all the scrolling happens unseen, then one repaint if anything moved
    let pos = || (GetScrollPos(tv, SB_HORZ), GetScrollPos(tv, SB_VERT));
    let before = pos();
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(0)), None);
    SendMessageW(tv, TVM_ENSUREVISIBLE, None, Some(LPARAM(item)));
    // sideways, a line at a time (avoids guessing the tree's scroll units): the selected folder's name
    // wins over the roots on the left, which may get cut off. One direction only, so a name wider than
    // the sidebar can't bounce between the two rules.
    let left = label().is_some_and(|r| r.left < margin);
    for _ in 0..300 {
        match label() {
            Some(r) if left && r.left < margin && GetScrollPos(tv, SB_HORZ) > 0 => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None), // SB_LINELEFT
            Some(r) if !left && r.right > c.right && r.left > margin => _ = SendMessageW(tv, WM_HSCROLL, Some(WPARAM(1)), None), // SB_LINERIGHT
            _ => break,
        }
    }
    if !left && label().is_some_and(|r| r.left < margin) {
        SendMessageW(tv, WM_HSCROLL, Some(WPARAM(0)), None); // overshot: back one, so the next pass is a no-op
    }
    SendMessageW(tv, WM_SETREDRAW, Some(WPARAM(1)), None);
    if pos() != before {
        _ = InvalidateRect(Some(tv), None, true);
    }
}

// The tree expands down to a new folder asynchronously, so the reveal runs a few times as it settles.
unsafe fn start_reveal(main: HWND) {
    app(|a| a.reveal_tries = 3);
    SetTimer(Some(main), TIMER_REVEAL, 120, None);
}

unsafe fn set_accent(c: u32) {
    let (bars, pane) = app(|a| {
        if a.s.dark {
            a.s.accent_dark = c;
        } else {
            a.s.accent = c;
        }
        (a.bars, a.panes.get(a.s.active).and_then(|p| p.as_ref()).map(|p| p.hwnd))
    });
    let old = SetClassLongPtrW(bars[0], GCLP_HBRBACKGROUND, CreateSolidBrush(cref(c)).0 as isize);
    if old != 0 {
        _ = DeleteObject(HGDIOBJ(old as _));
    }
    if let Some(p) = pane {
        repaint_header(p);
    }
}

// The hosted Explorer list ignores the app-wide dark mode, so its (new, per folder) list view is dyed here.
unsafe fn dark_list(pane: HWND) {
    extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
        let mut c = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut c) } as usize;
        let class = String::from_utf16_lossy(&c[..n]);
        if class == "SysListView32" || class == "SHELLDLL_DefView" {
            unsafe { (*(out.0 as *mut Vec<(String, HWND)>)).push((class, h)) };
        }
        true.into()
    }
    let mut found: Vec<(String, HWND)> = Vec::new();
    _ = EnumChildWindows(Some(pane), Some(collect), LPARAM(&mut found as *mut _ as isize));
    // the view paints the name column itself (custom draw, light colours); recolour it on the way back
    for (_, v) in found.iter().filter(|(c, _)| c == "SHELLDLL_DefView") {
        _ = SetWindowSubclass(*v, Some(dark_view_proc), 1, 0);
    }
    let lists: Vec<HWND> = found.into_iter().filter(|(c, _)| c == "SysListView32").map(|(_, h)| h).collect();
    let (card, text) = (cref(pal().card).0 as isize, cref(pal().text).0 as isize);
    for lv in lists {
        _ = SetWindowTheme(lv, w!("DarkMode_Explorer"), PCWSTR::null()); // also darkens its scrollbars
        // the sorted-column tint stays white in dark mode
        _ = SetWindowSubclass(lv, Some(dark_list_proc), 2, 0);
        SendMessageW(lv, LVM_SETSELECTEDCOLUMN, Some(WPARAM(usize::MAX)), None);
        SendMessageW(lv, LVM_SETBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTBKCOLOR, None, Some(LPARAM(card)));
        SendMessageW(lv, LVM_SETTEXTCOLOR, None, Some(LPARAM(text)));
        let header = HWND(SendMessageW(lv, LVM_GETHEADER, None, None).0 as _);
        if !header.0.is_null() {
            _ = SetWindowTheme(header, w!("DarkMode_ItemsView"), PCWSTR::null());
        }
        _ = InvalidateRect(Some(lv), None, true);
    }
}

// The view re-sets the sorted column through an internal interface (no window message to catch),
// so it is cleared right before each paint instead. The column header is the list's child, so its
// draw notifications come through here too.
unsafe extern "system" fn dark_list_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let dark = DARK_ON.load(Ordering::Relaxed);
    if msg == WM_PAINT && dark && DefSubclassProc(h, LVM_GETSELECTEDCOLUMN, WPARAM(0), LPARAM(0)).0 as i32 != -1 {
        DefSubclassProc(h, LVM_SETSELECTEDCOLUMN, WPARAM(usize::MAX), LPARAM(0));
    }
    if msg == WM_NOTIFY && dark && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &*(lp.0 as *const NMCUSTOMDRAW);
        if cd.hdr.hwndFrom.0 as isize == DefSubclassProc(h, LVM_GETHEADER, WPARAM(0), LPARAM(0)).0 {
            if cd.dwDrawStage == CDDS_PREPAINT {
                return LRESULT(CDRF_NOTIFYITEMDRAW as isize);
            }
            if cd.dwDrawStage == CDDS_ITEMPREPAINT {
                draw_header_item(cd);
                return LRESULT(CDRF_SKIPDEFAULT as isize);
            }
        }
    }
    DefSubclassProc(h, msg, wp, lp)
}

// The column header's theme draws its labels in the light theme's dark grey whatever colour it is
// given, so in dark mode each column is drawn here: flat background, label, divider, sort chevron.
unsafe fn draw_header_item(cd: &NMCUSTOMDRAW) {
    let (dc, rc, hdr) = (cd.hdc, cd.rc, cd.hdr.hwndFrom);
    let s = |v: i32| v * GetDpiForWindow(hdr) as i32 / 96;
    let hot = cd.uItemState.0 & (CDIS_HOT.0 | CDIS_SELECTED.0) != 0;
    fill(dc, rc, if hot { blend(pal().card, pal().text, 0.08) } else { pal().card });
    fill(dc, RECT { left: rc.right - 1, ..rc }, pal().line);
    let mut text = [0u16; 260];
    let mut item = HDITEMW { mask: HDI_TEXT | HDI_FORMAT, pszText: PWSTR(text.as_mut_ptr()), cchTextMax: text.len() as i32, ..Default::default() };
    SendMessageW(hdr, HDM_GETITEMW, Some(WPARAM(cd.dwItemSpec)), Some(LPARAM(&mut item as *mut _ as isize)));
    let n = text.iter().position(|&c| c == 0).unwrap_or(0);
    if n > 0 {
        let align = if item.fmt.0 & HDF_RIGHT.0 != 0 {
            DT_RIGHT
        } else if item.fmt.0 & HDF_CENTER.0 != 0 {
            DT_CENTER
        } else {
            DT_LEFT
        };
        let mut r = RECT { left: rc.left + s(7), right: rc.right - s(7), ..rc };
        let font = SelectObject(dc, HGDIOBJ(SendMessageW(hdr, WM_GETFONT, None, None).0 as _));
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, cref(pal().sub));
        DrawTextW(dc, &mut text[..n], &mut r, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
        SelectObject(dc, font);
    }
    // sort chevron along the top edge, centred, as Windows 11 draws it
    let up = item.fmt.0 & HDF_SORTUP.0 != 0;
    if up || item.fmt.0 & HDF_SORTDOWN.0 != 0 {
        let (cx, t, arm) = ((rc.left + rc.right) / 2, s(1).max(1), s(4));
        for k in 0..=arm {
            let y = rc.top + s(1) + if up { k } else { arm - k };
            fill(dc, RECT { left: cx - k, top: y, right: cx - k + t, bottom: y + t }, pal().sub);
            fill(dc, RECT { left: cx + k, top: y, right: cx + k + t, bottom: y + t }, pal().sub);
        }
    }
}

unsafe extern "system" fn dark_view_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
    let r = DefSubclassProc(h, msg, wp, lp);
    if msg == WM_NOTIFY && DARK_ON.load(Ordering::Relaxed) && (*(lp.0 as *const NMHDR)).code == NM_CUSTOMDRAW {
        let cd = &mut *(lp.0 as *mut NMLVCUSTOMDRAW);
        let stage = cd.nmcd.dwDrawStage.0;
        if stage == CDDS_PREPAINT.0 {
            return LRESULT(r.0 | CDRF_NOTIFYITEMDRAW as isize);
        }
        if stage & CDDS_ITEMPREPAINT.0 == CDDS_ITEMPREPAINT.0 {
            (cd.clrText, cd.clrTextBk) = (cref(pal().text), cref(pal().card));
            if stage & CDDS_SUBITEM.0 == 0 {
                return LRESULT(r.0 | CDRF_NOTIFYSUBITEMDRAW as isize);
            }
        }
    }
    r
}

// Undocumented but long-stable uxtheme exports (Windows 10 1903+): app-wide dark mode, which is what
// makes menus, scrollbars and the hosted Explorer views follow the theme.
unsafe fn uxtheme(ordinal: usize) -> FARPROC {
    let m = LoadLibraryW(w!("uxtheme.dll")).ok()?;
    GetProcAddress(m, PCSTR(ordinal as *const u8))
}

// Applies the current theme everywhere; panes are rebuilt so their Explorer views pick it up.
unsafe fn apply_theme() {
    let (main, dark, tree, accent) = app(|a| (a.main, a.s.dark, a.tree_hwnd, a.s.accent_now()));
    DARK_ON.store(dark, Ordering::Relaxed);
    if let Some(f) = uxtheme(135) {
        let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
        set_preferred_app_mode(if dark { 2 } else { 3 }); // ForceDark / ForceLight
    }
    if let Some(f) = uxtheme(136) {
        let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
        flush_menu_themes();
    }
    let on = BOOL::from(dark);
    _ = DwmSetWindowAttribute(main, DWMWINDOWATTRIBUTE(20), &on as *const _ as _, 4); // immersive dark title bar
    let caption = cref(pal().bg);
    _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
    if let Ok(tv) = FindWindowExW(Some(tree), None, w!("SysTreeView32"), PCWSTR::null()) {
        _ = SetWindowTheme(tv, if dark { w!("DarkMode_Explorer") } else { w!("Explorer") }, PCWSTR::null());
        let colour = |c: u32| LPARAM(if dark { cref(c).0 as isize } else { -1 }); // -1: system default
        SendMessageW(tv, TVM_SETBKCOLOR, None, Some(colour(pal().card)));
        SendMessageW(tv, TVM_SETTEXTCOLOR, None, Some(colour(pal().text)));
    }
    set_accent(accent);
    for i in 0..MAXP {
        destroy_pane(i);
    }
    relayout();
    redraw_all();
}

unsafe fn pick_accent() {
    let (main, cur) = app(|a| (a.main, a.s.accent_now()));
    let mut custom = [COLORREF(0xFFFFFF); 16];
    let mut cc = CHOOSECOLORW {
        lStructSize: size_of::<CHOOSECOLORW>() as u32,
        hwndOwner: main,
        rgbResult: cref(cur),
        lpCustColors: custom.as_mut_ptr(),
        Flags: CC_RGBINIT | CC_FULLOPEN,
        ..Default::default()
    };
    if ChooseColorW(&mut cc).as_bool() {
        set_accent(cref(cc.rgbResult.0).0); // COLORREF is BGR; cref swaps back
    }
}

// A double-click on empty space in a list (the first click cleared the selection) goes up a folder.
unsafe fn up_candidate(m: &MSG) -> Option<usize> {
    if m.message != WM_LBUTTONDBLCLK || !app(|a| a.s.dblup) {
        return None;
    }
    let i = pane_of(m.hwnd)?;
    let svh = view_of::<IShellView>(i)?.GetWindow().ok()?;
    let mut c = [0u16; 32];
    let n = GetClassNameW(m.hwnd, &mut c) as usize;
    let list = matches!(String::from_utf16_lossy(&c[..n]).as_str(), "DirectUIHWND" | "SysListView32");
    (list && (svh == m.hwnd || IsChild(svh, m.hwnd).as_bool())).then_some(i)
}

unsafe fn cycle(d: i32) {
    let (a, n) = app(|a| (a.s.active as i32, a.s.panes() as i32));
    let i = (a + d).rem_euclid(n) as usize;
    set_active(i);
    focus_view(i);
}

unsafe fn set_layout(h: Vec<usize>) {
    app(|a| a.s.set_heights(h));
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
    let (main, n) = app(|a| (a.main, a.s.panes()));
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

// Save button / Ctrl+S: overwrite the current session file; the first time, ask for a name.
unsafe fn save_current() {
    let file = app(|a| a.s.file.clone());
    if file.is_empty() {
        _ = save_session_as();
    } else {
        write_session(Path::new(&file));
    }
}

unsafe fn write_session(path: &Path) {
    for i in 0..app(|a| a.s.panes()) {
        save_view(i);
    }
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

// "New name": the save dialog does naming, overwrite prompts, rename and delete for us.
unsafe fn save_session_as() -> Result<()> {
    let main = app(|a| a.main);
    let dir = sessions_dir();
    _ = std::fs::create_dir_all(&dir);
    let dlg: IFileSaveDialog = CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER)?;
    let kind = HSTRING::from(tr("Grids session (*.grids)", "Grids 세션 (*.grids)"));
    dlg.SetFileTypes(&[COMDLG_FILTERSPEC { pszName: PCWSTR(kind.as_ptr()), pszSpec: w!("*.grids") }])?;
    dlg.SetDefaultExtension(w!("grids"))?;
    dlg.SetTitle(&HSTRING::from(tr("Save session", "세션 저장")))?;
    if let Ok(f) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(dir.to_string_lossy().as_ref()), None) {
        dlg.SetFolder(&f)?;
    }
    dlg.Show(Some(main))?; // Err on cancel
    let path = pwstr(dlg.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?);
    write_session(Path::new(&path));
    Ok(())
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
    app(|a| {
        a.s.take_layout(&other);
        a.s.file = path.to_string_lossy().into_owned();
        (a.base, a.dirty) = (Some(a.s.clone()), false);
    });
    refresh_save_button();
    relayout();
    redraw_all();
    focus_view(app(|a| a.s.active));
}

// Paths from the command line or a second instance: a session file loads, folders fill panes from the active one on.
unsafe fn open_args(args: Vec<String>) {
    if let Some(f) = session_arg(&args) {
        return load_session_file(Path::new(f));
    }
    let (a, n) = app(|x| (x.s.active, x.s.panes()));
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
    let ko = KO.load(Ordering::Relaxed);
    let mut parts = vec![if ko { format!("항목 {n}개") } else { format!("{n} items") }];
    if sel > 0 {
        let size = match selected_size(&fv) {
            0 => String::new(),
            size => format!(" ({})", human(size)),
        };
        parts.push(if ko { format!("{sel}개 선택{size}") } else { format!("{sel} selected{size}") });
    }
    if !spec.is_empty() {
        parts.push(format!("{} {spec}", tr("Filter", "필터")));
    }
    let drive = path.get(1..2) == Some(":");
    if drive || path.starts_with(r"\\") {
        // ponytail: a dead network share can stall this call once per change
        let (mut free, mut total) = (0, 0);
        if GetDiskFreeSpaceExW(&HSTRING::from(&path), Some(&mut free), Some(&mut total), None).is_ok() {
            let label = if drive { &path[..2] } else { tr("Network", "네트워크") };
            parts.push(format!("{label} {} {} / {}", tr("free", "여유"), human(free), human(total)));
        }
    }
    let rc = app(|a| {
        (a.status_key, a.status) = (key, parts.join("     ·     "));
        status_rect(&geo(a))
    });
    _ = InvalidateRect(Some(main), Some(&rc), true);
}

// ---------- painting ----------

// Ground, the sidebar plate and the status line. Pane plates are child windows (paint_pane).
unsafe fn paint_main(h: HWND) {
    let (g, status, font, dpi) = app(|a| (geo(a), a.status.clone(), a.font, a.dpi));
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().bg);
        let side = side_rect(&g);
        fill(dc, side, pal().card);
        frame(dc, side, 1, pal().line);
        let st = status_rect(&g);
        text(dc, RECT { left: g.pad + 4 * dpi / 96, ..st }, &status, font, pal().sub, DT_LEFT);
    });
}

// One white plate per pane: a single accent border when active, flat fields in the top bar.
unsafe fn paint_pane(h: HWND) {
    let (active, dpi, icon_font, accent) =
        app(|a| (a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h), a.dpi, a.icon_font, a.s.accent_now()));
    let (head, ink) = header_colors(h);
    let focus = GetFocus();
    let focused = |id: i32| GetDlgItem(Some(h), id).is_ok_and(|e| e == focus);
    let (addr_on, filt_on) = (focused(ID_EDIT), focused(ID_FILTER));
    buffered(h, |dc, rc| {
        let s = |v: i32| v * dpi / 96;
        let pg = pane_geo(rc.right, rc.bottom, dpi);
        fill(dc, rc, pal().card);
        // active: header bar in the selection colour, same colour as the border
        fill(dc, RECT { bottom: pg.view.top, ..rc }, head);
        if active {
            frame(dc, rc, s(2), accent);
        } else {
            frame(dc, rc, 1, pal().line);
            fill(dc, RECT { left: pg.view.left, top: pg.view.top - 1, right: pg.view.right, bottom: pg.view.top }, pal().line);
        }
        for (p, on) in [(pg.addr, addr_on), (pg.filt, filt_on)] {
            fill(dc, p, pal().pill);
            if on {
                frame(dc, p, 1, if active { ink } else { accent });
            }
        }
        let icon = RECT { left: pg.filt.left + s(4), right: pg.filt.left + s(22), ..pg.filt };
        text(dc, icon, ICON_SEARCH, icon_font, pal().sub, DT_CENTER);
    });
}

// ---------- layout picker: click = even grid, press and drag across columns = rows per column, presets ----------

const PRESETS: [&[usize]; 6] = [&[1, 2], &[1, 3], &[2, 1], &[3, 1], &[1, 1, 2], &[2, 3]];

struct PickGeo {
    w: i32,
    h: i32,
    pad: i32,
    cell: i32,
    gap: i32,
    label: RECT,
    presets: [RECT; 6],
}

fn pick_geo(dpi: i32) -> PickGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, cell, gap) = (s(12), s(24), s(6));
    let gw = 4 * cell + 3 * gap;
    let label = RECT { left: pad, top: pad + gw, right: pad + gw, bottom: pad + gw + s(26) };
    let (pw, ph) = ((gw - 2 * gap) / 3, s(24));
    let presets = std::array::from_fn(|i| {
        let (x, y) = (pad + (i as i32 % 3) * (pw + gap), label.bottom + (i as i32 / 3) * (ph + gap));
        RECT { left: x, top: y, right: x + pw, bottom: y + ph }
    });
    PickGeo { w: 2 * pad + gw, h: label.bottom + 2 * ph + gap + pad, pad, cell, gap, label, presets }
}

fn pick_cell(g: &PickGeo, r: usize, c: usize) -> RECT {
    let (x, y) = (g.pad + c as i32 * (g.cell + g.gap), g.pad + r as i32 * (g.cell + g.gap));
    RECT { left: x, top: y, right: x + g.cell, bottom: y + g.cell }
}

/// (row, column) under the point; `clamp` keeps a drag that leaves the cells on the nearest one.
fn pick_at(g: &PickGeo, x: i32, y: i32, clamp: bool) -> Option<(usize, usize)> {
    let span = 4 * g.cell + 3 * g.gap;
    let inside = (g.pad..g.pad + span).contains(&x) && (g.pad..g.pad + span).contains(&y);
    let at = |v: i32| ((v - g.pad).max(0) / (g.cell + g.gap)).min(3) as usize;
    (inside || clamp).then(|| (at(y), at(x)))
}

/// Dragging: columns follow the cursor; the column under it takes the cursor's row.
fn drag_heights(mut h: Vec<usize>, r: usize, c: usize) -> Vec<usize> {
    h.resize(c + 1, r + 1);
    h[c] = r + 1;
    h
}

unsafe fn open_grid() {
    let (main, dpi, open, closed, cur) = app(|a| (a.main, a.dpi, a.grid, a.grid_closed, a.s.heights.clone()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LAYOUT) {
        _ = GetWindowRect(b, &mut rc);
    }
    let g = pick_geo(dpi);
    app(|a| (a.grid_preview, a.grid_drag, a.grid_preset) = (cur, None, None));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsPicker"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.grid = h);
}

unsafe fn paint_grid(h: HWND) {
    let (preview, cur, hot, dpi, font, accent) =
        app(|a| (a.grid_preview.clone(), a.s.heights.clone(), a.grid_preset, a.dpi, a.font, a.s.accent_now()));
    let soft = blend(accent, pal().card, 0.8);
    let g = pick_geo(dpi);
    let on = |v: &[usize], r: usize, c: usize| c < v.len() && r < v[c];
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        for r in 0..4 {
            for c in 0..4 {
                let col = if on(&preview, r, c) {
                    accent
                } else if on(&cur, r, c) {
                    soft
                } else {
                    pal().pill
                };
                fill(dc, pick_cell(&g, r, c), col);
            }
        }
        let label = match preview.iter().all(|&n| n == preview[0]) {
            true if KO.load(Ordering::Relaxed) => format!("{}행 × {}열", preview[0], preview.len()),
            true => format!("{} × {}", preview[0], preview.len()),
            false => format!("{}  {}", tr("Per column", "열마다"), layout_label(&preview)),
        };
        text(dc, g.label, &label, font, pal().text, DT_CENTER);
        // preset thumbnails: the panes of each layout on a tile
        for (i, (p, t)) in PRESETS.iter().zip(g.presets).enumerate() {
            let (tile, pane) = if hot == Some(i) { (soft, accent) } else { (pal().pill, blend(pal().card, pal().text, 0.25)) };
            fill(dc, t, tile);
            let inner = RECT { left: t.left + 3, top: t.top + 3, right: t.right - 3, bottom: t.bottom - 3 };
            for (c, (x0, x1)) in cells(inner.left, inner.right - inner.left, &even(p.len()), 2).into_iter().enumerate() {
                for (y0, y1) in cells(inner.top, inner.bottom - inner.top, &even(p[c]), 2) {
                    fill(dc, RECT { left: x0, top: y0, right: x1, bottom: y1 }, pane);
                }
            }
        }
    });
}

extern "system" fn grid_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = pick_geo(app(|a| a.dpi));
        let show = |v: Vec<usize>, preset: Option<usize>| {
            let changed = app(|a| {
                let changed = a.grid_preview != v || a.grid_preset != preset;
                (a.grid_preview, a.grid_preset) = (v, preset);
                changed
            });
            if changed {
                _ = InvalidateRect(Some(h), None, false);
            }
        };
        let apply = |v: Vec<usize>| {
            _ = DestroyWindow(h);
            set_layout(v);
        };
        match msg {
            WM_PAINT => paint_grid(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN => {
                let (x, y) = xy(lp);
                if let Some((r, c)) = pick_at(&g, x, y, false) {
                    SetCapture(h);
                    let v = vec![r + 1; c + 1];
                    app(|a| a.grid_drag = Some(v.clone()));
                    show(v, None);
                }
            }
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                if let Some(d) = app(|a| a.grid_drag.clone()) {
                    if let Some((r, c)) = pick_at(&g, x, y, true) {
                        let v = drag_heights(d, r, c);
                        app(|a| a.grid_drag = Some(v.clone()));
                        show(v, None);
                    }
                } else if let Some((r, c)) = pick_at(&g, x, y, false) {
                    show(vec![r + 1; c + 1], None);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    show(PRESETS[i].to_vec(), Some(i));
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                if let Some(v) = app(|a| a.grid_drag.take()) {
                    _ = ReleaseCapture();
                    apply(v);
                } else if let Some(i) = g.presets.iter().position(|t| x >= t.left && x < t.right && y >= t.top && y < t.bottom) {
                    apply(PRESETS[i].to_vec());
                }
            }
            WM_KEYDOWN => {
                // keyboard picks even grids: arrows move the corner, Enter applies
                let v = app(|a| a.grid_preview.clone());
                let (r, c) = (v[v.len() - 1] - 1, v.len() - 1);
                let (r, c) = match VIRTUAL_KEY(wp.0 as u16) {
                    VK_LEFT => (r, c.saturating_sub(1)),
                    VK_RIGHT => (r, (c + 1).min(3)),
                    VK_UP => (r.saturating_sub(1), c),
                    VK_DOWN => ((r + 1).min(3), c),
                    VK_RETURN => {
                        apply(v);
                        return LRESULT(0);
                    }
                    VK_ESCAPE => {
                        _ = DestroyWindow(h);
                        return LRESULT(0);
                    }
                    _ => (r, c),
                };
                show(vec![r + 1; c + 1], None);
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.grid, a.grid_drag, a.grid_closed) = (HWND::default(), None, Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- load popup: saved sessions as cards with a to-scale layout preview ----------

const LOAD_SAVE_AS: usize = 1000;
const LOAD_FOLDER: usize = 1001;
const LOAD_MAX: usize = 9;

struct LoadGeo {
    w: i32,
    h: i32,
    cards: Vec<(RECT, RECT)>, // (card, thumbnail)
    save_as: RECT,
    folder: RECT,
}

fn load_geo(n: usize, dpi: i32) -> LoadGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, gap, cw, th, ch) = (s(12), s(10), s(140), s(84), s(84 + 44));
    let rows = n.div_ceil(3).max(1) as i32;
    let w = 2 * pad + 3 * cw + 2 * gap;
    let cards = (0..n as i32)
        .map(|k| {
            let (x, y) = (pad + (k % 3) * (cw + gap), pad + (k / 3) * (ch + gap));
            let card = RECT { left: x, top: y, right: x + cw, bottom: y + ch };
            (card, RECT { left: x + s(6), top: y + s(6), right: x + cw - s(6), bottom: y + s(6) + th - s(12) })
        })
        .collect();
    let fy = pad + rows * (ch + gap);
    let link = |x0: i32, x1: i32| RECT { left: x0, top: fy, right: x1, bottom: fy + s(30) };
    LoadGeo { w, h: fy + s(30) + s(8), cards, save_as: link(pad, w / 2), folder: link(w / 2, w - pad) }
}

fn short_name(path: &str) -> String {
    if path.contains("20D04FE0-3AEA-1069-A2D8-08002B30309D") {
        return tr("This PC", "내 PC").into();
    }
    let t = path.trim_end_matches('\\');
    t.rsplit('\\').next().filter(|x| !x.starts_with("::")).unwrap_or(t).to_string()
}

fn ago(t: std::time::SystemTime) -> String {
    let m = t.elapsed().map(|d| d.as_secs() / 60).unwrap_or(0);
    match m {
        0 => tr("just now", "방금").into(),
        1..=59 => format!("{m}{}", tr(" min ago", "분 전")),
        60..=1439 => format!("{}{}", m / 60, tr(" h ago", "시간 전")),
        _ => format!("{}{}", m / 1440, tr(" d ago", "일 전")),
    }
}

// The layout drawn to scale, each pane labelled with its folder name.
unsafe fn draw_thumb(dc: HDC, r: RECT, s: &Session, font: HFONT) {
    fill(dc, r, pal().bg);
    let xs = cells(r.left + 2, r.right - r.left - 4, &s.colr, 2);
    let mut i = 0;
    for (c, &(x0, x1)) in xs.iter().enumerate() {
        for (y0, y1) in cells(r.top + 2, r.bottom - r.top - 4, &s.rowr[c], 2) {
            let pane = RECT { left: x0, top: y0, right: x1, bottom: y1 };
            fill(dc, pane, pal().card);
            frame(dc, pane, 1, pal().line);
            if x1 - x0 > 24 && y1 - y0 > 12 {
                let inner = RECT { left: x0 + 3, right: x1 - 3, ..pane };
                text(dc, inner, &short_name(&s.paths[i]), font, pal().sub, DT_CENTER);
            }
            i += 1;
        }
    }
}

unsafe fn open_load() {
    let (main, dpi, open, closed) = app(|a| (a.main, a.dpi, a.load, a.load_closed));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let entries = std::fs::read_dir(sessions_dir()).into_iter().flatten().flatten();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .map(|e| e.path())
        .filter(|p| is_session_file(p))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0)); // most recent first
    let cards: Vec<_> = files
        .into_iter()
        .take(LOAD_MAX)
        .filter_map(|(t, p)| Some((p.clone(), Session::parse(&std::fs::read_to_string(&p).ok()?), ago(t))))
        .collect();
    let g = load_geo(cards.len(), dpi);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_LOAD) {
        _ = GetWindowRect(b, &mut rc);
    }
    app(|a| (a.cards, a.load_hot) = (cards, None));
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsLoad"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.load = h);
}

unsafe fn paint_load(h: HWND) {
    let (cards, hot, cur, dpi, font, small, accent) = app(|a| {
        let cards: Vec<_> = a.cards.iter().map(|(p, s, t)| (p.clone(), s.clone(), t.clone())).collect();
        (cards, a.load_hot, a.s.file.clone(), a.dpi, a.font, a.small_font, a.s.accent_now())
    });
    let g = load_geo(cards.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if cards.is_empty() {
            let r = RECT { bottom: g.save_as.top, ..rc };
            text(dc, r, tr("No saved sessions", "저장된 세션이 없습니다"), font, pal().sub, DT_CENTER);
        }
        for (k, ((path, sess, when), (card, thumb))) in cards.iter().zip(&g.cards).enumerate() {
            if hot == Some(k) {
                fill(dc, *card, pal().pill);
            }
            draw_thumb(dc, *thumb, sess, small);
            if path.to_string_lossy() == cur.as_str() {
                frame(dc, *thumb, s(2), accent); // the session you're in
            }
            let name = path.file_stem().unwrap_or_default().to_string_lossy();
            let line = |top: i32, hgt: i32| RECT { left: card.left + s(6), right: card.right - s(6), top, bottom: top + hgt };
            text(dc, line(thumb.bottom + s(4), s(20)), &name, font, pal().text, DT_LEFT);
            let meta = format!("{}  ·  {}", layout_label(&sess.heights), when);
            text(dc, line(thumb.bottom + s(22), s(16)), &meta, small, pal().sub, DT_LEFT);
        }
        fill(dc, RECT { left: s(12), top: g.save_as.top - 1, right: rc.right - s(12), bottom: g.save_as.top }, pal().line);
        let link = |r: RECT, label: &str, k: usize, align| {
            text(dc, r, label, font, if hot == Some(k) { pal().text } else { accent }, align);
        };
        link(g.save_as, tr("+ Save as new", "+ 새 이름으로 저장"), LOAD_SAVE_AS, DT_LEFT);
        link(g.folder, tr("Open sessions folder", "세션 폴더 열기"), LOAD_FOLDER, DT_RIGHT);
    });
}

fn load_hit(g: &LoadGeo, x: i32, y: i32) -> Option<usize> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    if inside(&g.save_as) {
        return Some(LOAD_SAVE_AS);
    }
    if inside(&g.folder) {
        return Some(LOAD_FOLDER);
    }
    g.cards.iter().position(|(c, _)| inside(c))
}

extern "system" fn load_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = app(|a| load_geo(a.cards.len(), a.dpi));
        match msg {
            WM_PAINT => paint_load(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = load_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.load_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                let Some(k) = load_hit(&g, x, y) else { return LRESULT(0) };
                let file = app(|a| a.cards.get(k).map(|c| c.0.clone()));
                _ = DestroyWindow(h);
                match k {
                    LOAD_SAVE_AS => _ = save_session_as(),
                    LOAD_FOLDER => {
                        let d = sessions_dir();
                        _ = std::fs::create_dir_all(&d);
                        navigate(app(|a| a.s.active), &d.to_string_lossy());
                    }
                    _ => {
                        if let Some(f) = file {
                            load_session_file(&f);
                        }
                    }
                }
            }
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => _ = DestroyWindow(h),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.load, a.load_closed) = (HWND::default(), Some(Instant::now()))),
            _ => return DefWindowProcW(h, msg, wp, lp),
        }
        LRESULT(0)
    }
}

// ---------- favourites: ☆+ in each pane header, the ★ list in the toolbar ----------

unsafe fn repaint_fav_buttons() {
    for h in app(|a| a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>()) {
        if let Ok(b) = GetDlgItem(Some(h), ID_FAV) {
            _ = InvalidateRect(Some(b), None, false);
        }
    }
}

unsafe fn toggle_fav(i: usize) {
    app(|a| {
        let path = a.s.paths[i].clone();
        if a.s.is_fav(&path) {
            a.s.favs.retain(|f| !f.eq_ignore_ascii_case(&path));
        } else if !path.is_empty() {
            a.s.favs.push(path);
        }
    });
    repaint_fav_buttons();
}

struct FavGeo {
    w: i32,
    h: i32,
    rows: Vec<(RECT, RECT)>, // (row, remove ✕)
}

fn fav_geo(n: usize, dpi: i32) -> FavGeo {
    let s = |v: i32| v * dpi / 96;
    let (pad, rh, w) = (s(6), s(40), s(380));
    let rows = (0..n as i32)
        .map(|k| {
            let row = RECT { left: pad, top: pad + k * rh, right: w - pad, bottom: pad + (k + 1) * rh };
            (row, RECT { left: row.right - s(32), ..row })
        })
        .collect();
    FavGeo { w, h: 2 * pad + n.max(1) as i32 * rh, rows }
}

unsafe fn open_favs() {
    let (main, dpi, open, closed, n) = app(|a| (a.main, a.dpi, a.fav_popup, a.fav_closed, a.s.favs.len()));
    // a click on the button first deactivates (closes) the popup; don't reopen it right away
    if !open.0.is_null() || closed.is_some_and(|t| t.elapsed().as_millis() < 300) {
        return;
    }
    let g = fav_geo(n, dpi);
    let mut rc = RECT::default();
    if let Ok(b) = GetDlgItem(Some(main), ID_TB_FAV) {
        _ = GetWindowRect(b, &mut rc);
    }
    app(|a| a.fav_hot = None);
    let style = WS_POPUP | WS_VISIBLE;
    let y = rc.bottom + 6 * dpi / 96;
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, w!("GridsFav"), w!(""), style, rc.left, y, g.w, g.h, Some(main), None, None, None) else {
        return;
    };
    let square = DWMWCP_DONOTROUND;
    _ = DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
    app(|a| a.fav_popup = h);
}

unsafe fn paint_favs(h: HWND) {
    let (favs, hot, dpi, font, small, icon_font) = app(|a| (a.s.favs.clone(), a.fav_hot, a.dpi, a.font, a.small_font, a.icon_font));
    let g = fav_geo(favs.len(), dpi);
    let s = |v: i32| v * dpi / 96;
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        frame(dc, rc, 1, pal().line);
        if favs.is_empty() {
            text(dc, rc, tr("No favourites yet · add one with ☆+ in a pane header", "즐겨찾기가 없습니다 · 패널 머리줄의 ☆+로 추가"), font, pal().sub, DT_CENTER);
        }
        for (k, (f, (row, remove))) in favs.iter().zip(&g.rows).enumerate() {
            if hot.is_some_and(|(i, _)| i == k) {
                fill(dc, *row, pal().pill);
                let x_on = hot == Some((k, true));
                text(dc, *remove, "\u{E711}", icon_font, if x_on { pal().text } else { pal().sub }, DT_CENTER);
            }
            let icon = RECT { left: row.left + s(6), right: row.left + s(28), ..*row };
            text(dc, icon, "\u{E8B7}", icon_font, pal().sub, DT_CENTER);
            let line = |top: i32, hgt: i32| RECT { left: icon.right + s(6), right: remove.left - s(4), top, bottom: top + hgt };
            text(dc, line(row.top + s(3), s(19)), &short_name(f), font, pal().text, DT_LEFT);
            text(dc, line(row.top + s(21), s(16)), f, small, pal().sub, DT_LEFT | DT_PATH_ELLIPSIS);
        }
    });
}

fn fav_hit(g: &FavGeo, x: i32, y: i32) -> Option<(usize, bool)> {
    let inside = |r: &RECT| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
    g.rows.iter().position(|(row, _)| inside(row)).map(|k| (k, inside(&g.rows[k].1)))
}

extern "system" fn fav_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let g = app(|a| fav_geo(a.s.favs.len(), a.dpi));
        match msg {
            WM_PAINT => paint_favs(h),
            WM_ERASEBKGND => return LRESULT(1),
            WM_MOUSEMOVE => {
                let (x, y) = xy(lp);
                let hot = fav_hit(&g, x, y);
                if app(|a| std::mem::replace(&mut a.fav_hot, hot)) != hot {
                    _ = InvalidateRect(Some(h), None, false);
                }
            }
            WM_LBUTTONUP => {
                let (x, y) = xy(lp);
                match fav_hit(&g, x, y) {
                    Some((k, true)) => {
                        // ✕: drop it and shrink the list in place
                        app(|a| {
                            a.s.favs.remove(k);
                            a.fav_hot = None;
                        });
                        let g = app(|a| fav_geo(a.s.favs.len(), a.dpi));
                        _ = SetWindowPos(h, None, 0, 0, g.w, g.h, SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
                        _ = InvalidateRect(Some(h), None, false);
                        repaint_fav_buttons();
                    }
                    Some((k, false)) => {
                        let (i, path) = app(|a| (a.s.active, a.s.favs[k].clone()));
                        _ = DestroyWindow(h);
                        if !navigate(i, &path) {
                            _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
                        }
                        focus_view(i);
                    }
                    None => {}
                }
            }
            WM_KEYDOWN if wp.0 == VK_ESCAPE.0 as usize => _ = DestroyWindow(h),
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE => _ = PostMessageW(Some(h), WM_CLOSE, WPARAM(0), LPARAM(0)),
            WM_DESTROY => app(|a| (a.fav_popup, a.fav_closed) = (HWND::default(), Some(Instant::now()))),
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

// "New Text Document.txt", then "New Text Document (2).txt", … The number sits before the extension.
fn fresh_file(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let (stem, ext) = match base.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, format!(".{ext}")),
        _ => (base, String::new()),
    };
    let mut n = 2u32;
    loop {
        let name = format!("{stem} ({n}){ext}");
        if !taken(&name) || n == 9999 {
            return name;
        }
        n += 1;
    }
}

// File created by Ctrl+Shift+M, waiting for the view to list it so it can be selected and renamed.
struct PendingFile {
    pane: usize,
    name: String,
    left: u8,
}
static PENDING_FILE: Mutex<Option<PendingFile>> = Mutex::new(None);

// Ctrl+Shift+M: an empty text file in the active pane, then the shell's own rename (F2) once it shows up.
unsafe fn new_file(i: usize) {
    let dir = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    if !is_fs(&dir) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let base = tr("New Text Document.txt", "새 텍스트 문서.txt");
    let name = fresh_file(base, |n| Path::new(&dir).join(n).exists());
    let full = Path::new(&dir).join(&name);
    if std::fs::OpenOptions::new().write(true).create_new(true).open(&full).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let wide: Vec<u16> = full.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    SHChangeNotify(SHCNE_CREATE, SHCNF_PATHW, Some(wide.as_ptr() as _), None);
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.Refresh();
    }
    if let Ok(mut p) = PENDING_FILE.lock() {
        *p = Some(PendingFile { pane: i, name, left: 8 });
    }
    if let Some(h) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)) {
        SetTimer(Some(h), TIMER_NEWFILE, 40, None);
    }
    focus_view(i);
}

// The view lists the new file a moment after Refresh. Select it and start the shell rename.
unsafe fn select_new_file(i: usize, name: &str) -> bool {
    let Some(fv) = view_of::<IFolderView2>(i) else { return false };
    let Ok(folder) = fv.GetFolder::<IShellFolder>() else { return false };
    let mut pidl = std::ptr::null_mut();
    let parsed = folder.ParseDisplayName(HWND::default(), None, &HSTRING::from(name), None, &mut pidl, std::ptr::null_mut());
    if parsed.is_err() || pidl.is_null() {
        return false;
    }
    let Some(sv) = view_of::<IShellView>(i) else {
        CoTaskMemFree(Some(pidl as _));
        return false;
    };
    // SVSI_EDIT is the shell's rename (the same one F2 uses). If this view won't edit, selecting is enough.
    let edit = (SVSI_EDIT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32;
    let ok = sv.SelectItem(pidl, edit).is_ok()
        || sv.SelectItem(pidl, (SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0) as u32).is_ok();
    CoTaskMemFree(Some(pidl as _));
    ok
}

unsafe fn key(m: &MSG) -> bool {
    let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
    let (ctrl, shift, alt) = (down(VK_CONTROL), down(VK_SHIFT), down(VK_MENU));
    if let Some(i) = pane_of(m.hwnd) {
        set_active(i);
    }
    let a = active_pane();
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
        VK_S if ctrl => save_current(),
        VK_M if ctrl && shift && !alt && !addr && !filt => new_file(a),
        VK_F1 => open_help(),
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
            // While the user drags the window edge, lay the panes out once at the end (same cost reason as splitters).
            WM_SIZE => {
                if !app(|a| a.sizing) {
                    relayout();
                    redraw_all(); // maximize / restore
                }
            }
            WM_ENTERSIZEMOVE => app(|a| a.sizing = true),
            WM_EXITSIZEMOVE => {
                app(|a| a.sizing = false);
                relayout();
                redraw_all();
            }
            WM_PAINT => paint_main(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_main covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            WM_TIMER if wp.0 == TIMER_REVEAL => {
                _ = KillTimer(Some(h), TIMER_REVEAL);
                reveal_tree_selection();
                let left = app(|a| {
                    a.reveal_tries = a.reveal_tries.saturating_sub(1);
                    a.reveal_tries
                });
                if left > 0 {
                    SetTimer(Some(h), TIMER_REVEAL, if left == 2 { 330 } else { 550 }, None);
                }
            }
            WM_TIMER if wp.0 == TIMER_STATUS => {
                update_status();
                check_dirty();
            }
            WM_SETCURSOR if wp.0 == h.0 as usize && (lp.0 & 0xffff) as u32 == HTCLIENT => {
                let Some(d) = app(|a| a.drag).or_else(|| cursor_for(h)) else {
                    return DefWindowProcW(h, msg, wp, lp);
                };
                let c = match d {
                    Hit::Side | Hit::Col(_) => IDC_SIZEWE,
                    Hit::Row(..) => IDC_SIZENS,
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
                        Hit::Side => a.s.side = x - g.pad - g.gap / 2,
                        Hit::Col(k) => drag_ratio(&mut a.s.colr, k, x - g.gap / 2, g.gx, g.w - g.pad - g.gx, g.gap, min),
                        Hit::Row(c, k) => drag_ratio(&mut a.s.rowr[c], k, y - g.gap / 2, g.top, g.bot - g.top, g.gap, min),
                    }
                    a.s.side = geo(a).side;
                    Some(())
                });
                if moved.is_some() {
                    show_bars(); // panes stay put until release: relaying out 16 Explorer views per mouse move is ~120ms
                }
            }
            WM_LBUTTONUP => {
                _ = ReleaseCapture();
                end_drag();
            }
            WM_CAPTURECHANGED => end_drag(),
            WM_LBUTTONDBLCLK => {
                let (x, y) = xy(lp);
                let d = app(|a| hit(&geo(a), x, y));
                app(|a| match d {
                    Some(Hit::Side) => a.s.side = 240 * a.dpi / 96,
                    Some(Hit::Col(_)) => a.s.colr = even(a.s.heights.len()),
                    Some(Hit::Row(c, _)) => a.s.rowr[c] = even(a.s.heights[c]),
                    None => {}
                });
                relayout();
                redraw_all();
            }
            WM_COMMAND => match wp.0 & 0xffff {
                id if id == ID_TB_LAYOUT as usize => open_grid(),
                id if id == ID_TB_SAVE as usize => save_current(),
                id if id == ID_TB_LOAD as usize => open_load(),
                id if id == ID_TB_FAV as usize => open_favs(),
                id if id == ID_TB_THEME as usize => {
                    app(|a| a.s.dark ^= true);
                    apply_theme();
                }
                id if id == ID_TB_VIEW as usize => popup_menu(app(|a| a.view_menu), ID_TB_VIEW),
                ID_HIDDEN => toggle_hidden(),
                ID_DBLUP => app(|a| a.s.dblup ^= true),
                ID_LANG => set_language(!KO.load(Ordering::Relaxed)),
                ID_HELP => open_help(),
                ID_ABOUT => about(h),
                id @ ID_ACCENT..=1405 => set_accent(pal().accents[id - ID_ACCENT].1),
                ID_ACCENT_PICK => pick_accent(),
                ID_KEYS => _ = MessageBoxW(Some(h), &HSTRING::from(tr(KEYS_EN, KEYS_KO)), &HSTRING::from(tr("Keyboard shortcuts", "단축키")), MB_OK),
                _ => {}
            },
            WM_INITMENUPOPUP => {
                let (menu, dblup, accent) = app(|a| (a.view_menu, a.s.dblup, a.s.accent_now()));
                let check = |id: usize, on: bool| CheckMenuItem(menu, id as u32, (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0);
                check(ID_HIDDEN, hidden());
                check(ID_DBLUP, dblup);
                check(ID_LANG, KO.load(Ordering::Relaxed));
                for (k, (_, c)) in pal().accents.iter().enumerate() {
                    check(ID_ACCENT + k, *c == accent);
                }
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
            WM_TIMER if wp.0 == TIMER_SETTLE => settle_cols(h),
            WM_TIMER if wp.0 == TIMER_NEWFILE => {
                let job = PENDING_FILE.lock().ok().and_then(|mut p| {
                    let cur = p.as_mut()?;
                    if pane_of(h) != Some(cur.pane) {
                        return None;
                    }
                    cur.left = cur.left.saturating_sub(1);
                    let (pane, name, left) = (cur.pane, cur.name.clone(), cur.left);
                    if left == 0 {
                        *p = None;
                    }
                    Some((pane, name, left))
                });
                if let Some((pane, name, left)) = job {
                    if select_new_file(pane, &name) || left == 0 {
                        if let Ok(mut p) = PENDING_FILE.lock() {
                            *p = None;
                        }
                        _ = KillTimer(Some(h), TIMER_NEWFILE);
                    }
                } else {
                    _ = KillTimer(Some(h), TIMER_NEWFILE);
                }
            }
            WM_SIZE => layout_pane(h),
            WM_PAINT => paint_pane(h),
            WM_ERASEBKGND => return LRESULT(1), // paint_pane covers everything
            WM_DRAWITEM => {
                draw_button(&*(lp.0 as *const DRAWITEMSTRUCT));
                return LRESULT(1);
            }
            // borderless edits sit on the field colour
            WM_CTLCOLOREDIT => {
                let dc = HDC(wp.0 as _);
                SetBkColor(dc, cref(pal().pill));
                SetTextColor(dc, cref(pal().text));
                SetDCBrushColor(dc, cref(pal().pill));
                return LRESULT(GetStockObject(DC_BRUSH).0 as isize);
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
                (ID_FAV, _) => {
                    if let Some(i) = pane_of(h) {
                        toggle_fav(i);
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

// The README on GitHub, in the UI language.
unsafe fn open_help() {
    let url = format!("{HOME}/blob/main/{}", tr("README.md", "README.ko.md"));
    ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
}

unsafe fn about(owner: HWND) {
    let text = format!(
        "Grids {}\n{}\n\n{}\n{HOME}\n\n{}",
        env!("CARGO_PKG_VERSION"),
        tr("Fast multi-pane file manager for Windows", "Windows용 빠른 다중 패널 파일 관리자"),
        tr("MIT License · © 2026 efww", "MIT 라이선스 · © 2026 efww"),
        tr("Open the project page?", "프로젝트 페이지를 열까요?"),
    );
    if MessageBoxW(Some(owner), &HSTRING::from(text), &HSTRING::from(tr("About Grids", "Grids 정보")), MB_YESNO | MB_ICONINFORMATION) == IDYES {
        ShellExecuteW(None, w!("open"), &HSTRING::from(HOME), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

// Toolbar buttons: id, accessible name, tooltip.
fn toolbar_tips() -> [(i32, &'static str, &'static str); 6] {
    [
        (ID_TB_LAYOUT, "", tr("Layout: click = even grid, drag = rows per column", "레이아웃: 클릭 = 균등, 끌기 = 열마다 칸 수")),
        (ID_TB_SAVE, tr("Save session", "세션 저장"), ""), // tooltip set by refresh_save_button
        (ID_TB_LOAD, tr("Load", "불러오기"), tr("Load a session", "세션 불러오기")),
        (ID_TB_VIEW, tr("View", "보기"), tr("View: hidden files · colours · language · help", "보기: 숨김 파일 · 선택 색 · 언어 · 도움말")),
        (ID_TB_FAV, tr("Favourites", "즐겨찾기"), tr("Favourite folders (open in the active pane)", "즐겨찾기 폴더 (선택한 패널에서 열기)")),
        (ID_TB_THEME, tr("Theme", "테마"), tr("Light / dark theme", "라이트 / 다크 테마")),
    ]
}

// Switches the UI language now: menu, toolbar tooltips, and the panes (rebuilt with their tooltips).
unsafe fn set_language(ko: bool) {
    KO.store(ko, Ordering::Relaxed);
    let (main, old) = app(|a| {
        a.s.ko = ko;
        (a.main, a.view_menu)
    });
    if let Ok(m) = make_menus() {
        app(|a| a.view_menu = m);
        _ = DestroyMenu(old);
    }
    for (id, _, hint) in toolbar_tips() {
        if let Ok(b) = GetDlgItem(Some(main), id) {
            tip_text(b, hint, TTM_UPDATETIPTEXTW);
        }
    }
    refresh_save_button();
    app(|a| a.status_key.clear());
    apply_theme();
}

// The view popup opened from the top toolbar (no classic menu bar).
unsafe fn make_menus() -> Result<HMENU> {
    let view = CreatePopupMenu()?;
    let item = |menu: HMENU, flags: MENU_ITEM_FLAGS, id: usize, text: &str| AppendMenuW(menu, flags, id, &HSTRING::from(text));
    item(view, MF_STRING, ID_HIDDEN, tr("Show hidden files\tCtrl+H", "숨김 파일 표시\tCtrl+H"))?;
    item(view, MF_STRING, ID_DBLUP, tr("Double-click empty space: parent folder", "빈 곳 더블클릭: 위 폴더로"))?;
    let colors = CreatePopupMenu()?;
    for (k, (name, _)) in LIGHT.accents.iter().enumerate() {
        item(colors, MF_STRING, ID_ACCENT + k, tr(ACCENT_EN[k], name))?;
    }
    AppendMenuW(colors, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(colors, MF_STRING, ID_ACCENT_PICK, tr("Custom...", "직접 선택..."))?;
    item(view, MF_POPUP, colors.0 as usize, tr("Selection colour", "선택 색"))?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_LANG, "한국어 (Korean)")?;
    AppendMenuW(view, MF_SEPARATOR, 0, PCWSTR::null())?;
    item(view, MF_STRING, ID_KEYS, tr("Keyboard shortcuts...", "단축키..."))?;
    item(view, MF_STRING, ID_HELP, tr("Help (online)\tF1", "도움말 (온라인)\tF1"))?;
    item(view, MF_STRING, ID_ABOUT, tr("About Grids", "Grids 정보"))?;
    Ok(view)
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
        // qdir.ini: settings from before the rename, read once until grids.ini exists
        let ini = std::fs::read_to_string(ini_path()).or_else(|_| std::fs::read_to_string(ini_path().with_file_name("qdir.ini")));
        let mut s = ini.map(|t| Session::parse(&t)).unwrap_or_default();
        match session_arg(&args).and_then(|f| std::fs::read_to_string(f).ok()) {
            Some(t) => s.take_layout(&Session::parse(&t)),
            None => apply_args(&mut s, args),
        }

        _ = BufferedPaintInit();

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            hCursor: arrow,
            hIcon: LoadIconW(Some(hinst), PCWSTR(1 as _)).unwrap_or_default(), // app.ico, group id 1
            lpszClassName: w!("GridsMain"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW, // the plate is drawn to the window size
            lpfnWndProc: Some(pane_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPane"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(bar_proc),
            hInstance: hinst,
            lpszClassName: w!("GridsBar"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(fav_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsFav"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(load_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsLoad"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(grid_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("GridsPicker"),
            ..Default::default()
        });
        KO.store(s.ko, Ordering::Relaxed);
        let view_menu = make_menus()?;
        let main = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("GridsMain"),
            w!("Grids"),
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
        let caption = cref(pal().bg);
        _ = DwmSetWindowAttribute(main, DWMWA_CAPTION_COLOR, &caption as *const _ as _, 4);
        let square = DWMWCP_DONOTROUND;
        _ = DwmSetWindowAttribute(main, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as _, 4);
        let dpi = GetDpiForWindow(main) as i32;
        if s.side <= 0 {
            s.side = 240 * dpi / 96;
        }
        let win = s.win;
        let tip_style = WS_POPUP | WINDOW_STYLE(0x01 | 0x02); // TTS_ALWAYSTIP | TTS_NOPREFIX
        let tip = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, None, tip_style, 0, 0, 0, 0, Some(main), None, None, None)?;
        app(|a| (a.s, a.main, a.dpi, a.view_menu, a.tip) = (s, main, dpi, view_menu, tip));
        let bar_ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        let mut bars = [HWND::default(); 2];
        for b in &mut bars {
            *b = CreateWindowExW(bar_ex, w!("GridsBar"), w!(""), WS_POPUP, 0, 0, 0, 0, Some(main), None, None, None)?;
            _ = SetLayeredWindowAttributes(*b, COLORREF(0), 170, LWA_ALPHA);
        }
        app(|a| a.bars = bars);
        for (id, label, hint) in toolbar_tips() {
            add_tip(child(main, w!("BUTTON"), PCWSTR(HSTRING::from(label).as_ptr()), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), id, HFONT::default()), hint);
        }
        if let Ok((t, th)) = create_tree(main) {
            app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
        }
        apply_theme();
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
        let file = app(|a| a.s.file.clone());
        let base = std::fs::read_to_string(&file).ok().map(|t| Session::parse(&t));
        app(|a| a.base = Some(base.unwrap_or_else(|| a.s.clone())));
        refresh_save_button();
        SetTimer(Some(main), TIMER_STATUS, 300, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if !pre_translate(&msg) {
                let up = up_candidate(&msg);
                _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                let empty = |i| view_of::<IFolderView2>(i).and_then(|fv| fv.ItemCount(SVGIO_SELECTION).ok()) == Some(0);
                if let Some(i) = up.filter(|&i| empty(i)) {
                    browse(i, SBSP_PARENT);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns() {
        let k = |pid| PROPERTYKEY { fmtid: GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC), pid };
        let text = cols_text(&[(k(10), 220), (k(12), 80)], &[(k(12), -1)]);
        assert_eq!(parse_cols(&text), (vec![(k(10), 220), (k(12), 80)], vec![(k(12), -1)]));
        assert_eq!(parse_cols("junk;"), (vec![], vec![]));
        // kept in the session; a change counts wherever the pane is (columns are a pane setting)
        let mut base = Session { paths: vec![r"C:\a".into(); MAXP], ..Default::default() };
        base.cols[0] = text.clone();
        let base = Session::parse(&base.to_ini());
        assert_eq!(base.cols[0], text);
        let mut cur = base.clone();
        cur.cols[0] = cols_text(&[(k(12), 80), (k(10), 220)], &[(k(12), -1)]);
        assert!(layout_differs(&cur, &base));
        cur.paths[0] = r"C:\b".into();
        assert!(layout_differs(&cur, &base));
        assert!(is_fs(r"C:\") && is_fs(r"\\srv\share") && !is_fs("::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
    }

    #[test]
    fn session_layout_args() {
        let mut s = Session { side: 200, active: 3, ..Default::default() };
        s.set_heights(vec![1, 3]);
        (s.colr, s.rowr[1]) = (vec![0.25, 0.75], vec![0.5, 0.25, 0.25]);
        s.win = Some([1, 2, 300, 400, 1]);
        s.paths[1] = r"C:\a=b\폴더".into();
        s.views[1] = (4, 16);
        assert_eq!(Session::parse(&s.to_ini()), s);
        // garbage in the ini falls back to sane values
        let bad = Session::parse("cols=9,0,2\ncolr=1,-1\nrowr=x|1|1,1\nactive=99\nv3=x\np99=C:\\");
        assert_eq!((bad.heights.clone(), bad.colr.clone(), bad.active), (vec![4, 1, 2], even(3), 0));
        assert_eq!(bad.rowr, vec![even(4), vec![1.0], even(2)]);
        // an older even-grid ini still loads: rows x cols, one row-ratio group for every column
        let old = Session::parse("grid=3,2\nrowr=0.5,0.25,0.25");
        assert_eq!((old.heights.clone(), old.rowr[1].clone()), (vec![3, 3], vec![0.5, 0.25, 0.25]));
        // a named session brings grid + folders but not the window
        let mut cur = Session { side: 300, win: Some([0, 0, 9, 9, 0]), ..Default::default() };
        cur.take_layout(&s);
        assert_eq!((cur.heights.clone(), cur.paths[1].as_str(), cur.side, cur.win), (vec![1, 3], s.paths[1].as_str(), 300, Some([0, 0, 9, 9, 0])));

        // panes are numbered down each column; the picker drag sets one column at a time
        assert_eq!([slot(&[1, 3], 0), slot(&[1, 3], 1), slot(&[1, 3], 3)], [(0, 0), (1, 0), (1, 2)]);
        assert_eq!(drag_heights(drag_heights(vec![1], 0, 1), 2, 1), vec![1, 3]); // left 1, then right column down to row 3
        assert_eq!(drag_heights(vec![2, 2, 2], 1, 0), vec![2]); // moving back left drops columns
        assert_eq!((layout_label(&[2, 2]), layout_label(&[1, 3])), ("2 × 2".into(), "1 | 3".into()));

        // green dot: layout, ratios and view modes count; folders and unknown (0) baseline views don't
        let base = s.clone();
        let mut cur = s.clone();
        (cur.paths[0], cur.views[2]) = ("D:\\".into(), (1, 48));
        assert!(!layout_differs(&cur, &base));
        cur.colr = vec![0.3, 0.7];
        assert!(layout_differs(&cur, &base));
        assert!(!s.snapshot().contains("win=") && !s.snapshot().contains("session="));
        // the drawn star: solid middle and empty corners when filled; hollow middle as an outline
        let (full, ring) = (star_coverage(15, Star::Filled), star_coverage(15, Star::Outline));
        assert_eq!((full[7 * 15 + 7], full[0], full[14 * 15 + 14]), (255, 0, 0));
        assert!(ring[8 * 15 + 7] == 0 && ring.iter().any(|&a| a == 255));
        for n in [15, 22] {
            // the + (what Plus adds to the outline) sits exactly on the box centre in both directions
            let (plus, ring) = (star_coverage(n, Star::Plus), star_coverage(n, Star::Outline));
            let added: Vec<(i32, i32)> = (0..n * n).filter(|&k| plus[k as usize] != ring[k as usize]).map(|k| (k % n, k / n)).collect();
            let span = |f: fn(&(i32, i32)) -> i32| added.iter().map(f).min().unwrap() + added.iter().map(f).max().unwrap();
            assert_eq!((span(|p| p.0), span(|p| p.1)), (n - 1, n - 1), "{n}");
        }
        // favourites: kept in grids.ini, case-insensitive, never in a named session
        let mut f = Session { favs: vec![r"C:\Work".into(), r"D:\".into()], ..Default::default() };
        f = Session::parse(&f.to_ini());
        assert!(f.is_fav(r"c:\work") && !f.is_fav(r"C:\Work\x") && !f.is_fav(""));
        assert!(!f.snapshot().contains("fav="));
        assert_eq!((short_name(r"C:\Users\x\"), short_name("C:\\"), short_name(THIS_PC)), ("x".into(), "C:".into(), "This PC".into()));

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
        assert_eq!((s.heights.clone(), s.paths[4].as_str()), (vec![2, 2, 2], "::{x4}"));
        assert_eq!((session_arg(&["a.GRIDS".into()]), session_arg(&["old.qdr".into()])), (Some("a.GRIDS"), Some("old.qdr")));
        assert_eq!(session_arg(&["a.grids".into(), "b".into()]), None);
        assert_eq!(fresh_file("New Text Document.txt", |_| false), "New Text Document.txt");
        assert_eq!(fresh_file("New Text Document.txt", |n| n == "New Text Document.txt"), "New Text Document (2).txt");
        assert_eq!(fresh_file("새 텍스트 문서.txt", |n| n == "새 텍스트 문서.txt" || n == "새 텍스트 문서 (2).txt"), "새 텍스트 문서 (3).txt");

        assert_eq!(filter_spec(" pdf; *.TXT ;"), "*pdf*;*.TXT");
        assert_eq!((human(10), human(1536), human(5 << 30)), ("10 B".into(), "1.5 KB".into(), "5.0 GB".into()));

        // pane top bar: back, fwd, up, address, ⇄, filter left to right, nothing overlapping, inside the plate
        for (w, dpi) in [(800, 96), (260, 96), (1200, 144)] {
            let p = pane_geo(w, 500, dpi);
            let row = [p.btn[0], p.btn[1], p.btn[2], p.addr, p.btn[3], p.filt, p.fav];
            assert!(row.windows(2).all(|x| x[0].right <= x[1].left), "{w}@{dpi}");
            assert!(p.fav.right <= w && p.view.top >= p.btn[0].bottom);
        }
    }
}
