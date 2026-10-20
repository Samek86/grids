// Q-Dir style multi-pane file manager: shell tree on the left, 1x1..4x4 grid of Explorer views on the right.
// Each pane hosts the real Explorer control (IExplorerBrowser), so context menus, drag & drop, rename,
// thumbnails, columns etc. come from Windows itself.
#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
use windows::Win32::System::Com::*;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::SFGAO_FOLDER;
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
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
// menu ids
const ID_LAYOUT: usize = 1000; // + (rows-1)*4 + (cols-1)
const ID_HIDDEN: usize = 1100;
const ID_KEYS: usize = 1101;
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

const KEYS_HELP: &str = "Ctrl+Tab / Ctrl+Shift+Tab\t다음 / 이전 패널\n\
Alt+← / Alt+→\t뒤로 / 앞으로\n\
Alt+↑\t상위 폴더\n\
Ctrl+L, Alt+D, F4\t주소창\n\
Ctrl+F\t필터 (*.pdf;*.txt 또는 이름 일부, 폴더는 항상 표시)\n\
Enter / Esc\t주소창·필터 적용 / 취소\n\
Ctrl+H\t숨김 파일 표시\n\
F2, Del, Ctrl+C/X/V, F5\t탐색기와 동일\n\n\
⇄ 버튼: 선택한 폴더(없으면 현재 폴더)를 다른 패널에서 열기\n\
세션 메뉴: 레이아웃+폴더를 .qdr로 저장 / 불러오기\n\
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

struct Geo {
    h: i32,      // content height (status bar below it)
    bottom: i32, // client bottom
    side: i32,
    gap: i32,
    gx: i32,
    w: i32,
    xs: Vec<(i32, i32)>,
    ys: Vec<(i32, i32)>,
}

fn geo(a: &App) -> Geo {
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(a.main, &mut rc) };
    let s = |v: i32| v * a.dpi / 96;
    let gap = s(5);
    let h = (rc.bottom - s(22)).max(0);
    let side = a.s.side.clamp(s(80), (rc.right - s(200)).max(s(80)));
    let gx = side + gap;
    Geo {
        xs: cells(gx, rc.right - gx, &a.s.colr, gap),
        ys: cells(0, h, &a.s.rowr, gap),
        h,
        bottom: rc.bottom,
        w: rc.right,
        side,
        gap,
        gx,
    }
}

fn status_rect(g: &Geo) -> RECT {
    RECT { left: 0, top: g.h, right: g.w, bottom: g.bottom }
}

fn hit(g: &Geo, x: i32, y: i32) -> Option<Hit> {
    if y >= g.h {
        return None;
    }
    if x >= g.side && x < g.gx {
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
    dpi: i32,
    panes: Vec<Option<Pane>>,
    drag: Option<Hit>,
    sizing: bool, // inside the window's own move/size loop
    status: String,
    status_key: String,
    sess_menu: HMENU,
    sess_files: Vec<PathBuf>,
}

thread_local! {
    static APP: RefCell<App> = RefCell::new(App {
        s: Session::default(),
        main: HWND::default(),
        tree: None,
        tree_hwnd: HWND::default(),
        tree_input: false,
        font: HFONT::default(),
        dpi: 96,
        panes: (0..MAXP).map(|_| None).collect(),
        drag: None,
        sizing: false,
        status: String::new(),
        status_key: String::new(),
        sess_menu: HMENU::default(),
        sess_files: Vec::new(),
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

unsafe fn make_font(dpi: i32) -> HFONT {
    let mut lf = LOGFONTW { lfHeight: -12 * dpi / 96, ..Default::default() };
    for (d, c) in lf.lfFaceName.iter_mut().zip("Segoe UI".encode_utf16()) {
        *d = c;
    }
    CreateFontIndirectW(&lf)
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
    let (btn, plain, sunk) = (BS_PUSHBUTTON as u32, WINDOW_EX_STYLE(0), WS_EX_CLIENTEDGE);
    child(h, w!("BUTTON"), w!("←"), btn, plain, ID_BACK, font);
    child(h, w!("BUTTON"), w!("→"), btn, plain, ID_FWD, font);
    child(h, w!("BUTTON"), w!("↑"), btn, plain, ID_UP, font);
    child(h, w!("BUTTON"), w!("⇄"), btn, plain, ID_OPEN_IN, font);
    child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, sunk, ID_EDIT, font);
    let f = child(h, w!("EDIT"), w!(""), ES_AUTOHSCROLL as u32, sunk, ID_FILTER, font);
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
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let s = |v: i32| v * dpi / 96;
    let (b, bar, bw) = (s(2), s(24), s(28));
    let rect = |x: i32, w: i32| RECT { left: x, top: b, right: x + w.max(0), bottom: b + bar };
    let x = b + 4 * bw + s(2);
    let fw = s(150).min((rc.right - x) / 3);
    let mut moves = Vec::new();
    for (k, id) in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN].into_iter().enumerate() {
        moves.push((GetDlgItem(Some(h), id).unwrap_or_default(), rect(b + k as i32 * bw, bw)));
    }
    moves.push((GetDlgItem(Some(h), ID_EDIT).unwrap_or_default(), rect(x, rc.right - b - x - fw - s(2))));
    moves.push((GetDlgItem(Some(h), ID_FILTER).unwrap_or_default(), rect(rc.right - b - fw, fw)));
    let view = RECT { left: b, top: b + bar + b, right: rc.right - b, bottom: rc.bottom - b };
    move_all(&moves, Some((&eb, view)));
}

unsafe fn relayout() {
    sync_panes();
    let Some((main, moves)) = app(|a| {
        if a.main.0.is_null() {
            return None;
        }
        let g = geo(a);
        let mut moves = vec![(a.tree_hwnd, RECT { left: 0, top: 0, right: g.side, bottom: g.h })];
        for (i, p) in a.panes.iter().enumerate().take(a.s.rows * a.s.cols) {
            if let Some(p) = p {
                let ((x0, x1), (y0, y1)) = (g.xs[i % a.s.cols], g.ys[i / a.s.cols]);
                moves.push((p.hwnd, RECT { left: x0, top: y0, right: x1, bottom: y1 }));
            }
        }
        Some((a.main, moves))
    }) else {
        return;
    };
    move_all(&moves, None);
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
    let (old, hs) = app(|a| {
        let old = std::mem::replace(&mut a.s.active, i);
        (old, [old, i].map(|k| a.panes[k].as_ref().map(|p| p.hwnd)))
    });
    if old != i {
        for h in hs.into_iter().flatten() {
            _ = InvalidateRect(Some(h), None, true);
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

unsafe fn paint_status(h: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(h, &mut ps);
    let (text, rc, font, pad) = app(|a| (a.status.clone(), status_rect(&geo(a)), a.font, 8 * a.dpi / 96));
    FillRect(hdc, &rc, GetSysColorBrush(COLOR_BTNFACE));
    let old = SelectObject(hdc, font.into());
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, COLORREF(GetSysColor(COLOR_BTNTEXT)));
    let mut r = RECT { left: rc.left + pad, ..rc };
    let mut w: Vec<u16> = text.encode_utf16().collect();
    if !w.is_empty() {
        // an empty Vec's dangling pointer crashes DrawTextW even with length 0
        DrawTextW(hdc, &mut w, &mut r, DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX);
    }
    SelectObject(hdc, old);
    _ = EndPaint(h, &ps);
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
            WM_PAINT => paint_status(h),
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
                id @ ID_LAYOUT..=1015 => set_layout((id - ID_LAYOUT) / 4 + 1, (id - ID_LAYOUT) % 4 + 1),
                ID_HIDDEN => toggle_hidden(),
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
                let menu = GetMenu(h);
                let cur = app(|a| (a.s.rows - 1) * 4 + a.s.cols - 1);
                let check = |on: bool| (MF_BYCOMMAND | if on { MF_CHECKED } else { MF_UNCHECKED }).0;
                for k in 0..16 {
                    CheckMenuItem(menu, (ID_LAYOUT + k) as u32, check(k == cur));
                }
                CheckMenuItem(menu, ID_HIDDEN as u32, check(hidden()));
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
                let (old, panes, font) = app(|a| {
                    a.s.side = a.s.side * dpi / a.dpi;
                    a.dpi = dpi;
                    let old = std::mem::replace(&mut a.font, make_font(dpi));
                    (old, a.panes.iter().flatten().map(|p| p.hwnd).collect::<Vec<_>>(), a.font)
                });
                for p in panes {
                    for id in [ID_BACK, ID_FWD, ID_UP, ID_OPEN_IN, ID_EDIT, ID_FILTER] {
                        if let Ok(c) = GetDlgItem(Some(p), id) {
                            SendMessageW(c, WM_SETFONT, Some(WPARAM(font.0 as usize)), Some(LPARAM(1)));
                        }
                    }
                }
                _ = DeleteObject(old.into());
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
            WM_ERASEBKGND => {
                let act = app(|a| a.panes[a.s.active].as_ref().is_some_and(|p| p.hwnd == h));
                let mut rc = RECT::default();
                _ = GetClientRect(h, &mut rc);
                FillRect(HDC(wp.0 as _), &rc, GetSysColorBrush(if act { COLOR_HIGHLIGHT } else { COLOR_BTNFACE }));
                return LRESULT(1);
            }
            WM_COMMAND => match ((wp.0 & 0xffff) as i32, ((wp.0 >> 16) & 0xffff) as u32) {
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

unsafe fn make_menu() -> Result<(HMENU, HMENU)> {
    let bar = CreateMenu()?;
    let lay = CreatePopupMenu()?;
    // 4 menu columns (MF_MENUBARBREAK) so the items form a rows x cols grid.
    for c in 1..=4 {
        for r in 1..=4 {
            let brk = if r == 1 && c > 1 { MF_MENUBARBREAK } else { MENU_ITEM_FLAGS(0) };
            let id = ID_LAYOUT + (r - 1) * 4 + (c - 1);
            AppendMenuW(lay, MF_STRING | brk, id, &HSTRING::from(format!("{r} × {c}")))?;
        }
    }
    let sess = CreatePopupMenu()?;
    AppendMenuW(sess, MF_STRING, ID_SESS_SAVE, w!("현재 세션 저장..."))?;
    AppendMenuW(sess, MF_STRING, ID_SESS_DIR, w!("세션 폴더 열기"))?;
    AppendMenuW(sess, MF_SEPARATOR, 0, PCWSTR::null())?; // saved sessions are listed below on open
    let view = CreatePopupMenu()?;
    AppendMenuW(view, MF_STRING, ID_HIDDEN, w!("숨김 파일 표시\tCtrl+H"))?;
    AppendMenuW(view, MF_STRING, ID_KEYS, w!("단축키..."))?;
    AppendMenuW(bar, MF_POPUP, lay.0 as usize, w!("레이아웃 (행×열)"))?;
    AppendMenuW(bar, MF_POPUP, sess.0 as usize, w!("세션"))?;
    AppendMenuW(bar, MF_POPUP, view.0 as usize, w!("보기"))?;
    Ok((bar, sess))
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

        let hinst: HINSTANCE = GetModuleHandleW(None)?.into();
        let arrow = LoadCursorW(None, IDC_ARROW)?;
        RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(main_proc),
            hInstance: hinst,
            hCursor: arrow,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: w!("QDirMain"),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW, // repaint the active border on every resize
            lpfnWndProc: Some(pane_proc),
            hInstance: hinst,
            hCursor: arrow,
            lpszClassName: w!("QDirPane"),
            ..Default::default()
        });
        let (menu, sess_menu) = make_menu()?;
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
            Some(menu),
            Some(hinst),
            None,
        )?;
        let dpi = GetDpiForWindow(main) as i32;
        if s.side <= 0 {
            s.side = 240 * dpi / 96;
        }
        let win = s.win;
        app(|a| {
            (a.s, a.main, a.dpi, a.sess_menu) = (s, main, dpi, sess_menu);
            a.font = make_font(dpi);
        });
        if let Ok((t, th)) = create_tree(main) {
            app(|a| (a.tree, a.tree_hwnd) = (Some(t), th));
        }
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
        let mut s = Session { rows: 2, cols: 3, colr: vec![0.25, 0.25, 0.5], side: 200, active: 4, ..Default::default() };
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
    }
}
