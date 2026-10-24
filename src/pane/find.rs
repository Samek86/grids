// Quick search for the pane's filter box. The box still filters the open folder.
// A result list streams from one background walker. A subfolder search asks Everything
// when that setting is on and Everything is running; otherwise it walks. A new query or Esc drops the previous one.

use std::collections::HashMap;
use std::sync::{Condvar, Mutex, Once};
use std::time::{Duration, Instant};

use crate::app::*;
use crate::everything::{self, forces_everywhere, search_text, without_scope_prefix, MAX_RESULTS};
use crate::i18n::*;
use crate::imports::*;
use crate::session::*;
use crate::ui::*;

use super::{apply_filter, focus_view, hidden, navigate};

pub(crate) const FIND_CAP: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindHit {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) dir: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FindEngine {
    Walk,
    Everything,
}

pub(crate) fn find_engine(recursive: bool, enabled: bool, everything_up: bool) -> FindEngine {
    if recursive && enabled && everything_up { FindEngine::Everything } else { FindEngine::Walk }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FindHitOpen {
    Folder(String),
    File { path: String, folder: String },
}

pub(crate) fn hit_open(path: &str, dir: bool) -> FindHitOpen {
    if dir {
        FindHitOpen::Folder(path.to_string())
    } else {
        let folder = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        FindHitOpen::File { path: path.to_string(), folder }
    }
}

pub(crate) fn query_parts(q: &str) -> Vec<String> {
    q.split(';').map(str::trim).filter(|t| !t.is_empty()).map(|t| t.to_string()).collect()
}

pub(crate) fn name_matches(name: &str, query: &str) -> bool {
    let parts = query_parts(query);
    !parts.is_empty() && parts.iter().any(|t| term_match(name, t))
}

fn term_match(name: &str, term: &str) -> bool {
    if term.contains(['*', '?']) { wild_ci(name, term) } else { contains_ci(name, term) }
}

fn lower(s: &str) -> Vec<char> {
    s.chars().flat_map(|c| c.to_lowercase()).collect()
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    let (h, n) = (lower(hay), lower(needle));
    !n.is_empty() && h.windows(n.len()).any(|w| w == n.as_slice())
}

fn wild_ci(name: &str, pat: &str) -> bool {
    wild(&lower(name), &lower(pat))
}

fn wild(n: &[char], p: &[char]) -> bool {
    let (mut i, mut j) = (0, 0);
    let (mut star, mut mark) = (None, 0);
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            star = Some(j);
            j += 1;
            mark = i;
        } else if let Some(s) = star {
            j = s + 1;
            mark += 1;
            i = mark;
        } else {
            return false;
        }
    }
    while j < p.len() && p[j] == '*' {
        j += 1;
    }
    j == p.len()
}

pub(crate) fn hit_label(root: &str, hit: &FindHit) -> String {
    let parent = Path::new(&hit.path).parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    let root = root.trim_end_matches('\\');
    let parent_trim = parent.trim_end_matches('\\');
    if parent_trim.eq_ignore_ascii_case(root) || parent.is_empty() {
        return hit.name.clone();
    }
    let rel = parent_trim.get(root.len()..).unwrap_or(parent_trim).trim_start_matches('\\');
    if rel.is_empty() { hit.name.clone() } else { format!("{}  ·  {rel}", hit.name) }
}

pub(crate) fn clamp_sel(sel: i32, n: usize) -> usize {
    if n == 0 { 0 } else { sel.clamp(0, n as i32 - 1) as usize }
}

/// Keep row `sel` inside the scrolled area under the sticky header. `view_h` is that area's height.
pub(crate) fn scroll_to_sel(sel: usize, scroll: i32, view_h: i32, row: i32) -> i32 {
    if row <= 0 || view_h <= 0 {
        return scroll.max(0);
    }
    let top = sel as i32 * row;
    let bot = top + row;
    let mut s = scroll;
    if top < s {
        s = top;
    }
    if bot > s + view_h {
        s = bot - view_h;
    }
    s.max(0)
}

pub(crate) struct Walked {
    pub(crate) more: bool,
    pub(crate) stopped: bool,
}

pub(crate) fn walk_search(root: &Path, query: &str, recursive: bool, show_hidden: bool, cap: usize, stop: &dyn Fn() -> bool, mut emit: impl FnMut(&[FindHit], bool)) -> Walked {
    let mut hits = Vec::new();
    let mut pending = Vec::new();
    let mut more = false;
    let mut stopped = false;
    let mut stack = vec![root.to_path_buf()];
    let mut last = Instant::now();
    let flush = |pending: &mut Vec<FindHit>, hits: &[FindHit], more: bool, emit: &mut dyn FnMut(&[FindHit], bool)| {
        if !pending.is_empty() {
            emit(pending, more && hits.len() >= cap);
            pending.clear();
        }
    };
    while let Some(dir) = stack.pop() {
        if stop() {
            stopped = true;
            break;
        }
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        let mut kids = Vec::new();
        for e in rd {
            if stop() {
                stopped = true;
                break;
            }
            let Ok(e) = e else { continue };
            let name = e.file_name().to_string_lossy().into_owned();
            if name.is_empty() {
                continue;
            }
            let path = e.path();
            if !show_hidden && is_hidden(&path) {
                continue;
            }
            let dir_ent = is_dir_follow(&e);
            if dir_ent && recursive && !is_reparse(&path) {
                kids.push(path.clone());
            }
            if !name_matches(&name, query) {
                continue;
            }
            if hits.len() >= cap {
                more = true;
                stopped = true;
                break;
            }
            let hit = FindHit { name, path: path.to_string_lossy().into_owned(), dir: dir_ent };
            hits.push(hit.clone());
            pending.push(hit);
            if pending.len() >= 24 || last.elapsed() >= Duration::from_millis(50) {
                flush(&mut pending, &hits, more, &mut emit);
                last = Instant::now();
            }
        }
        if stopped {
            break;
        }
        stack.extend(kids);
    }
    flush(&mut pending, &hits, more, &mut emit);
    Walked { more, stopped }
}

fn is_dir_follow(e: &std::fs::DirEntry) -> bool {
    e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink())
}

fn is_reparse(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const REPARSE: u32 = 0x400;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & REPARSE != 0)
}

fn is_hidden(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const HIDDEN: u32 = 0x2;
    const SYSTEM: u32 = 0x4;
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & (HIDDEN | SYSTEM) != 0)
}

struct FindUi {
    gen: usize,
    query: String,
    root: String,
    recursive: bool,
    hits: Vec<FindHit>,
    more: bool,
    running: bool,
    everywhere: bool,
    engine: FindEngine,
    sel: usize,
    scroll: i32,
    list: isize,
    show: bool,
}

fn hwnd_of(v: isize) -> HWND {
    HWND(v as _)
}

struct Job {
    hwnd: isize,
    gen: usize,
    root: String,
    query: String,
    recursive: bool,
    hidden: bool,
    everything: bool,
    everywhere: bool,
}

struct Batch {
    hwnd: isize,
    gen: usize,
    hits: Vec<FindHit>,
    more: bool,
    done: bool,
    engine: FindEngine,
}

fn find_map() -> std::sync::MutexGuard<'static, HashMap<isize, FindUi>> {
    static FIND: std::sync::OnceLock<Mutex<HashMap<isize, FindUi>>> = std::sync::OnceLock::new();
    FIND.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap_or_else(|e| e.into_inner())
}
static INBOX: Mutex<Vec<Batch>> = Mutex::new(Vec::new());
static JOB: Mutex<Option<Job>> = Mutex::new(None);
static WAKE: Condvar = Condvar::new();

fn live_gen(hwnd: isize) -> usize {
    find_map().get(&hwnd).map(|u| u.gen).unwrap_or(0)
}

fn row_h(dpi: i32) -> i32 {
    (22 * dpi.max(96)) / 96
}

pub(crate) unsafe fn drop_find(hwnd: HWND) {
    if hwnd.0.is_null() {
        return;
    }
    let mut map = find_map();
    if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
        u.gen = u.gen.wrapping_add(1);
        u.running = false;
        u.show = false;
    }
    map.remove(&(hwnd.0 as isize));
}

pub(crate) unsafe fn find_cancel(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let mut map = find_map();
    if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
        u.gen = u.gen.wrapping_add(1);
        u.running = false;
    }
}

pub(crate) unsafe fn find_clear(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let list = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return };
        u.gen = u.gen.wrapping_add(1);
        u.query.clear();
        u.hits.clear();
        u.more = false;
        u.running = false;
        u.show = false;
        u.sel = 0;
        u.scroll = 0;
        hwnd_of(u.list)
    };
    if !list.0.is_null() {
        _ = ShowWindow(list, SW_HIDE);
    }
}

pub(crate) unsafe fn hide_find(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let list = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return };
        u.show = false;
        hwnd_of(u.list)
    };
    if !list.0.is_null() {
        _ = ShowWindow(list, SW_HIDE);
    }
}

pub(crate) unsafe fn focus_is_find(pane: HWND, focus: HWND) -> bool {
    if pane.0.is_null() || focus.0.is_null() {
        return false;
    }
    find_map().get(&(pane.0 as isize)).is_some_and(|u| {
        let list = hwnd_of(u.list);
        !list.0.is_null() && (list == focus || IsChild(list, focus).as_bool())
    })
}

/// The search-result list of this pane has the keyboard focus.
pub(crate) unsafe fn find_focused(i: usize) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    focus_is_find(hwnd, GetFocus())
}

/// Put the keyboard on the result list, when that list is on screen.
pub(crate) unsafe fn focus_find_list(i: usize) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    let list = {
        let map = find_map();
        let Some(u) = map.get(&(hwnd.0 as isize)) else { return false };
        if !u.show {
            return false;
        }
        hwnd_of(u.list)
    };
    if list.0.is_null() || !IsWindow(Some(list)).as_bool() || !IsWindowVisible(list).as_bool() {
        return false;
    }
    _ = SetFocus(Some(list));
    true
}

/// The next or previous result, or nothing at either end. `sel` outside `0..n` has no neighbour.
pub(crate) fn find_neighbour(sel: usize, n: usize, prev: bool) -> Option<usize> {
    if n == 0 || sel >= n {
        return None;
    }
    if prev { sel.checked_sub(1) } else { (sel + 1 < n).then_some(sel + 1) }
}

/// Move the result selection by one row and return that hit. Stays put at either end.
pub(crate) unsafe fn step_find_sel(i: usize, prev: bool) -> Option<FindHit> {
    let hwnd = pane_hwnd(i)?;
    let dpi = app(|a| a.dpi);
    let mut map = find_map();
    let u = map.get_mut(&(hwnd.0 as isize))?;
    if !u.show {
        return None;
    }
    let next = find_neighbour(u.sel, u.hits.len(), prev)?;
    u.sel = next;
    let view_h = list_view_h(hwnd_of(u.list), dpi);
    u.scroll = scroll_to_sel(u.sel, u.scroll, view_h, row_h(dpi));
    let hit = u.hits[u.sel].clone();
    let list = hwnd_of(u.list);
    drop(map);
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
    Some(hit)
}

/// Quick look at the selected result. Nothing is selected: the same beep as an empty file list.
pub(crate) unsafe fn preview_find_sel(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let hit = {
        let map = find_map();
        let Some(u) = map.get(&(hwnd.0 as isize)) else { return };
        if !u.show { None } else { u.hits.get(u.sel).cloned() }
    };
    let Some(hit) = hit else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    super::open_peek_path(i, &hit.path, &hit.name);
}

pub(crate) unsafe fn find_retarget(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let focused = pane_ctl(i, ID_FILTER).is_some_and(|e| GetFocus() == e);
    let showing = find_map().get(&(hwnd.0 as isize)).is_some_and(|u| u.show);
    if focused || showing {
        refresh_find(i);
    }
}

pub(crate) unsafe fn refresh_find(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    let (text, root) = (pane_ctl(i, ID_FILTER).map(|e| text_of(e)).unwrap_or_default(), app(|a| a.s.paths.get(i).cloned().unwrap_or_default()));
    let query = text.trim().to_string();
    if query_parts(&query).is_empty() || !is_fs(&root) {
        find_clear(i);
        return;
    }
    let show_hidden = hidden();
    let resume = {
        let mut map = find_map();
        let u = map.entry(hwnd.0 as isize).or_insert_with(|| FindUi {
            gen: 0,
            query: String::new(),
            root: String::new(),
            recursive: false,
            hits: Vec::new(),
            more: false,
            running: false,
            everywhere: false,
            engine: FindEngine::Walk,
            sel: 0,
            scroll: 0,
            list: 0,
            show: false,
        });
        if u.query == query && u.root.eq_ignore_ascii_case(&root) && (u.running || !u.hits.is_empty()) {
            u.show = true;
            true
        } else {
            false
        }
    };
    if resume {
        show_list(hwnd);
        return;
    }
    let enabled = app(|a| a.s.use_everything);
    if enabled && find_map().get(&(hwnd.0 as isize)).is_some_and(|u| u.recursive) {
        ask_install_everything_once(hwnd);
    }
    let (gen, recursive, everywhere, use_ev) = {
        let mut map = find_map();
        let u = map.get_mut(&(hwnd.0 as isize)).unwrap();
        u.gen = u.gen.wrapping_add(1);
        u.query = query.clone();
        u.root = root.clone();
        u.hits.clear();
        u.more = false;
        u.running = true;
        u.sel = 0;
        u.scroll = 0;
        u.show = true;
        let up = u.recursive && enabled && everything::running();
        let use_ev = find_engine(u.recursive, enabled, up) == FindEngine::Everything;
        u.engine = if use_ev { FindEngine::Everything } else { FindEngine::Walk };
        (u.gen, u.recursive, u.everywhere, use_ev)
    };
    // Any other pane's walk is stale once this one starts.
    {
        let mut map = find_map();
        for (h, u) in map.iter_mut() {
            if *h != hwnd.0 as isize && u.running {
                u.gen = u.gen.wrapping_add(1);
                u.running = false;
            }
        }
    }
    show_list(hwnd);
    let job = Job { hwnd: hwnd.0 as isize, gen, root, query, recursive, hidden: show_hidden, everything: use_ev, everywhere };
    {
        let mut slot = JOB.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(job);
    }
    static WORKER: Once = Once::new();
    WORKER.call_once(|| _ = std::thread::Builder::new().name("grids-find".into()).spawn(find_worker));
    WAKE.notify_one();
}

pub(crate) unsafe fn toggle_find_scope(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    if let Some(e) = pane_ctl(i, ID_FILTER) {
        let text = text_of(e);
        if let Some(rest) = without_scope_prefix(&text) {
            {
                let mut map = find_map();
                if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
                    u.everywhere = false;
                }
            }
            _ = SetWindowTextW(e, &HSTRING::from(rest));
            return;
        }
    }
    {
        let mut map = find_map();
        if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
            u.everywhere = !u.everywhere;
            u.query.clear();
        }
    }
    refresh_find(i);
}

pub(crate) unsafe fn toggle_find_sub(i: usize) {
    let Some(hwnd) = pane_hwnd(i) else { return };
    {
        let mut map = find_map();
        if let Some(u) = map.get_mut(&(hwnd.0 as isize)) {
            u.recursive = !u.recursive;
            u.query.clear(); // force refresh_find to see a change
        }
    }
    refresh_find(i);
}

pub(crate) unsafe fn nudge_find(i: usize, delta: i32) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    let dpi = app(|a| a.dpi);
    let mut map = find_map();
    let Some(u) = map.get_mut(&(hwnd.0 as isize)) else { return false };
    if !u.show {
        return false;
    }
    u.sel = clamp_sel(u.sel as i32 + delta, u.hits.len());
    let view_h = list_view_h(hwnd_of(u.list), dpi);
    u.scroll = scroll_to_sel(u.sel, u.scroll, view_h, row_h(dpi));
    let list = hwnd_of(u.list);
    drop(map);
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
    true
}

pub(crate) unsafe fn open_find_sel(i: usize) -> bool {
    let Some(hwnd) = pane_hwnd(i) else { return false };
    let hit = {
        let map = find_map();
        let Some(u) = map.get(&(hwnd.0 as isize)) else { return false };
        if !u.show || u.hits.is_empty() {
            return false;
        }
        u.hits.get(u.sel).cloned()
    };
    let Some(hit) = hit else { return false };
    activate_hit(i, &hit);
    true
}

pub(crate) unsafe fn place_find(pane: HWND) {
    if pane.0.is_null() {
        return;
    }
    let (list, show) = {
        let map = find_map();
        match map.get(&(pane.0 as isize)) {
            Some(u) if u.list != 0 => (hwnd_of(u.list), u.show),
            _ => return,
        }
    };
    if !show {
        _ = ShowWindow(list, SW_HIDE);
        return;
    }
    let mut rc = RECT::default();
    _ = GetClientRect(pane, &mut rc);
    let view = pane_geo_of(pane, rc.right, rc.bottom, app(|a| a.dpi)).view;
    let (w, h) = ((view.right - view.left).max(0), (view.bottom - view.top).max(0));
    _ = SetWindowPos(list, Some(HWND_TOP), view.left, view.top, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
}

pub(crate) unsafe fn on_find_msg(pane: HWND) {
    let key = pane.0 as isize;
    let batches: Vec<Batch> = {
        let mut boxq = INBOX.lock().unwrap_or_else(|e| e.into_inner());
        let mut keep = Vec::new();
        let mut mine = Vec::new();
        for b in boxq.drain(..) {
            if b.hwnd == key { mine.push(b) } else { keep.push(b) }
        }
        *boxq = keep;
        mine
    };
    if batches.is_empty() {
        return;
    }
    let list = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&key) else { return };
        for b in batches {
            if b.gen != u.gen {
                continue;
            }
            u.engine = b.engine;
            let cap = if u.engine == FindEngine::Everything { MAX_RESULTS as usize } else { FIND_CAP };
            if !b.hits.is_empty() && u.hits.len() < cap {
                let room = cap - u.hits.len();
                u.hits.extend(b.hits.into_iter().take(room));
            }
            u.more = b.more || u.hits.len() >= cap;
            if b.done {
                u.running = false;
            }
            u.sel = clamp_sel(u.sel as i32, u.hits.len());
        }
        hwnd_of(u.list)
    };
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
}

fn pane_hwnd(i: usize) -> Option<HWND> {
    app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)).filter(|h| !h.0.is_null())
}

fn list_view_h(list: HWND, dpi: i32) -> i32 {
    if list.0.is_null() {
        return row_h(dpi) * 8;
    }
    let mut rc = RECT::default();
    unsafe { _ = GetClientRect(list, &mut rc) };
    (rc.bottom - row_h(dpi)).max(row_h(dpi))
}

unsafe fn show_list(pane: HWND) {
    let list = ensure_list(pane);
    {
        let mut map = find_map();
        if let Some(u) = map.get_mut(&(pane.0 as isize)) {
            u.list = list.0 as isize;
            u.show = true;
        }
    }
    place_find(pane);
}

unsafe fn ensure_list(pane: HWND) -> HWND {
    if let Some(h) = find_map().get(&(pane.0 as isize)).map(|u| hwnd_of(u.list)) {
        if !h.0.is_null() && IsWindow(Some(h)).as_bool() {
            return h;
        }
    }
    static CLASS: Once = Once::new();
    CLASS.call_once(|| {
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        _ = RegisterClassW(&WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(find_proc),
            hInstance: hinst.into(),
            hCursor: arrow,
            lpszClassName: w!("GridsFind"),
            ..Default::default()
        });
    });
    CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsFind"), w!(""), WS_CHILD | WS_CLIPSIBLINGS, 0, 0, 0, 0, Some(pane), None, None, None).unwrap_or_default()
}

unsafe fn activate_hit(i: usize, hit: &FindHit) {
    let open = hit_open(&hit.path, hit.dir);
    if let Some(e) = pane_ctl(i, ID_FILTER) {
        _ = SetWindowTextW(e, w!(""));
    }
    find_clear(i);
    apply_filter(i);
    match open {
        FindHitOpen::Folder(p) => {
            navigate(i, &p);
        }
        FindHitOpen::File { path, folder } => {
            let cur = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
            ShellExecuteW(None, w!("open"), &HSTRING::from(&path), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
            if !folder.is_empty() && !same_dir(&folder, &cur) {
                navigate(i, &folder);
            }
        }
    }
    focus_view(i);
}

fn same_dir(a: &str, b: &str) -> bool {
    a.trim_end_matches('\\').eq_ignore_ascii_case(b.trim_end_matches('\\'))
}

fn publish(hwnd: isize, gen: usize, hits: Vec<FindHit>, more: bool, done: bool, engine: FindEngine) {
    if live_gen(hwnd) != gen {
        return;
    }
    INBOX.lock().unwrap_or_else(|e| e.into_inner()).push(Batch { hwnd, gen, hits, more, done, engine });
    unsafe { _ = PostMessageW(Some(HWND(hwnd as _)), WM_FIND, WPARAM(0), LPARAM(0)) };
}

fn find_worker() {
    loop {
        let job = {
            let mut slot = JOB.lock().unwrap_or_else(|e| e.into_inner());
            while slot.is_none() {
                slot = WAKE.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
            slot.take().unwrap()
        };
        let stop = || live_gen(job.hwnd) != job.gen;
        if stop() {
            continue;
        }
        if job.everything {
            if let Some(plan) = search_text(&job.root, &job.query, job.everywhere, job.hidden) {
                if !stop() {
                    if let Some(list) = everything::search(&plan.text, MAX_RESULTS) {
                        if !stop() {
                            let hits = list.hits.into_iter().map(|h| FindHit { name: h.name, path: h.path, dir: h.dir }).collect();
                            publish(job.hwnd, job.gen, hits, list.more, true, FindEngine::Everything);
                            continue;
                        }
                    }
                }
            }
        }
        let root = PathBuf::from(&job.root);
        let walked = walk_search(&root, &job.query, job.recursive, job.hidden, FIND_CAP, &stop, |delta, more| {
            if !stop() {
                publish(job.hwnd, job.gen, delta.to_vec(), more, false, FindEngine::Walk);
            }
        });
        // A cancel sets stopped without filling the cap. A full cap sets more and is still delivered.
        if !stop() && (!walked.stopped || walked.more) {
            publish(job.hwnd, job.gen, Vec::new(), walked.more, true, FindEngine::Walk);
        }
    }
}

pub(crate) unsafe extern "system" fn find_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pane = GetParent(h).unwrap_or_default();
    bind_hwnd(h);
    match msg {
        WM_PAINT => paint_find(h, pane),
        WM_ERASEBKGND => return LRESULT(1),
        WM_MOUSEWHEEL => {
            let delta = ((wp.0 >> 16) as i16 as i32) / 120;
            scroll_find(pane, -delta * row_h(app(|a| a.dpi)) * 3);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = crate::xy(lp);
            let row = click_find(pane, x, y, msg == WM_LBUTTONDBLCLK);
            if row && msg != WM_LBUTTONDBLCLK {
                // A result row takes keyboard focus. The header, its checkbox, and empty space
                // leave the caret in the search box. Space and F3 are read in the key loop
                // once this window is focused, and preview the selected row.
                _ = SetFocus(Some(h));
            } else if !row {
                if let Ok(e) = GetDlgItem(Some(pane), ID_FILTER) {
                    _ = SetFocus(Some(e));
                }
            }
        }
        // Up and Down move the row. Enter opens it. Esc clears the search, as in the box.
        // Space and F3 are not here: the key loop previews the selected row while this window is focused.
        WM_KEYDOWN => {
            let Some(i) = pane_of(pane) else { return DefWindowProcW(h, msg, wp, lp) };
            let down = |k: VIRTUAL_KEY| GetKeyState(k.0 as i32) < 0;
            let plain = !down(VK_CONTROL) && !down(VK_SHIFT) && !down(VK_MENU);
            match VIRTUAL_KEY(wp.0 as u16) {
                VK_UP if plain => { nudge_find(i, -1); }
                VK_DOWN if plain => { nudge_find(i, 1); }
                VK_PRIOR if plain => { nudge_find(i, -8); }
                VK_NEXT if plain => { nudge_find(i, 8); }
                VK_RETURN => {
                    if !open_find_sel(i) {
                        apply_filter(i);
                        focus_view(i);
                    }
                }
                VK_ESCAPE => {
                    if let Ok(e) = GetDlgItem(Some(pane), ID_FILTER) {
                        _ = SetWindowTextW(e, w!(""));
                    }
                    find_clear(i);
                    apply_filter(i);
                    focus_view(i);
                }
                _ => return DefWindowProcW(h, msg, wp, lp),
            }
        }
        _ => return DefWindowProcW(h, msg, wp, lp),
    }
    LRESULT(0)
}

unsafe fn scroll_find(pane: HWND, dy: i32) {
    let dpi = app(|a| a.dpi);
    let mut map = find_map();
    let Some(u) = map.get_mut(&(pane.0 as isize)) else { return };
    let rows = u.hits.len() as i32 + if u.more { 1 } else { 0 };
    let view = list_view_h(hwnd_of(u.list), dpi);
    let max = (rows * row_h(dpi) - view).max(0);
    u.scroll = (u.scroll + dy).clamp(0, max);
    let list = hwnd_of(u.list);
    drop(map);
    if !list.0.is_null() {
        _ = InvalidateRect(Some(list), None, false);
    }
}

/// Design pixels (96 dpi) of the search-list header.
/// The subfolder checkbox and its label occupy `[0, SUB_LABEL_RIGHT)`.
/// The scope pill is `[SCOPE_LEFT, SCOPE_RIGHT)`.
const SUB_LABEL_RIGHT: i32 = 146;
const SCOPE_LEFT: i32 = 150;
const SCOPE_RIGHT: i32 = 292;

fn header_px(v: i32, dpi: i32) -> i32 {
    v * dpi / 96
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HeaderHit {
    Sub,
    Scope,
    None,
}

/// Click target in the search-list header. The checkbox square and the Subfolders label
/// are `Sub`. The scope pill is `Scope` only while the search includes subfolders.
/// The gap before the pill and the status text are `None`.
pub(crate) fn header_hit(x: i32, dpi: i32, recursive: bool) -> HeaderHit {
    if x >= 0 && x < header_px(SUB_LABEL_RIGHT, dpi) {
        HeaderHit::Sub
    } else if recursive && x >= header_px(SCOPE_LEFT, dpi) && x < header_px(SCOPE_RIGHT, dpi) {
        HeaderHit::Scope
    } else {
        HeaderHit::None
    }
}

unsafe fn click_find(pane: HWND, x: i32, y: i32, dbl: bool) -> bool {
    let i = match pane_of(pane) {
        Some(i) => i,
        None => return false,
    };
    let dpi = app(|a| a.dpi);
    let rh = row_h(dpi);
    if y < rh {
        // CS_DBLCLKS: a quick second press is WM_LBUTTONDBLCLK, not another WM_LBUTTONDOWN.
        // Toggle once for that message, the same as a single click. `dbl` stays for result rows.
        let recursive = find_map().get(&(pane.0 as isize)).is_some_and(|u| u.recursive);
        match header_hit(x, dpi, recursive) {
            HeaderHit::Sub => toggle_find_sub(i),
            HeaderHit::Scope => toggle_find_scope(i),
            HeaderHit::None => {}
        }
        return false;
    }
    let hit = {
        let mut map = find_map();
        let Some(u) = map.get_mut(&(pane.0 as isize)) else { return false };
        let idx = (y - rh + u.scroll) / rh;
        if idx < 0 || idx as usize >= u.hits.len() {
            return false;
        }
        u.sel = idx as usize;
        let hit = u.hits.get(u.sel).cloned();
        let list = hwnd_of(u.list);
        drop(map);
        if !list.0.is_null() {
            _ = InvalidateRect(Some(list), None, false);
        }
        hit
    };
    // True for a result row: the caller focuses this list. A double-click opens the hit
    // and moves focus on to the folder. The header returns false above.
    if dbl {
        if let Some(hit) = hit {
            activate_hit(i, &hit);
        }
    }
    true
}

unsafe fn paint_find(h: HWND, pane: HWND) {
    let dpi = app(|a| a.dpi);
    let (font, icon_font, accent) = app(|a| (a.font, a.icon_font, a.s.accent_now()));
    let snap = {
        let map = find_map();
        map.get(&(pane.0 as isize)).map(|u| (u.query.clone(), u.root.clone(), u.recursive, u.everywhere, u.engine, u.hits.clone(), u.more, u.running, u.sel, u.scroll))
    };
    let Some((query, root, recursive, everywhere, engine, hits, more, running, sel, scroll)) = snap else {
        buffered(h, |dc, rc| fill(dc, rc, pal().card));
        return;
    };
    let all = everywhere || forces_everywhere(&query);
    buffered(h, |dc, rc| {
        let rh = row_h(dpi);
        let s = |v: i32| v * dpi / 96;
        fill(dc, rc, pal().card);
        let head = RECT { left: 0, top: 0, right: rc.right, bottom: rh };
        fill(dc, head, pal().bg);
        let mark = RECT { left: s(8), top: (rh - s(14)) / 2, right: s(8) + s(14), bottom: (rh - s(14)) / 2 + s(14) };
        fill(dc, mark, if recursive { accent } else { pal().card });
        frame(dc, mark, 1, if recursive { accent } else { pal().line });
        if recursive {
            text(dc, mark, "\u{E73E}", icon_font, on_color(accent), DT_CENTER);
        }
        let label_right = if recursive { header_px(SUB_LABEL_RIGHT, dpi) } else { rc.right / 2 };
        text(dc, RECT { left: s(26), right: label_right, ..head }, tr("Subfolders", "하위 폴더", "サブフォルダー"), font, pal().text, DT_LEFT);
        if recursive {
            let pill = RECT { left: header_px(SCOPE_LEFT, dpi), top: s(3), right: header_px(SCOPE_RIGHT, dpi), bottom: rh - s(3) };
            fill(dc, pill, pal().card);
            frame(dc, pill, 1, if all { accent } else { pal().line });
            let scope = if all { tr("All drives", "모든 드라이브", "すべてのドライブ") } else { tr("This folder", "이 폴더", "このフォルダー") };
            text(dc, pill, scope, font, if all { accent } else { pal().text }, DT_CENTER);
        }
        let status = find_status(running, hits.len(), more, engine, all && engine == FindEngine::Everything);
        let status_left = if recursive { s(300) } else { rc.right / 2 };
        text(dc, RECT { left: status_left, right: rc.right - s(8), ..head }, &status, font, pal().sub, DT_RIGHT);
        let mut y = rh - scroll;
        if hits.is_empty() && !running {
            text(dc, RECT { left: s(8), top: rh, right: rc.right - s(8), bottom: rh + rh }, tr("No matches", "결과 없음", "一致なし"), font, pal().sub, DT_LEFT);
        }
        for (i, hit) in hits.iter().enumerate() {
            let r = RECT { left: 0, top: y, right: rc.right, bottom: y + rh };
            if r.bottom < rh || r.top > rc.bottom {
                y += rh;
                continue;
            }
            if i == sel {
                fill(dc, r, blend(pal().card, accent, 0.28));
            }
            let label = hit_label(&root, hit);
            text(dc, RECT { left: s(8), right: rc.right - s(8), ..r }, &label, font, if hit.dir { pal().text } else { pal().text }, DT_LEFT);
            y += rh;
        }
        if more {
            let r = RECT { left: s(8), top: y, right: rc.right - s(8), bottom: y + rh };
            text(dc, r, tr("More results are not shown", "나머지는 표시하지 않아요", "これ以上は表示していません"), font, pal().sub, DT_LEFT);
        }
    });
}

fn find_status(running: bool, n: usize, more: bool, engine: FindEngine, everywhere: bool) -> String {
    let engine = match engine {
        FindEngine::Everything if everywhere => tr("Everything · all drives", "Everything · 모든 드라이브", "Everything · すべてのドライブ"),
        FindEngine::Everything => "Everything",
        FindEngine::Walk => tr("Folders", "폴더", "フォルダー"),
    };
    if running && n == 0 {
        return format!("{engine} · {}", tr("Searching…", "검색 중…", "検索中…"));
    }
    let mut s = format!("{engine} · {n} {}", tr("results", "개 결과", "件"));
    if more {
        s.push_str(" · ");
        s.push_str(tr("more", "더 있음", "続きあり"));
    } else if running {
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_substring_wildcard_and_either_term() {
        assert!(name_matches("MyReport.PDF", "report"));
        assert!(name_matches("보고서2026.txt", "보고서"));
        assert!(!name_matches("notes.txt", "report"));
        assert!(!name_matches("a.txt", "   ;  "));
        assert!(name_matches("a.txt", "*.TXT"));
        assert!(name_matches("abc", "a*c") && name_matches("abbc", "a*c") && !name_matches("ab", "a*c"));
        assert!(name_matches("abc", "a?c") && !name_matches("ac", "a?c"));
        assert!(name_matches("song.mp3", "*.md;*.mp3"));
        assert!(!name_matches("song.mp3", "*.md;*.txt"));
        assert_eq!(find_engine(false, true, true), FindEngine::Walk);
        assert_eq!(find_engine(true, false, true), FindEngine::Walk);
        assert_eq!(find_engine(true, true, false), FindEngine::Walk);
        assert_eq!(find_engine(true, true, true), FindEngine::Everything);
    }

    #[test]
    fn opening_a_hit_goes_to_the_folder_or_the_file() {
        assert_eq!(hit_open(r"C:\a\b", true), FindHitOpen::Folder(r"C:\a\b".into()));
        assert_eq!(hit_open(r"C:\a\b.txt", false), FindHitOpen::File { path: r"C:\a\b.txt".into(), folder: r"C:\a".into() });
        let hit = FindHit { name: "b.txt".into(), path: r"C:\Work\sub\b.txt".into(), dir: false };
        assert_eq!(hit_label(r"C:\Work", &hit), "b.txt  ·  sub");
        let here = FindHit { name: "b.txt".into(), path: r"C:\Work\b.txt".into(), dir: false };
        assert_eq!(hit_label(r"C:\Work\", &here), "b.txt");
    }

    #[test]
    fn header_hit_is_the_checkbox_label_or_the_scope_pill() {
        // 96 dpi: label ends at 146, pill is [150, 292).
        assert_eq!(header_hit(0, 96, true), HeaderHit::Sub);
        assert_eq!(header_hit(8, 96, true), HeaderHit::Sub);
        assert_eq!(header_hit(19, 96, true), HeaderHit::Sub);
        assert_eq!(header_hit(26, 96, true), HeaderHit::Sub);
        assert_eq!(header_hit(145, 96, true), HeaderHit::Sub);
        assert_eq!(header_hit(146, 96, true), HeaderHit::None);
        assert_eq!(header_hit(149, 96, true), HeaderHit::None);
        assert_eq!(header_hit(150, 96, true), HeaderHit::Scope);
        assert_eq!(header_hit(291, 96, true), HeaderHit::Scope);
        assert_eq!(header_hit(292, 96, true), HeaderHit::None);
        assert_eq!(header_hit(400, 96, true), HeaderHit::None);
        assert_eq!(header_hit(-1, 96, true), HeaderHit::None);
        // The pill is drawn only while the search includes subfolders.
        assert_eq!(header_hit(0, 96, false), HeaderHit::Sub);
        assert_eq!(header_hit(145, 96, false), HeaderHit::Sub);
        assert_eq!(header_hit(150, 96, false), HeaderHit::None);
        assert_eq!(header_hit(291, 96, false), HeaderHit::None);
        // 192 dpi is exactly double; 120 dpi truncates the same way the paint scale does.
        assert_eq!(header_hit(291, 192, true), HeaderHit::Sub);
        assert_eq!(header_hit(292, 192, true), HeaderHit::None);
        assert_eq!(header_hit(299, 192, true), HeaderHit::None);
        assert_eq!(header_hit(300, 192, true), HeaderHit::Scope);
        assert_eq!(header_hit(583, 192, true), HeaderHit::Scope);
        assert_eq!(header_hit(584, 192, true), HeaderHit::None);
        assert_eq!(header_hit(300, 192, false), HeaderHit::None);
        assert_eq!(header_hit(181, 120, true), HeaderHit::Sub);
        assert_eq!(header_hit(182, 120, true), HeaderHit::None);
        assert_eq!(header_hit(186, 120, true), HeaderHit::None);
        assert_eq!(header_hit(187, 120, true), HeaderHit::Scope);
        assert_eq!(header_hit(364, 120, true), HeaderHit::Scope);
        assert_eq!(header_hit(365, 120, true), HeaderHit::None);
        assert_eq!(header_hit(187, 120, false), HeaderHit::None);
    }

    #[test]
    fn result_neighbour_stops_at_either_end() {
        assert_eq!(find_neighbour(0, 3, true), None);
        assert_eq!(find_neighbour(0, 3, false), Some(1));
        assert_eq!(find_neighbour(1, 3, true), Some(0));
        assert_eq!(find_neighbour(2, 3, false), None);
        assert_eq!(find_neighbour(2, 3, true), Some(1));
        assert_eq!(find_neighbour(0, 0, false), None);
        assert_eq!(find_neighbour(0, 1, false), None);
        assert_eq!(find_neighbour(4, 3, false), None);
    }

    #[test]
    fn selection_stays_in_range_and_in_view() {
        assert_eq!(clamp_sel(-3, 4), 0);
        assert_eq!(clamp_sel(9, 4), 3);
        assert_eq!(clamp_sel(1, 0), 0);
        assert_eq!(scroll_to_sel(0, 40, 100, 20), 0);
        assert_eq!(scroll_to_sel(8, 0, 60, 20), 120);
        assert_eq!(scroll_to_sel(1, 0, 80, 20), 0);
    }

    #[test]
    fn walk_is_case_blind_capped_and_stops() {
        let root = std::env::temp_dir().join(format!("grids-find-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Sub")).unwrap();
        std::fs::write(root.join("Report.TXT"), b"a").unwrap();
        std::fs::write(root.join("other.md"), b"b").unwrap();
        std::fs::write(root.join("Sub").join("deep.TXT"), b"c").unwrap();
        let mut flat_hits = Vec::new();
        let flat = walk_search(&root, "txt", false, true, 10, &|| false, |d, _| flat_hits.extend_from_slice(d));
        let names = flat_hits.iter().map(|h| h.name.to_lowercase()).collect::<Vec<_>>();
        assert!(names.contains(&"report.txt".to_string()));
        assert!(!names.iter().any(|n| n == "deep.txt"));
        assert!(!flat.more && !flat.stopped);
        let mut deep_hits = Vec::new();
        let deep = walk_search(&root, "*.txt", true, true, 10, &|| false, |d, _| deep_hits.extend_from_slice(d));
        assert_eq!(deep_hits.len(), 2);
        assert!(!deep.more && !deep.stopped);
        let mut cap_hits = Vec::new();
        let cap = walk_search(&root, "*", true, true, 1, &|| false, |d, _| cap_hits.extend_from_slice(d));
        assert_eq!(cap_hits.len(), 1);
        assert!(cap.more);
        let mut stopped_hits = Vec::new();
        let stopped = walk_search(&root, "*", true, true, 10, &|| true, |d, _| stopped_hits.extend_from_slice(d));
        assert!(stopped.stopped);
        assert!(stopped_hits.is_empty());
        _ = std::fs::remove_dir_all(&root);
    }
}

/// The first subfolder search on a PC without Everything asks once whether to install it. Either answer
/// is remembered, so it is never asked again. Settings keeps an install button to do it later.
unsafe fn ask_install_everything_once(owner: HWND) {
    if app(|a| a.s.ev_asked) || everything::running() || !everything::not_installed() {
        return;
    }
    app(|a| a.s.ev_asked = true);
    crate::save_session();
    let q = tr(
        "Install Everything to make subfolder search much faster?\n\nThis is asked only once. You can install it later from Settings.",
        "Everything을 설치하면 하위 폴더 검색이 훨씬 빨라져요. 설치할까요?\n\n이 질문은 한 번만 합니다. 나중에 설정 화면에서 설치할 수 있어요.",
        "Everything をインストールすると、サブフォルダー検索がずっと速くなります。インストールしますか？\n\nこの質問は一度だけです。あとで設定画面からインストールできます。",
    );
    if MessageBoxW(Some(owner), &HSTRING::from(q), w!("grids"), MB_YESNO | MB_ICONQUESTION) == IDYES {
        everything::install_async();
    }
}
