use std::sync::Arc;

use crate::imports::*;
use super::*;
use crate::app::*;
use crate::ui::*;
use crate::session::*;
use crate::shell::*;
use crate::i18n::*;
use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetFileAttributesW, INVALID_FILE_ATTRIBUTES};

// ---------- navigation ----------

/// How long a UNC or mapped drive may block a worker before the pane gives up.
/// A dead share's redirector timeout is ~45s; the UI thread never waits that out.
/// The breadcrumb `›` list uses the same wait.
pub(crate) const NET_WAIT_MS: u32 = 8000;
const NET_CAP: usize = 4;

fn slash(c: u8) -> bool {
    c == b'\\' || c == b'/'
}

/// `\\server\share`, `//server/share`, and `\\?\UNC\...`. `\\?\C:\` and `\\.\` are not shares.
pub(crate) fn unc_path(path: &str) -> bool {
    let p = path.trim();
    let b = p.as_bytes();
    if b.len() < 2 || !slash(b[0]) || b[0] != b[1] {
        return false;
    }
    if b.len() >= 4 && (b[2] == b'?' || b[2] == b'.') && slash(b[3]) {
        if b[2] != b'?' {
            return false;
        }
        let r = p[4..].as_bytes();
        return r.len() >= 4 && r[..3].eq_ignore_ascii_case(b"UNC") && slash(r[3]);
    }
    true
}

/// `C:\foo` and `\\?\D:\foo` → `C:\` / `D:\`. A UNC path has no drive root.
pub(crate) fn drive_root(path: &str) -> Option<String> {
    let p = path.trim();
    let b = p.as_bytes();
    let rest = if b.len() >= 4 && slash(b[0]) && b[0] == b[1] && b[2] == b'?' && slash(b[3]) {
        let r = &p[4..];
        let rb = r.as_bytes();
        if rb.len() >= 4 && rb[..3].eq_ignore_ascii_case(b"UNC") && slash(rb[3]) {
            return None;
        }
        r
    } else if b.len() >= 2 && slash(b[0]) && b[0] == b[1] {
        return None;
    } else {
        p
    };
    let rb = rest.as_bytes();
    if rb.len() >= 2 && rb[1] == b':' && rb[0].is_ascii_alphabetic() {
        Some(format!("{}:\\", (rb[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

/// Drive type 4 is `DRIVE_REMOTE`. Callers pass `None` when the path has no drive letter.
pub(crate) fn share_kind(unc: bool, drive_type: Option<u32>) -> bool {
    unc || drive_type == Some(4)
}

/// Where a path may be checked. A share is never stat'd on the UI thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PathPlace {
    Share,
    Local,
    Other,
}

pub(crate) fn path_place(unc: bool, remote: bool, fs: bool) -> PathPlace {
    if unc || remote {
        PathPlace::Share
    } else if fs {
        PathPlace::Local
    } else {
        PathPlace::Other
    }
}

/// `GetFileAttributesW`: valid attributes, or the Win32 error when they are not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LocalHit {
    Here,
    Gone,
    Other,
}

/// Not found is the only result treated as missing. Access denied and busy drives are not.
pub(crate) fn local_hit(valid: bool, err: u32) -> LocalHit {
    if valid {
        LocalHit::Here
    } else if err == ERROR_FILE_NOT_FOUND.0 || err == ERROR_PATH_NOT_FOUND.0 {
        LocalHit::Gone
    } else {
        LocalHit::Other
    }
}

/// What activating a favourite should do. A share is never a missing local folder:
/// it is not removed, and the pane can show the network error view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FavAct {
    Open,
    Missing,
    Share,
}

pub(crate) fn fav_act(place: PathPlace, hit: Option<LocalHit>) -> FavAct {
    match place {
        PathPlace::Share => FavAct::Share,
        PathPlace::Local if hit == Some(LocalHit::Gone) => FavAct::Missing,
        _ => FavAct::Open,
    }
}

/// `IExplorerBrowser::BrowseToIDList` access-violates on a null PIDL when the flags are
/// absolute (`SBSP_ABSOLUTE` is 0). Back, forward, and parent document a null PIDL.
pub(crate) fn pidl_browse_ok(pidl_null: bool, flags: u32) -> bool {
    if !pidl_null {
        return true;
    }
    let travel = SBSP_PARENT | SBSP_NAVIGATEBACK | SBSP_NAVIGATEFORWARD;
    (flags & travel) != 0
}

/// UNC, or a mapped network drive. `GetDriveTypeW` is only asked about a drive root:
/// calling it on `\\server\share` itself can sit on the redirector.
pub(crate) unsafe fn share_path(path: &str) -> bool {
    if unc_path(path) {
        return true;
    }
    let kind = drive_root(path).map(|root| GetDriveTypeW(&HSTRING::from(root)));
    share_kind(false, kind)
}

/// The address bar may treat `path` as the current folder. A share skips the stat.
pub(crate) fn edit_cur_ok(fs: bool, share: bool, is_dir: Option<bool>) -> bool {
    fs && (share || is_dir == Some(true))
}

/// Path the address bar opens. A share uses `full` without a stat. A local `full` is
/// used only when `exists`, which is what `metadata` used to decide.
pub(crate) fn edit_target<'a>(typed: &'a str, full: Option<&'a str>, share: bool, exists: bool) -> &'a str {
    match full {
        Some(f) if share || exists => f,
        _ => typed,
    }
}

/// Status line while a share is in flight. Same words as the address pill.
pub(crate) fn net_wait_text() -> &'static str {
    net_bar_text(false)
}

/// Status line when the share did not answer. Same words as the address pill.
pub(crate) fn net_fail_text() -> &'static str {
    net_bar_text(true)
}

/// Address-bar line while a share is in flight. A failure keeps the path in the edit instead.
pub(crate) fn net_bar_text(bad: bool) -> &'static str {
    if bad {
        tr("Unreachable", "연결 실패", "接続失敗")
    } else {
        tr("Connecting…", "연결 중…", "接続中…")
    }
}

/// Seconds named in the timeout message. The same wait the worker is given.
pub(crate) fn net_wait_secs() -> u32 {
    NET_WAIT_MS / 1000
}

/// Sentence for the in-pane error. `secs` is set only when the wait ran out.
pub(crate) fn net_fail_message(path: &str, secs: Option<u32>) -> String {
    let n = secs.unwrap_or(0);
    match (ui(), secs) {
        (1, Some(_)) => format!("{path}에 연결하지 못했습니다 (시간 초과 {n}초)"),
        (2, Some(_)) => format!("{path} に接続できませんでした（タイムアウト {n}秒）"),
        (_, Some(_)) => format!("Couldn't connect to {path} (timed out after {n} seconds)"),
        (1, None) => format!("{path}에 연결하지 못했습니다"),
        (2, None) => format!("{path} に接続できませんでした"),
        _ => format!("Couldn't connect to {path}"),
    }
}

/// [Retry] and [Previous folder], in the UI language.
pub(crate) fn net_fail_buttons() -> (&'static str, &'static str) {
    (tr("Retry", "다시 시도", "再試行"), tr("Previous folder", "이전 폴더로", "前のフォルダー"))
}

/// Path a Retry or F5 opens. An empty address falls back to the path that failed.
pub(crate) fn retry_path<'a>(stored: &'a str, typed: &'a str) -> &'a str {
    let t = typed.trim();
    if t.is_empty() { stored } else { t }
}

/// Unfocused empty address after a failure shows the attempt again, so the bar cannot go blank.
pub(crate) fn addr_keep<'a>(attempted: &'a str, typed: &str, focused: bool) -> Option<&'a str> {
    if focused || !typed.trim().is_empty() { None } else { Some(attempted) }
}

/// Back and Forward while the error is up. The failed path was not opened, so they reveal the
/// folder already current instead of walking the browser's history.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FolderStep {
    Stay,
    History,
}

pub(crate) fn folder_step(failed: bool, history: bool) -> FolderStep {
    if failed && history { FolderStep::Stay } else { FolderStep::History }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct BoxI {
    pub(crate) l: i32,
    pub(crate) t: i32,
    pub(crate) r: i32,
    pub(crate) b: i32,
}

impl BoxI {
    pub(crate) fn contains(self, x: i32, y: i32) -> bool {
        x >= self.l && x < self.r && y >= self.t && y < self.b
    }

    pub(crate) fn to_rect(self) -> RECT {
        RECT { left: self.l, top: self.t, right: self.r, bottom: self.b }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct FailLayout {
    pub(crate) message: BoxI,
    pub(crate) retry: BoxI,
    pub(crate) back: BoxI,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FailHit {
    Retry,
    Back,
    None,
}

/// Message above, the two buttons centred just under the middle of the list.
pub(crate) fn fail_layout(w: i32, h: i32, dpi: i32, retry_w: i32, back_w: i32) -> FailLayout {
    let s = |v: i32| v * dpi / 96;
    let pad = s(16);
    let gap = s(8);
    let bh = s(30).clamp(1, h.max(1));
    let mut rw = (retry_w + pad * 2).max(s(80));
    let mut bw = (back_w + pad * 2).max(s(80));
    // A narrow pane still shows both buttons. Widths shrink together; they do not run off the list.
    let room = (w - gap).max(0);
    let buttons = rw.saturating_add(bw);
    if buttons > room && buttons > 0 {
        rw = (rw as i64 * i64::from(room) / i64::from(buttons)) as i32;
        bw = room - rw;
    }
    let total = rw + gap + bw;
    let left = ((w - total) / 2).max(0);
    let max_top = (h - bh).max(0);
    let btn_top = (h / 2 + s(6)).clamp(0, max_top);
    let retry = BoxI { l: left, t: btn_top, r: left + rw, b: btn_top + bh };
    let back_l = retry.r + gap;
    let back = BoxI { l: back_l, t: btn_top, r: back_l + bw, b: btn_top + bh };
    let inset = s(20).min((w / 4).max(0));
    let msg_top = s(8).min(btn_top);
    let msg_b = (btn_top - s(8)).max(msg_top);
    let message = BoxI { l: inset, t: msg_top, r: (w - inset).max(inset), b: msg_b };
    FailLayout { message, retry, back }
}

pub(crate) fn fail_hit(lay: &FailLayout, x: i32, y: i32) -> FailHit {
    if lay.retry.contains(x, y) {
        FailHit::Retry
    } else if lay.back.contains(x, y) {
        FailHit::Back
    } else {
        FailHit::None
    }
}

/// Break a long UNC path inside DrawText without changing the characters the user reads.
fn fail_draw_text(msg: &str) -> String {
    msg.replace('\\', "\\\u{200B}").replace('/', "/\u{200B}")
}

fn clear_net_notes(a: &mut App, path: &str) {
    let wait = net_wait_text();
    let fail = net_fail_text();
    // Another pane may still be on the same short line. Leave it until that pane finishes.
    let other = a.panes.iter().flatten().any(|p| p.net.as_ref().is_some_and(|n| n.path != path));
    if other {
        return;
    }
    if a.note.as_ref().is_some_and(|(t, _)| t == wait || t == fail) {
        a.note = None;
    }
}

/// What the address bar asked to do if the name is not on the share.
pub(crate) struct NetOpen {
    pub(crate) hwnd: isize,
    pub(crate) id: usize,
    pub(crate) path: String,
    pub(crate) edit: bool,
    pub(crate) run: Option<(String, String)>,
    pub(crate) dir: Option<String>,
    pub(crate) dir_share: bool,
    pub(crate) at: Instant,
}

struct NetSlot {
    open: NetOpen,
    pidl: usize,
    sfgao: u32,
    ready: bool,
}

pub(crate) struct NetStop {
    pub(crate) path: String,
    pub(crate) run: Option<(String, String)>,
    /// Local directory only. A share is not handed to `ShellExecute` from a timeout.
    pub(crate) dir: Option<String>,
}

pub(crate) struct NetDone {
    pub(crate) path: String,
    pub(crate) edit: bool,
    pub(crate) run: Option<(String, String)>,
    pub(crate) dir: Option<String>,
    pub(crate) pidl: usize,
    pub(crate) sfgao: u32,
}

pub(crate) enum NetWait {
    Idle,
    Early(u32),
    Ready,
    Stop(NetStop),
}

/// One in-flight resolve per pane window. A newer request drops the previous id,
/// so a late PIDL cannot browse somewhere the user has already left.
pub(crate) struct NetBook {
    slots: Vec<NetSlot>,
}

impl NetBook {
    pub(crate) fn begin(&mut self, open: NetOpen) -> Option<usize> {
        let hwnd = open.hwnd;
        let old = self.slots.iter().position(|s| s.open.hwnd == hwnd).map(|i| self.slots.remove(i).pidl).filter(|p| *p != 0);
        self.slots.push(NetSlot { open, pidl: 0, sfgao: 0, ready: false });
        old
    }

    /// False when this id is no longer the pane's request. The caller frees `pidl`.
    pub(crate) fn publish(&mut self, hwnd: isize, id: usize, pidl: usize, sfgao: u32) -> bool {
        let Some(s) = self.slots.iter_mut().find(|s| s.open.hwnd == hwnd && s.open.id == id && !s.ready) else {
            return false;
        };
        s.pidl = pidl;
        s.sfgao = sfgao;
        s.ready = true;
        true
    }

    pub(crate) fn take(&mut self, hwnd: isize, id: usize) -> Option<NetDone> {
        let i = self.slots.iter().position(|s| s.open.hwnd == hwnd && s.open.id == id && s.ready)?;
        let s = self.slots.remove(i);
        Some(NetDone { path: s.open.path, edit: s.open.edit, run: s.open.run, dir: s.open.dir, pidl: s.pidl, sfgao: s.sfgao })
    }

    pub(crate) fn poll(&mut self, hwnd: isize, id: usize, now: Instant, wait: std::time::Duration) -> NetWait {
        let Some(i) = self.slots.iter().position(|s| s.open.hwnd == hwnd && s.open.id == id) else {
            return NetWait::Idle;
        };
        if self.slots[i].ready {
            return NetWait::Ready;
        }
        let elapsed = now.saturating_duration_since(self.slots[i].open.at);
        if elapsed < wait {
            let left = (wait - elapsed).as_millis().min(60_000) as u32;
            return NetWait::Early(left.max(1));
        }
        let s = self.slots.remove(i);
        let dir = if s.open.dir_share { None } else { s.open.dir };
        NetWait::Stop(NetStop { path: s.open.path, run: s.open.run, dir })
    }

    /// The worker never started (too many shares already stuck). Same shape as a timeout.
    pub(crate) fn force_stop(&mut self, hwnd: isize, id: usize) -> Option<NetStop> {
        let i = self.slots.iter().position(|s| s.open.hwnd == hwnd && s.open.id == id && !s.ready)?;
        let s = self.slots.remove(i);
        let dir = if s.open.dir_share { None } else { s.open.dir };
        Some(NetStop { path: s.open.path, run: s.open.run, dir })
    }

    pub(crate) fn cancel(&mut self, hwnd: isize) -> Option<usize> {
        let i = self.slots.iter().position(|s| s.open.hwnd == hwnd)?;
        let pidl = self.slots.remove(i).pidl;
        (pidl != 0).then_some(pidl)
    }
}

static NET: Mutex<NetBook> = Mutex::new(NetBook { slots: Vec::new() });
static NET_ID: AtomicUsize = AtomicUsize::new(1);
static NET_BUSY: AtomicUsize = AtomicUsize::new(0);

fn net_book() -> std::sync::MutexGuard<'static, NetBook> {
    NET.lock().unwrap_or_else(|e| e.into_inner())
}

fn next_net_id() -> usize {
    loop {
        let id = NET_ID.fetch_add(1, Ordering::Relaxed);
        if id != 0 {
            return id;
        }
    }
}

fn free_pidl(p: usize) {
    if p != 0 {
        unsafe { ILFree(Some(p as *const ITEMIDLIST)) };
    }
}

/// Posted when a share resolve finishes. wParam is the request id. The PIDL stays in `NET`
/// until the UI thread takes it, so a stale message frees nothing that is still in use.
pub(crate) const WM_NET: u32 = WM_APP + 55;

unsafe fn pane_hwnd(i: usize) -> Option<HWND> {
    app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd))
}

unsafe fn repaint_link(h: HWND) {
    let dpi = app(|a| a.dpi);
    _ = InvalidateRect(Some(h), Some(&addr_rect(h, dpi)), false);
    update_status();
}

/// Drop a share resolve for this pane. A local navigation, Back, or a destroyed pane.
pub(crate) unsafe fn end_share(i: usize, h: HWND) {
    _ = KillTimer(Some(h), TIMER_NET);
    let pidl = net_book().cancel(h.0 as isize);
    free_pidl(pidl.unwrap_or(0));
    let cleared = app(|a| {
        let p = a.panes.get_mut(i).and_then(|p| p.as_mut())?;
        if p.hwnd != h {
            return None;
        }
        let n = p.net.take()?;
        clear_net_notes(a, &n.path);
        Some(())
    });
    if cleared.is_some() {
        sync_net_fail(h);
        repaint_link(h);
    }
}

unsafe fn show_link(i: usize, h: HWND, id: usize, path: &str) {
    // The address edit stays up while its text isn't the folder on display. Put the current
    // folder back first, so hiding it (and the focus loss that follows) does not pop it open
    // again over the "Connecting…" line.
    let disp = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).filter(|p| p.hwnd == h).map(|p| p.disp.clone())).unwrap_or_default();
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        if text_of(e) != disp {
            _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
        }
        if IsWindowVisible(e).as_bool() {
            _ = ShowWindow(e, SW_HIDE);
        }
    }
    let armed = app(|a| {
        let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) else { return false };
        if p.hwnd != h {
            return false;
        }
        p.net = Some(NetUi { id, path: path.to_string(), bad: false, timed_out: false });
        // The folder we were in stays painted until a new icon arrives. Drop it for the wait.
        retire_folder_icon(p);
        a.lost = 0;
        a.note = Some((net_wait_text().to_string(), Instant::now() + std::time::Duration::from_millis(u64::from(NET_WAIT_MS) + 1000)));
        true
    });
    if armed {
        drop_icon_request(h);
        sync_net_fail(h);
    }
    repaint_link(h);
}

/// The shell icon of the folder we left. A late lookup for it must not land during the wait.
fn retire_folder_icon(p: &mut Pane) {
    p.icon = -1;
    p.ticket = next_ticket(&ICON_TICKET);
}

unsafe fn clear_link(i: usize, h: HWND, id: usize, path: &str) {
    let on = app(|a| {
        let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) else { return false };
        if p.hwnd != h || p.net.as_ref().is_some_and(|n| n.id != id) {
            // A newer resolve owns the pane. Still drop this path's note when it is showing.
            clear_net_notes(a, path);
            return false;
        }
        p.net = None;
        clear_net_notes(a, path);
        true
    });
    if on {
        sync_net_fail(h);
        repaint_link(h);
    } else {
        update_status();
    }
}

/// This pane is showing the share error, not a folder.
pub(crate) fn net_failed(i: usize) -> bool {
    app(|a| a.panes.get(i).and_then(|p| p.as_ref()).and_then(|p| p.net.as_ref()).is_some_and(|n| n.bad))
}

/// Put the failed path back in the address and cover the list. The browser is not moved:
/// the last good folder stays the current history entry.
unsafe fn show_link_fail(i: usize, h: HWND, id: usize, path: &str, timed_out: bool) {
    let shown = app(|a| {
        let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) else { return None };
        if p.hwnd != h {
            return None;
        }
        match p.net.as_mut() {
            Some(n) if n.id == id => {
                n.bad = true;
                n.timed_out = timed_out;
                if n.path.is_empty() {
                    n.path = path.to_string();
                }
            }
            Some(_) => return None,
            None => p.net = Some(NetUi { id, path: path.to_string(), bad: true, timed_out }),
        }
        let path = p.net.as_ref().map(|n| n.path.clone()).unwrap_or_default();
        retire_folder_icon(p);
        a.lost = 0;
        clear_net_notes(a, &path);
        Some(path)
    });
    let Some(path) = shown else { return };
    drop_icon_request(h);
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(path.as_str()));
        _ = ShowWindow(e, SW_SHOWNA);
        show_path_end(e, &path);
    }
    sync_net_fail(h);
    repaint_link(h);
    // The active pane's address is ready to edit. Enter retries. Another pane keeps the focus it has.
    if app(|a| a.s.active == i) {
        if let Some(e) = pane_ctl(i, ID_EDIT) {
            _ = SetFocus(Some(e));
            SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
        }
    }
}

/// Open the path in the address, or the path that failed when the address is empty.
pub(crate) unsafe fn retry_failed(i: usize) {
    let stored = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).and_then(|p| p.net.as_ref()).filter(|n| n.bad).map(|n| n.path.clone()));
    let Some(stored) = stored else { return };
    let typed = pane_ctl(i, ID_EDIT).map(|e| text_of(e)).unwrap_or_default();
    let path = retry_path(&stored, &typed).to_string();
    if path.is_empty() || !navigate(i, &path) {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    // Still failed means the attempt ended before it left this call. Leave the address focused.
    // Otherwise the list takes focus so a hidden error view does not keep it.
    if !net_failed(i) {
        focus_view(i);
    }
}

/// Drop the error and show the folder the browser is already on. History is not walked.
pub(crate) unsafe fn leave_net_fail(i: usize) {
    let Some(h) = pane_hwnd(i) else { return };
    if !net_failed(i) {
        return;
    }
    let disp = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.disp.clone())).unwrap_or_default();
    if let Some(e) = pane_ctl(i, ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
    }
    end_share(i, h);
    if let Some(e) = pane_ctl(i, ID_EDIT) {
        show_path_end(e, &disp);
        sync_addr(h);
    }
    focus_view(i);
}

/// Show or hide the error view over this pane's file list. The browser window stays where it is.
pub(crate) unsafe fn sync_net_fail(pane: HWND) {
    if pane.0.is_null() {
        return;
    }
    let show = app(|a| a.panes.iter().flatten().any(|p| p.hwnd == pane && p.net.as_ref().is_some_and(|n| n.bad)));
    if !show {
        if let Ok(f) = GetDlgItem(Some(pane), ID_NETFAIL) {
            _ = ShowWindow(f, SW_HIDE);
        }
        return;
    }
    let f = ensure_net_fail(pane);
    if f.0.is_null() {
        return;
    }
    let mut rc = RECT::default();
    _ = GetClientRect(pane, &mut rc);
    let view = pane_geo_of(pane, rc.right, rc.bottom, app(|a| a.dpi)).view;
    let (w, hgt) = ((view.right - view.left).max(0), (view.bottom - view.top).max(0));
    _ = SetWindowPos(f, Some(HWND_TOP), view.left, view.top, w, hgt, SWP_NOACTIVATE | SWP_SHOWWINDOW);
    let msg = app(|a| {
        let n = a.panes.iter().flatten().find(|p| p.hwnd == pane)?.net.as_ref().filter(|n| n.bad)?;
        Some(net_fail_message(&n.path, n.timed_out.then_some(net_wait_secs())))
    });
    if let Some(msg) = msg {
        _ = SetWindowTextW(f, &HSTRING::from(msg.as_str()));
    }
    _ = InvalidateRect(Some(f), None, false);
}

unsafe fn ensure_net_fail(pane: HWND) -> HWND {
    if let Ok(h) = GetDlgItem(Some(pane), ID_NETFAIL) {
        if IsWindow(Some(h)).as_bool() {
            return h;
        }
    }
    static CLASS: Once = Once::new();
    CLASS.call_once(|| {
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        _ = RegisterClassW(&WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(net_fail_proc),
            hInstance: hinst.into(),
            hCursor: arrow,
            lpszClassName: w!("PanegridNetFail"),
            ..Default::default()
        });
    });
    CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("PanegridNetFail"),
        w!(""),
        WS_CHILD | WS_CLIPSIBLINGS,
        0,
        0,
        0,
        0,
        Some(pane),
        Some(HMENU(ID_NETFAIL as isize as _)),
        None,
        None,
    )
    .unwrap_or_default()
}

static FAIL_PRESS: Mutex<Option<(isize, i32)>> = Mutex::new(None);

fn fail_code(hit: FailHit) -> i32 {
    match hit {
        FailHit::Retry => 1,
        FailHit::Back => 2,
        FailHit::None => 0,
    }
}

unsafe fn text_px(dc: HDC, font: HFONT, s: &str) -> i32 {
    let old = SelectObject(dc, font.into());
    let w: Vec<u16> = s.encode_utf16().collect();
    let mut sz = SIZE::default();
    if !w.is_empty() {
        _ = GetTextExtentPoint32W(dc, &w, &mut sz);
    }
    SelectObject(dc, old);
    sz.cx
}

unsafe fn fail_boxes(h: HWND, dc: HDC) -> Option<(FailLayout, String, String, String)> {
    let (path, timed, dpi, font) = app(|a| {
        let i = pane_of(h)?;
        let n = a.panes.get(i).and_then(|p| p.as_ref())?.net.as_ref().filter(|n| n.bad)?;
        Some((n.path.clone(), n.timed_out, a.dpi, a.font))
    })?;
    let msg = net_fail_message(&path, timed.then_some(net_wait_secs()));
    let (retry, back) = net_fail_buttons();
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let lay = fail_layout(rc.right.max(0), rc.bottom.max(0), dpi, text_px(dc, font, retry), text_px(dc, font, back));
    Some((lay, msg, retry.to_string(), back.to_string()))
}

unsafe fn fail_at(h: HWND, x: i32, y: i32) -> FailHit {
    let dc = GetDC(Some(h));
    let hit = fail_boxes(h, dc).map(|(lay, ..)| fail_hit(&lay, x, y)).unwrap_or(FailHit::None);
    _ = ReleaseDC(Some(h), dc);
    hit
}

unsafe fn paint_message(dc: HDC, r: RECT, msg: &str, font: HFONT) {
    if r.right <= r.left || r.bottom <= r.top {
        return;
    }
    let mut w: Vec<u16> = fail_draw_text(msg).encode_utf16().collect();
    if w.is_empty() {
        return;
    }
    let old = SelectObject(dc, font.into());
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, cref(pal().text));
    let flags = DT_CENTER | DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX;
    let mut measure = r;
    DrawTextW(dc, &mut w, &mut measure, flags | DT_CALCRECT);
    let th = measure.bottom - measure.top;
    let room = r.bottom - r.top;
    let mut dest = r;
    if th > 0 && th < room {
        dest.top += (room - th) / 2;
        dest.bottom = dest.top + th;
    }
    DrawTextW(dc, &mut w, &mut dest, flags);
    SelectObject(dc, old);
}

unsafe fn paint_fail_btn(dc: HDC, r: RECT, label: &str, font: HFONT, hot: bool, accent: u32) {
    fill(dc, r, if hot { blend(pal().pill, pal().text, 0.10) } else { pal().pill });
    frame(dc, r, 1, if hot { accent } else { pal().line });
    text(dc, r, label, font, pal().text, DT_CENTER);
}

unsafe fn paint_net_fail(h: HWND) {
    let hot = GetWindowLongPtrW(h, GWLP_USERDATA) as i32;
    let (accent, font) = app(|a| (a.s.accent_now(), a.font));
    buffered(h, |dc, rc| {
        fill(dc, rc, pal().card);
        let Some((lay, msg, retry, back)) = fail_boxes(h, dc) else { return };
        paint_message(dc, lay.message.to_rect(), &msg, font);
        paint_fail_btn(dc, lay.retry.to_rect(), &retry, font, hot == 1, accent);
        paint_fail_btn(dc, lay.back.to_rect(), &back, font, hot == 2, accent);
    });
}

pub(crate) unsafe extern "system" fn net_fail_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let pane = GetParent(h).unwrap_or_default();
    bind_hwnd(h);
    match msg {
        WM_PAINT => paint_net_fail(h),
        WM_ERASEBKGND => return LRESULT(1),
        WM_MOUSEMOVE => {
            let (x, y) = xy(lp);
            let code = fail_code(fail_at(h, x, y));
            let prev = GetWindowLongPtrW(h, GWLP_USERDATA) as i32;
            if prev != code {
                _ = SetWindowLongPtrW(h, GWLP_USERDATA, code as isize);
                _ = InvalidateRect(Some(h), None, false);
                if code != 0 && prev == 0 {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: h,
                        dwHoverTime: 0,
                    };
                    _ = TrackMouseEvent(&mut t);
                }
            }
        }
        WM_MOUSELEAVE => {
            _ = SetWindowLongPtrW(h, GWLP_USERDATA, 0);
            _ = InvalidateRect(Some(h), None, false);
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            let (x, y) = xy(lp);
            let code = fail_code(fail_at(h, x, y));
            *FAIL_PRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some((h.0 as isize, code));
            _ = SetCapture(h);
            _ = SetWindowLongPtrW(h, GWLP_USERDATA, code as isize);
            _ = InvalidateRect(Some(h), None, false);
        }
        WM_LBUTTONUP => {
            let (x, y) = xy(lp);
            _ = ReleaseCapture();
            let down = FAIL_PRESS.lock().unwrap_or_else(|e| e.into_inner()).take();
            let hit = fail_at(h, x, y);
            let same = down.is_some_and(|(hwnd, code)| hwnd == h.0 as isize && code == fail_code(hit) && code != 0);
            if same {
                if let Some(i) = pane_of(pane) {
                    set_active(i);
                    match hit {
                        FailHit::Retry => retry_failed(i),
                        FailHit::Back => leave_net_fail(i),
                        FailHit::None => {}
                    }
                }
            }
        }
        _ => return DefWindowProcW(h, msg, wp, lp),
    }
    LRESULT(0)
}

unsafe fn run_cmd(prog: &str, args: &str, dir: Option<&str>) -> bool {
    let dir = dir.map(HSTRING::from);
    let prog = HSTRING::from(prog);
    let args = HSTRING::from(args);
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_FLAG_NO_UI,
        lpFile: PCWSTR(prog.as_ptr()),
        lpParameters: PCWSTR(args.as_ptr()),
        lpDirectory: dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr())),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    ShellExecuteExW(&mut sei).is_ok()
}

unsafe fn open_idlist(pidl: *mut ITEMIDLIST) -> bool {
    let mut sei = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI,
        lpIDList: pidl as _,
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    ShellExecuteExW(&mut sei).is_ok()
}

unsafe fn finish_stop(h: HWND, id: usize, stop: NetStop) {
    let Some(i) = pane_of(h) else { return };
    if let Some((prog, args)) = stop.run {
        let ok = run_cmd(&prog, &args, stop.dir.as_deref());
        clear_link(i, h, id, &stop.path);
        if ok {
            if let Some(e) = pane_ctl(i, ID_EDIT) {
                addr_reset(i, e);
            }
        } else {
            _ = MessageBeep(MB_ICONWARNING);
        }
        return;
    }
    show_link_fail(i, h, id, &stop.path, true);
}

/// `SHParseDisplayName` off the UI thread. The PIDL is cloned (`ILClone`) before it is posted,
/// so the worker can drop COM. The UI thread browses only after that clone is in hand.
unsafe fn resolve_share(hwnd: isize, id: usize, path: &str) {
    let com = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
    let mut pidl = std::ptr::null_mut();
    let mut sfgao = 0u32;
    let parsed = com && SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, SFGAO_FOLDER.0, Some(&mut sfgao)).is_ok() && !pidl.is_null();
    let owned = if parsed {
        let cloned = ILClone(pidl);
        CoTaskMemFree(Some(pidl as _));
        cloned as usize
    } else {
        if !pidl.is_null() {
            CoTaskMemFree(Some(pidl as _));
        }
        0
    };
    let flags = if owned == 0 { 0 } else { sfgao };
    if net_book().publish(hwnd, id, owned, flags) {
        _ = PostMessageW(Some(HWND(hwnd as _)), WM_NET, WPARAM(id), LPARAM(0));
    } else {
        free_pidl(owned);
    }
    if com {
        CoUninitialize();
    }
}

fn spawn_share(hwnd: isize, id: usize, path: String) -> bool {
    if NET_BUSY.fetch_add(1, Ordering::Relaxed) >= NET_CAP {
        NET_BUSY.fetch_sub(1, Ordering::Relaxed);
        return false;
    }
    let spawned = std::thread::Builder::new().name("panegrid-net".into()).spawn(move || {
        unsafe { resolve_share(hwnd, id, &path) };
        NET_BUSY.fetch_sub(1, Ordering::Relaxed);
    });
    if spawned.is_err() {
        NET_BUSY.fetch_sub(1, Ordering::Relaxed);
        return false;
    }
    true
}

/// UNC or mapped path, off the UI thread. A file is launched once a PIDL comes back.
/// An unreachable path shows this pane's network error view (Retry / Previous folder).
/// False only when the pane is not open. The favourite itself is left in place.
pub(crate) unsafe fn open_share(i: usize, path: &str, file: bool) -> bool {
    start_share(i, path, file, None, None, false)
}

/// Start a share resolve. True when the pane is showing the wait (or the immediate error).
unsafe fn start_share(i: usize, path: &str, edit: bool, run: Option<(String, String)>, dir: Option<String>, dir_share: bool) -> bool {
    let Some(h) = pane_hwnd(i) else { return false };
    let id = next_net_id();
    let old = net_book().begin(NetOpen {
        hwnd: h.0 as isize,
        id,
        path: path.to_string(),
        edit,
        run,
        dir,
        dir_share,
        at: Instant::now(),
    });
    free_pidl(old.unwrap_or(0));
    show_link(i, h, id, path);
    SetTimer(Some(h), TIMER_NET, NET_WAIT_MS, None);
    if spawn_share(h.0 as isize, id, path.to_string()) {
        return true;
    }
    _ = KillTimer(Some(h), TIMER_NET);
    if let Some(stop) = net_book().force_stop(h.0 as isize, id) {
        finish_stop(h, id, stop);
    }
    true
}

/// Give up the share named by the pane's current wait. A result that already arrived is left for `WM_NET`.
pub(crate) unsafe fn net_timeout(h: HWND) {
    _ = KillTimer(Some(h), TIMER_NET);
    let Some(id) = app(|a| a.panes.iter().find_map(|p| p.as_ref().filter(|p| p.hwnd == h).and_then(|p| p.net.as_ref())).map(|n| n.id)) else {
        return;
    };
    let wait = std::time::Duration::from_millis(u64::from(NET_WAIT_MS));
    match net_book().poll(h.0 as isize, id, Instant::now(), wait) {
        NetWait::Early(ms) => {
            SetTimer(Some(h), TIMER_NET, ms, None);
        }
        NetWait::Stop(stop) => finish_stop(h, id, stop),
        NetWait::Idle | NetWait::Ready => {}
    }
}

/// `WM_NET`. A mismatched id (the user moved on, or the timer won) is ignored.
pub(crate) unsafe fn net_done(h: HWND, id: usize) {
    _ = KillTimer(Some(h), TIMER_NET);
    let Some(done) = net_book().take(h.0 as isize, id) else { return };
    let Some(i) = pane_of(h) else {
        free_pidl(done.pidl);
        return;
    };
    if done.pidl == 0 {
        if let Some((prog, args)) = done.run {
            let ok = run_cmd(&prog, &args, done.dir.as_deref());
            clear_link(i, h, id, &done.path);
            if ok {
                if let Some(e) = pane_ctl(i, ID_EDIT) {
                    addr_reset(i, e);
                }
            } else {
                _ = MessageBeep(MB_ICONWARNING);
            }
            return;
        }
        show_link_fail(i, h, id, &done.path, false);
        return;
    }
    let pidl = done.pidl as *mut ITEMIDLIST;
    clear_link(i, h, id, &done.path);
    if done.edit && done.sfgao & SFGAO_FOLDER.0 == 0 {
        let ok = open_idlist(pidl);
        free_pidl(done.pidl);
        if ok {
            if let Some(e) = pane_ctl(i, ID_EDIT) {
                addr_reset(i, e);
            }
        } else {
            _ = MessageBeep(MB_ICONWARNING);
        }
        return;
    }
    let ok = eb_of(i).is_some_and(|eb| browse_id(&eb, pidl, SBSP_ABSOLUTE).is_ok());
    free_pidl(done.pidl);
    if ok {
        focus_view(i);
    } else {
        show_link_fail(i, h, id, &done.path, false);
    }
}

pub(crate) unsafe fn navigate(i: usize, path: &str) -> bool {
    go_near(i, path)
}

/// Attributes of a local path. Does not touch a UNC or mapped drive.
pub(crate) unsafe fn probe_local(path: &str) -> LocalHit {
    let attr = GetFileAttributesW(&HSTRING::from(path));
    if attr != INVALID_FILE_ATTRIBUTES {
        LocalHit::Here
    } else {
        local_hit(false, GetLastError().0)
    }
}

/// A local filesystem path that is confirmed missing. Shares and shell places are not probed:
/// `GetFileAttributesW` on a dead share blocks for the redirector's timeout.
pub(crate) unsafe fn local_path_gone(path: &str) -> bool {
    let path = path.trim();
    if path.is_empty() || !is_fs(path) || share_path(path) {
        return false;
    }
    probe_local(path) == LocalHit::Gone
}

/// Browse only when the PIDL is safe to hand to the shell view. A null absolute PIDL is the crash.
unsafe fn browse_id(eb: &IExplorerBrowser, pidl: *mut ITEMIDLIST, flags: u32) -> Result<()> {
    if !pidl_browse_ok(pidl.is_null(), flags) {
        return Err(Error::from(HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)));
    }
    eb.BrowseToIDList(pidl, flags)
}

pub(crate) unsafe fn try_nav(i: usize, path: &str) -> Result<()> {
    if share_path(path) {
        // Accepted: BrowseToIDList runs on WM_NET, after the worker has a PIDL or the wait ends.
        return if start_share(i, path, false, None, None, false) { Ok(()) } else { Err(Error::from(E_FAIL)) };
    }
    if let Some(h) = pane_hwnd(i) {
        end_share(i, h);
    }
    // A missing local folder must not reach the shell view. SHParseDisplayName can return
    // S_OK with a null PIDL for a path that is already gone, and BrowseToIDList then
    // access-violates inside explorerframe. The caller (go_near) opens the nearest parent.
    if local_path_gone(path) {
        return Err(Error::from(HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)));
    }
    let eb = eb_of(i).ok_or_else(|| Error::from(E_FAIL))?;
    let mut pidl = std::ptr::null_mut();
    let parsed = SHParseDisplayName(&HSTRING::from(path), None, &mut pidl, 0, None);
    if parsed.is_err() || pidl.is_null() {
        if !pidl.is_null() {
            CoTaskMemFree(Some(pidl as _));
        }
        return parsed.and(Err(Error::from(HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0))));
    }
    let r = browse_id(&eb, pidl, SBSP_ABSOLUTE);
    CoTaskMemFree(Some(pidl as _));
    r
}

/// The shell's answer for a path that isn't there. Denied, offline, or not ready is something else.
pub(crate) fn gone(code: HRESULT) -> bool {
    code == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) || code == HRESULT::from_win32(ERROR_PATH_NOT_FOUND.0)
}

/// Nearest folder above `path` that still exists. Stops at anything but "not found".
pub(crate) fn near_dir(path: &str, probe: impl Fn(&Path) -> std::io::Result<bool>) -> Option<String> {
    for p in Path::new(path).ancestors().skip(1) {
        match probe(p) {
            Ok(true) => return Some(p.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return None,
        }
    }
    None
}

#[derive(Clone, Copy)]
pub(crate) enum Nav {
    Pending,
    Complete,
    Failed,
}

/// A recovery through one pane's navigation events. Returns the missing folder once its parent has opened.
pub(crate) fn rec_step(rec: &mut Option<(String, String)>, ev: Nav, path: &str) -> Option<String> {
    let hit = rec.as_ref().is_some_and(|(_, t)| t.eq_ignore_ascii_case(path));
    match ev {
        Nav::Pending if !hit => *rec = None,
        Nav::Failed if hit => *rec = None,
        Nav::Complete => return rec.take().filter(|_| hit).map(|(m, _)| m),
        _ => {}
    }
    None
}

/// A local folder that is confirmed missing opens its nearest existing parent.
/// The status line says so once that parent has opened. A share returns as soon as the
/// off-thread resolve has started; this thread does not stat it or walk its parents.
pub(crate) unsafe fn go_near(i: usize, path: &str) -> bool {
    let Err(e) = try_nav(i, path) else { return true };
    if !gone(e.code()) {
        return false;
    }
    let cleared = app(|a| {
        if i >= a.s.paths.len() {
            return false;
        }
        a.s.recent.retain(|r| !r.eq_ignore_ascii_case(path));
        let hit = a.s.paths[i].eq_ignore_ascii_case(path);
        if hit {
            a.s.paths[i].clear();
        }
        hit
    });
    if cleared {
        tabs_invalidate();
    }
    let root = path.get(..3).unwrap_or("");
    let local = root.len() == 3 && root.as_bytes()[1] == b':' && matches!(GetDriveTypeW(&HSTRING::from(root)), 2 | 3);
    if !local {
        return false;
    }
    let Some(up) = near_dir(path, |p| std::fs::metadata(p).map(|m| m.is_dir())) else { return false };
    let Some(eb) = eb_of(i) else { return false };
    let mut pidl = std::ptr::null_mut();
    if SHParseDisplayName(&HSTRING::from(up.as_str()), None, &mut pidl, 0, None).is_err() || pidl.is_null() {
        if !pidl.is_null() {
            CoTaskMemFree(Some(pidl as _));
        }
        return false;
    }
    let target = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let arm = |set: bool| {
        app(|a| {
            let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) else { return };
            if set {
                p.recover = Some((path.to_string(), target.clone()));
            } else if p.recover.as_ref().is_some_and(|(_, t)| *t == target) {
                p.recover = None;
            }
        })
    };
    arm(true);
    let ok = browse_id(&eb, pidl, SBSP_ABSOLUTE).is_ok();
    if !ok {
        arm(false);
    }
    CoTaskMemFree(Some(pidl as _));
    ok
}

pub(crate) fn gone_note(a: &mut App, missing: &str, opened: &str) {
    let live = a.lost > 0 && a.note.as_ref().is_some_and(|(_, t)| Instant::now() < *t);
    a.lost = if live { a.lost + 1 } else { 1 };
    let n = a.lost;
    let text = match (ui(), n) {
        (_, 1) if ui() == 1 => format!("없는 폴더: {missing} · {opened} 엶"),
        (_, 1) if ui() == 2 => format!("見つかりません: {missing} · {opened} を開きました"),
        (_, 1) => format!("Not found: {missing} · opened {opened}"),
        (1, n) => format!("저장된 폴더 {n}개가 없어 가장 가까운 상위 폴더를 엶"),
        (2, n) => format!("保存されたフォルダー {n} 件が見つからず、最も近い上位フォルダーを開きました"),
        (_, n) => format!("{n} saved folders were missing · opened the nearest parent folders"),
    };
    a.note = Some((text, Instant::now() + std::time::Duration::from_secs(8)));
}

pub(crate) unsafe fn rec_event(i: usize, ev: Nav, pidl: *const ITEMIDLIST) {
    if pidl.is_null() {
        return;
    }
    if app(|a| a.panes.get(i).and_then(|p| p.as_ref()).is_some_and(|p| p.recover.is_some())) {
        let path = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
        app(|a| {
            if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                rec_step(&mut p.recover, ev, &path);
            }
        });
    }
}

pub(crate) unsafe fn browse(i: usize, flags: u32) {
    let history = flags == SBSP_NAVIGATEBACK || flags == SBSP_NAVIGATEFORWARD;
    if folder_step(net_failed(i), history) == FolderStep::Stay {
        leave_net_fail(i);
        return;
    }
    if let Some(h) = pane_hwnd(i) {
        end_share(i, h);
    }
    if let Some(eb) = eb_of(i) {
        _ = browse_id(&eb, std::ptr::null_mut(), flags);
    }
}

pub(crate) unsafe fn browse_item(i: usize, si: &IShellItem) {
    let path = shell_parse(si);
    // A deleted folder still sitting in the tree must not be handed to the shell view.
    if local_path_gone(&path) {
        _ = go_near(i, &path);
        return;
    }
    if let Some(h) = pane_hwnd(i) {
        end_share(i, h);
    }
    let Some(eb) = eb_of(i) else { return };
    if let Ok(pidl) = SHGetIDListFromObject(si) {
        if !pidl.is_null() {
            _ = browse_id(&eb, pidl, SBSP_ABSOLUTE);
            CoTaskMemFree(Some(pidl as _));
        }
    }
}

/// Folder a `.lnk` points at. `None` for a file shortcut, or a link that does not resolve.
/// No `Resolve`: that can search the network and show UI. The stored target is enough.
pub(crate) unsafe fn folder_link_target(path: &str) -> Option<String> {
    if !crate::folderapp::link_file(path) {
        return None;
    }
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
    link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(path), STGM_READ).ok()?;
    if let Ok(pidl) = link.GetIDList() {
        if !pidl.is_null() {
            let item = SHCreateItemFromIDList::<IShellItem>(pidl).ok();
            let folder = item.as_ref().and_then(|it| it.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM | SFGAO_BROWSABLE).ok()).is_some_and(place_flags);
            // A file shortcut stays None so the verb still opens it. The parsing name
            // covers a library or a zip; GetPath covers a plain directory.
            let name = folder.then(|| {
                item.as_ref()
                    .and_then(|it| it.GetDisplayName(SIGDN_DESKTOPABSOLUTEPARSING).ok().map(|p| pwstr(p)))
                    .filter(|s| !s.is_empty())
                    .or_else(|| stored_link_path(&link))
            }).flatten();
            CoTaskMemFree(Some(pidl as _));
            return name.filter(|s| !s.is_empty());
        }
    }
    // No pidl. A local directory is still a folder. A share is not stat'd on this thread.
    let s = stored_link_path(&link)?;
    if share_path(&s) || std::fs::metadata(&s).is_ok_and(|m| m.is_dir()) {
        Some(s)
    } else {
        None
    }
}

unsafe fn stored_link_path(link: &IShellLinkW) -> Option<String> {
    let mut buf = [0u16; 1024];
    if link.GetPath(&mut buf, std::ptr::null_mut(), 0).is_err() || buf[0] == 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..end]);
    (!s.is_empty()).then_some(s)
}

fn place_flags(flags: SFGAO_FLAGS) -> bool {
    crate::folderapp::navigable(flags.contains(SFGAO_FOLDER), flags.contains(SFGAO_STREAM), flags.contains(SFGAO_BROWSABLE))
}

pub(crate) unsafe fn focus_view(i: usize) {
    if let Some(sv) = view_of::<IShellView>(i) {
        _ = sv.UIActivate(SVUIA_ACTIVATE_FOCUS.0 as u32);
        if let Ok(h) = sv.GetWindow() {
            _ = SetFocus(Some(h));
        }
    }
}

/// Remember which of this window's controls had focus as it goes to the back.
pub(crate) unsafe fn keep_focus(main: HWND) {
    let f = GetFocus();
    if IsChild(main, f).as_bool() {
        app(|a| a.focus_keep = f);
    }
}

/// Put focus back on the address, filter, list, folder tree, or favourites it left.
/// The edits keep their text and caret.
pub(crate) unsafe fn restore_focus() {
    let (main, i, k, panel, th, side) = app(|a| (a.main, a.s.active, a.focus_keep, a.s.tree_on, a.tree_hwnd, a.side_hwnd));
    let shown = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)).is_some_and(|p| IsWindowVisible(p).as_bool());
    let within = |root: HWND| !root.0.is_null() && (root == k || IsChild(root, k).as_bool());
    let ok = IsWindow(Some(k)).as_bool()
        && ((shown && (pane_ctl(i, ID_EDIT) == Some(k) || pane_ctl(i, ID_FILTER) == Some(k)))
            || view_of::<IShellView>(i).and_then(|sv| sv.GetWindow().ok()).is_some_and(within)
            || (panel && IsWindowVisible(th).as_bool() && within(th))
            || (panel && IsWindowVisible(side).as_bool() && within(side)));
    if ok {
        if pane_ctl(i, ID_EDIT) == Some(k) {
            _ = ShowWindow(k, SW_SHOWNA);
        }
        _ = SetFocus(Some(k));
    } else {
        focus_view(i);
    }
    if GetFocus().is_invalid() {
        _ = SetFocus(Some(main));
    }
}

pub(crate) unsafe fn focus_ctl(i: usize, id: i32) -> bool {
    if let Some(e) = pane_ctl(i, id) {
        _ = SetFocus(Some(e));
        SendMessageW(e, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
    }
    true
}

// Address bar input as (a path, a program and its arguments); either may be missing. Unpaired quotes: nothing.
pub(crate) fn addr_parts(raw: &str) -> Option<(Option<&str>, Option<(&str, &str)>)> {
    let quotes = raw.matches('"').count();
    if quotes % 2 == 1 {
        return None;
    }
    let path = match quotes {
        0 => Some(raw),
        2 if raw.len() > 1 && raw.starts_with('"') && raw.ends_with('"') => Some(&raw[1..raw.len() - 1]),
        _ => None,
    };
    let cmd = if let Some(q) = raw.strip_prefix('"') {
        let (prog, rest) = q.split_once('"')?;
        (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| (prog, rest.trim()))
    } else {
        let (prog, rest) = raw.split_once(char::is_whitespace).unwrap_or((raw, ""));
        (!prog.contains(['\\', '/', ':', '"'])).then(|| (prog, rest.trim()))
    };
    Some((path, cmd.filter(|(prog, _)| !prog.is_empty())))
}

pub(crate) fn is_rel(p: &str) -> bool {
    !p.contains(':') && !p.starts_with(['\\', '/'])
}

pub(crate) unsafe fn addr_reset(i: usize, e: HWND) {
    if net_failed(i) {
        leave_net_fail(i);
        return;
    }
    let disp = app(|x| x.panes[i].as_ref().map(|p| p.disp.clone())).unwrap_or_default();
    _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
    focus_view(i);
    show_path_end(e, &disp);
    if let Ok(p) = GetParent(e) {
        sync_addr(p);
    }
}

// Enter in the address bar: a folder (relative to this pane's) is browsed to, a file opened, else a command
// (cmd, wt, powershell ...) runs in this pane's folder.
pub(crate) unsafe fn go_edit(i: usize) {
    let Some(e) = pane_ctl(i, ID_EDIT) else { return };
    let raw = text_of(e);
    let Some((path, cmd)) = addr_parts(raw.trim()) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    // A zip or a shell place is not a directory. metadata on a share blocks for the
    // redirector's timeout, so a UNC or mapped drive is not stat'd on this thread.
    let cur_raw = app(|a| a.s.paths[i].clone());
    let cur_share = share_path(&cur_raw);
    let is_dir = (is_fs(&cur_raw) && !cur_share).then(|| std::fs::metadata(&cur_raw).is_ok_and(|m| m.is_dir()));
    let cur = edit_cur_ok(is_fs(&cur_raw), cur_share, is_dir).then_some(cur_raw);
    // ShellExecute's working directory. A share is left out here: a dead one would freeze the window.
    // The share is passed only after an off-thread resolve has already returned.
    let dir = cur.as_deref().filter(|_| !cur_share).map(HSTRING::from);
    let run = |mut sei: SHELLEXECUTEINFOW| {
        sei.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.lpDirectory = dir.as_ref().map_or(PCWSTR::null(), |d| PCWSTR(d.as_ptr()));
        sei.nShow = SW_SHOWNORMAL.0;
        ShellExecuteExW(&mut sei).is_ok()
    };
    // Some(true) = browsed, Some(false) = opened or ran
    let mut resolved = false;
    let done = 'go: {
        if let Some(p) = path {
            // std::path::absolute is GetFullPathNameW: folds `..` and `.`
            let full = cur.as_ref().filter(|_| is_rel(p)).and_then(|c| std::path::absolute(Path::new(c).join(p)).ok());
            let full_owned = full.as_ref().map(|f| f.to_string_lossy().into_owned());
            let full_share = match full_owned.as_deref() {
                Some(f) => share_path(f),
                None => false,
            };
            let exists = full.as_ref().is_some_and(|f| !full_share && std::fs::metadata(f).is_ok());
            let mut target = edit_target(p, full_owned.as_deref(), full_share, exists).to_string();
            // A shortcut to a folder would otherwise ShellExecute, and that verb is us.
            if crate::folderapp::shown_on() && crate::folderapp::link_file(&target) {
                if let Some(dir) = folder_link_target(&target) {
                    target = dir;
                }
            }
            if share_path(&target) {
                // A bare `cmd` while this pane is on a share still runs if the name is not a file there.
                let bare = is_rel(p) && !p.contains(['\\', '/']);
                let run = cmd.filter(|_| bare).map(|(prog, args)| {
                    let wt = prog.eq_ignore_ascii_case("wt") || prog.eq_ignore_ascii_case("wt.exe");
                    let args = if wt && args.is_empty() && cur.is_some() && !cur_share { "-d ." } else { args };
                    (prog.to_string(), args.to_string())
                });
                let cwd = cur.clone();
                if start_share(i, &target, true, run, cwd, cur_share || full_share) {
                    resolved = true;
                    break 'go Some(true);
                }
                resolved = true;
                break 'go None;
            }
            if let Some(h) = pane_hwnd(i) {
                end_share(i, h);
            }
            // Same rule as try_nav: a missing local path never becomes a null or dead PIDL.
            if is_fs(&target) && local_path_gone(&target) {
                let ok = go_near(i, &target);
                resolved = true;
                break 'go ok.then_some(true);
            }
            let (mut pidl, mut sfgao) = (std::ptr::null_mut(), 0u32);
            if SHParseDisplayName(&HSTRING::from(&target), None, &mut pidl, SFGAO_FOLDER.0, Some(&mut sfgao)).is_ok() && !pidl.is_null() {
                let r = if sfgao & SFGAO_FOLDER.0 != 0 {
                    eb_of(i).is_some_and(|eb| browse_id(&eb, pidl, SBSP_ABSOLUTE).is_ok()).then_some(true)
                } else {
                    run(SHELLEXECUTEINFOW { fMask: SEE_MASK_IDLIST | SEE_MASK_FLAG_NO_UI, lpIDList: pidl as _, ..Default::default() }).then_some(false)
                };
                CoTaskMemFree(Some(pidl as _));
                resolved = true;
                break 'go r;
            }
            if !pidl.is_null() {
                CoTaskMemFree(Some(pidl as _));
            }
        }
        let Some((prog, args)) = cmd else { break 'go None };
        // wt opens in its own default folder unless told; lpDirectory makes `.` this pane's
        let wt = prog.eq_ignore_ascii_case("wt") || prog.eq_ignore_ascii_case("wt.exe");
        let args = if wt && args.is_empty() && cur.is_some() { "-d ." } else { args };
        let (prog, args) = (HSTRING::from(prog), HSTRING::from(args));
        run(SHELLEXECUTEINFOW { fMask: SEE_MASK_FLAG_NO_UI, lpFile: PCWSTR(prog.as_ptr()), lpParameters: PCWSTR(args.as_ptr()), ..Default::default() }).then_some(false)
    };
    match done {
        // A share that failed before this returns keeps the address: Enter is how the path is retried.
        Some(true) if !net_failed(i) => focus_view(i),
        Some(true) => {}
        Some(false) => addr_reset(i, e),
        None if addr_jumps(done, resolved) => {
            let picked = app(|a| {
                let mut favs = fav_folders(&a.s.tree);
                if favs.is_empty() {
                    favs = a.s.favs.clone();
                }
                jump_pick(&raw, &a.s.recent, &favs, &a.s.paths[i]).map(str::to_string)
            });
            match picked {
                Some(p) if go_near(i, &p) => {
                    if !net_failed(i) {
                        focus_view(i);
                    }
                }
                Some(_) => {
                    _ = MessageBeep(MB_ICONWARNING);
                    focus_view(i);
                }
                None => _ = MessageBeep(MB_ICONWARNING),
            }
        }
        None => _ = MessageBeep(MB_ICONWARNING),
    }
}

// The selected folder (or, with none selected, the folder on display) of `src` opens in `dst`.
pub(crate) unsafe fn open_in(src: usize, dst: usize) {
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

pub(crate) unsafe fn open_in_menu(h: HWND, src: usize) {
    // (pane, shown name, folder it held when the menu opened, colour label)
    let items = app(|a| {
        let live = a.panes.iter().enumerate().take(a.s.panes());
        let live = live.filter(|(k, _)| *k != src);
        live.filter_map(|(k, p)| Some((k, p.as_ref()?.disp.clone(), a.s.paths[k].clone(), a.s.tints.get(k).copied().unwrap_or(0)))).collect::<Vec<_>>()
    });
    let label = |k: usize, t: u8, disp: &str| {
        let tag = if t == 0 { String::new() } else { format!(" ({})", tint_name(t)) };
        HSTRING::from(format!("{} {}{tag}    {disp}", tr("Pane", "패널", "ペイン"), k + 1))
    };
    let arr = view_of::<IFolderView2>(src).and_then(|fv| fv.GetSelection(false).ok());
    let count = arr.as_ref().and_then(|a| a.GetCount().ok()).unwrap_or(0);
    let picked = collect_all(count, |k| unsafe { arr.as_ref()?.GetItemAt(k).ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)) });
    let Ok(menu) = CreatePopupMenu() else { return };
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Open the selected folder (or this one) in:", "선택한 폴더(없으면 현재 폴더)를 열 패널:", "選択したフォルダー（なければ現在のフォルダー）を開くペイン:")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for (k, disp, _, t) in &items {
        _ = AppendMenuW(menu, MF_STRING, k + 1, &label(*k, *t, disp));
    }
    if let Ok(sub) = CreatePopupMenu() {
        for (k, disp, _, t) in &items {
            _ = AppendMenuW(sub, MF_STRING, 301 + k, &label(*k, *t, disp));
        }
        let flags = if items.is_empty() { MF_POPUP | MF_GRAYED } else { MF_POPUP };
        _ = AppendMenuW(
            menu,
            flags,
            sub.0 as usize,
            &HSTRING::from(tr("Swap places with\tCtrl+Alt+Shift+arrows", "자리 바꾸기\tCtrl+Alt+Shift+방향키", "入れ替え\tCtrl+Alt+Shift+方向キー")),
        );
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    match &picked {
        Ok(srcs) => {
            let n = srcs.len();
            let titles = [
                (false, 101, tr("Copy {n} selected items to", "선택한 {n}개 복사", "選択した {n} 個をコピー").replace("{n}", &n.to_string())),
                (true, 201, tr("Move {n} selected items to", "선택한 {n}개 이동", "選択した {n} 個を移動").replace("{n}", &n.to_string())),
            ];
            let dropm = app(|a| a.s.dropm);
            for (mv, base, title) in titles {
                if mv && dropm == 1 {
                    continue; // always copy: the move submenu stays hidden
                }
                let Ok(sub) = CreatePopupMenu() else { continue };
                for (k, disp, path, t) in &items {
                    let ok = is_fs(path) && send_mode(srcs, path, mv).is_some();
                    let flags = if ok { MF_STRING } else { MF_STRING | MF_GRAYED };
                    _ = AppendMenuW(sub, flags, base + k, &label(*k, *t, disp));
                }
                _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, &HSTRING::from(title));
            }
        }
        Err(why) => {
            let text = match why {
                Pick::TooMany => tr("Copy / move: more than 10,000 selected items — use Ctrl+C / Ctrl+V", "복사·이동: 선택이 10,000개를 넘습니다 — Ctrl+C / Ctrl+V를 쓰세요", "コピー / 移動: 選択が 10,000 個を超えています — Ctrl+C / Ctrl+V を使ってください"),
                Pick::Unreadable => tr("Copy / move: some selected items aren't regular files", "복사·이동: 일반 파일이 아닌 항목이 섞여 있습니다", "コピー / 移動: 通常のファイルでない項目が含まれています"),
                Pick::Empty => tr("Copy / move: select files first", "복사·이동: 먼저 파일을 고르세요", "コピー / 移動: 先にファイルを選択してください"),
            };
            _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(text));
        }
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(src, ID_OPEN_IN) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    match (pick as usize, picked) {
        (0, _) => {}
        (p @ 1..=100, _) => open_in(src, p - 1),
        (p @ 301..=316, _) => swap_panes(src, p - 301),
        (p, Ok(srcs)) => {
            // only to the folder the menu showed, and only the whole selection it counted
            let (mv, k) = (p > 200, (p - 1) % 100);
            let shown = items.iter().find(|(i, ..)| *i == k).map(|(_, _, path, _)| path.clone());
            let sent = shown.filter(|s| *s == app(|a| a.s.paths[k].clone())).and_then(|dst| {
                let rename = send_mode(&srcs, &dst, mv)?;
                Some(start_copy(srcs, dst, rename, mv))
            });
            if sent != Some(true) {
                _ = MessageBeep(MB_ICONWARNING);
            }
        }
        _ => {}
    }
}

/// Right-click on a pane header: this pane's Explorer view, then a colour label.
const VIEW_CMD: usize = 210;
const SAVE_VIEW_CMD: usize = 218;

pub(crate) unsafe fn tint_menu(h: HWND, i: usize, at: POINT) {
    let cur = app(|a| a.s.tints.get(i).copied().unwrap_or(0));
    let live = pane_mode_icon(i);
    let checked = live.and_then(|(m, z)| explorer_view_index(m, z));
    let Ok(menu) = CreatePopupMenu() else { return };
    for (k, &(_, _, en, ko, ja)) in EXPLORER_VIEWS.iter().enumerate() {
        _ = AppendMenuW(menu, MF_STRING, VIEW_CMD + k, &HSTRING::from(tr(en, ko, ja)));
    }
    if let Some(k) = checked {
        _ = CheckMenuRadioItem(menu, VIEW_CMD as u32, (VIEW_CMD + EXPLORER_VIEWS.len() - 1) as u32, (VIEW_CMD + k) as u32, MF_BYCOMMAND.0);
    }
    let save_flags = if checked.is_some() { MF_STRING } else { MF_STRING | MF_GRAYED };
    _ = AppendMenuW(menu, save_flags, SAVE_VIEW_CMD, &HSTRING::from(tr("Save as default view", "기본 보기 방식으로 저장", "既定の表示として保存")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("Pane colour", "패널 색", "ペインの色")));
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    for k in 0..=6u8 {
        let flags = if k == cur { MF_STRING | MF_CHECKED } else { MF_STRING };
        let name = if k == 0 { tr("None", "없음", "なし") } else { tint_name(k) };
        _ = AppendMenuW(menu, flags, k as usize + 1, &HSTRING::from(name));
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0 as usize;
    _ = DestroyMenu(menu);
    if (VIEW_CMD..VIEW_CMD + EXPLORER_VIEWS.len()).contains(&pick) {
        let (mode, icon, ..) = EXPLORER_VIEWS[pick - VIEW_CMD];
        commit_pane_view(i, mode, icon);
        return;
    }
    if pick == SAVE_VIEW_CMD {
        save_pane_default_view(i);
        return;
    }
    if pick > 0 && pick <= 7 && i < MAXP {
        app(|a| {
            if i < a.s.tints.len() {
                a.s.tints[i] = pick as u8 - 1;
            }
        });
        repaint_header(h);
        check_dirty();
        note_save();
    }
}

unsafe fn pane_mode_icon(i: usize) -> Option<(i32, i32)> {
    let fv = view_of::<IFolderView2>(i)?;
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    fv.GetViewModeAndIconSize(&mut m, &mut z).ok()?;
    (m.0 != 0).then_some((m.0, z))
}

/// The menu's view. With per-folder memory on, the folder keeps this mode and size
/// (columns and grouping already stored for it stay). With memory off, the pane keeps it.
pub(crate) unsafe fn commit_pane_view(i: usize, mode: i32, icon: i32) {
    let Some(fv) = view_of::<IFolderView2>(i) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if fv.SetViewModeAndIconSize(FOLDERVIEWMODE(mode), icon).is_err() {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    let (remember, path) = app(|a| (a.s.remember_views, a.s.paths.get(i).cloned().unwrap_or_default()));
    let key = remember.then(|| crate::folderview::normalize_folder_key(&path)).flatten();
    let Some(key) = key else {
        // Memory off: the pane keeps its columns. A new Details pane with none yet gets Name, Date modified, Size.
        if mode == 4 && is_fs(&path) && app(|a| a.s.cols.get(i).is_none_or(|c| c.is_empty())) {
            arm_builtin_columns(i, &fv);
        }
        app(|a| {
            if i < a.s.views.len() {
                a.s.views[i] = (mode, icon);
            }
        });
        check_dirty();
        note_save();
        return;
    };
    let mut snap = app(|a| {
        a.panes.get(i).and_then(|p| p.as_ref()).and_then(|p| p.view_watch.as_ref()).filter(|w| w.key == key).map(|w| w.applied.clone())
    })
    .unwrap_or(crate::folderview::ViewSnap { mode, icon, columns: String::new(), group: None });
    snap.mode = mode;
    snap.icon = icon;
    // Switching to Details with no column list must not snapshot the shell's Type column into memory.
    let fill_default = snap.columns.is_empty() && mode == 4 && is_fs(&path);
    if fill_default {
        arm_builtin_columns(i, &fv);
    }
    if snap.columns.is_empty() || snap.group.is_none() {
        if let Some(live) = read_snap(i) {
            if snap.columns.is_empty() && !fill_default {
                snap.columns = live.columns;
            }
            if snap.group.is_none() {
                snap.group = live.group;
            }
        }
    }
    if crate::folderview::remember(&key, snap.clone()) {
        note_folder_views();
    }
    let mut applied = snap.clone();
    if fill_default {
        applied.columns = crate::folderview::default_details_columns();
    }
    let quiet = crate::folderview::watch_quiet(&applied.columns);
    app(|a| {
        let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) else { return };
        match p.view_watch.as_mut().filter(|w| w.key == key) {
            Some(w) => {
                w.applied = applied;
                if fill_default {
                    w.baseline = None;
                } else if let Some(b) = w.baseline.as_mut() {
                    b.mode = mode;
                    b.icon = icon;
                }
                w.quiet = quiet;
            }
            None => {
                p.view_watch = Some(crate::folderview::ViewWatch { key, applied, baseline: None, quiet });
            }
        }
    });
}

/// Put Name, Date modified, Size on the view and keep putting them back while the shell rebuilds.
unsafe fn arm_builtin_columns(i: usize, fv: &IFolderView2) {
    let text = crate::folderview::default_details_columns();
    let lv = view_list(fv);
    free_min_widths(lv);
    apply_cols(fv, &text, lv);
    let h = app(|a| {
        let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) else { return None };
        p.settle = Some((text, 4));
        Some(p.hwnd)
    });
    if let Some(h) = h {
        SetTimer(Some(h), TIMER_SETTLE, 50, None);
    }
}

/// The pane's current view becomes "Default view for a new pane", saved the same way the settings window saves it.
pub(crate) unsafe fn save_pane_default_view(i: usize) {
    let Some((mode, icon)) = pane_mode_icon(i).and_then(|(m, z)| explorer_view_index(m, z).map(|k| (EXPLORER_VIEWS[k].0, EXPLORER_VIEWS[k].1))) else {
        _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    app(|a| {
        a.s.view_new = mode;
        a.s.view_icon = stored_view_icon(mode, icon);
    });
    save_session();
    crate::settings::refresh();
}

// Alt+↓ or right-click on ← / →: folders recently visited in any pane, opened in this one.
pub(crate) unsafe fn recent_menu(i: usize) {
    let Some(h) = app(|a| a.panes[i].as_ref().map(|p| p.hwnd)) else { return };
    let items = app(|a| recent_items(&a.s.recent, &a.s.paths[i]));
    let Ok(menu) = CreatePopupMenu() else { return };
    if items.is_empty() {
        _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, &HSTRING::from(tr("No recent folders yet", "최근 폴더가 아직 없습니다", "最近のフォルダーはまだありません")));
    }
    for (k, p) in items.iter().enumerate() {
        // "name<tab>parent"; drive roots and share roots show whole
        let path = std::path::Path::new(p);
        let (leaf, parent) = match (path.file_name(), path.parent()) {
            (Some(n), Some(d)) => (n.to_string_lossy().into_owned(), d.display().to_string()),
            _ => (p.clone(), String::new()),
        };
        _ = AppendMenuW(menu, MF_STRING, k + 1, &HSTRING::from(format!("{leaf}\t{parent}").replace('&', "&&")));
    }
    let mut rc = RECT::default();
    if let Some(b) = pane_ctl(i, ID_BACK) {
        _ = GetWindowRect(b, &mut rc);
    }
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, rc.left, rc.bottom, None, h, None).0;
    _ = DestroyMenu(menu);
    if pick > 0 && !navigate(i, &items[pick as usize - 1]) {
        _ = MessageBeep(MB_ICONWARNING); // gone or unreachable
    }
    // An immediate share failure leaves the address selected. The list is still the last good folder.
    if !net_failed(i) {
        focus_view(i);
    }
}

/// Filter icon and Alt+↓ while the filter has focus. Pictures, video, music, documents, archives, programs.
/// "Same type as the selected file" only when exactly one file is selected.
pub(crate) const FILE_KINDS: [(&str, &str, &str, &str); 6] = [
    ("Pictures", "사진", "画像", "*.jpg;*.jpeg;*.png;*.gif;*.bmp;*.webp;*.heic;*.tif;*.tiff;*.svg;*.ico"),
    ("Videos", "동영상", "動画", "*.mp4;*.mkv;*.avi;*.mov;*.wmv;*.webm;*.m4v"),
    ("Music", "음악", "音楽", "*.mp3;*.flac;*.wav;*.m4a;*.aac;*.ogg;*.wma"),
    ("Documents", "문서", "文書", "*.pdf;*.doc;*.docx;*.xls;*.xlsx;*.ppt;*.pptx;*.hwp;*.hwpx;*.txt;*.md;*.csv;*.rtf;*.odt"),
    ("Archives", "압축", "アーカイブ", "*.zip;*.7z;*.rar;*.tar;*.gz;*.iso"),
    ("Programs", "프로그램", "プログラム", "*.exe;*.msi;*.bat;*.cmd;*.ps1"),
];

pub(crate) unsafe fn kind_menu(i: usize) {
    let (Some(h), Some(f)) = (app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.hwnd)), pane_ctl(i, ID_FILTER)) else { return };
    let cur = filter_of(i);
    let arr = view_of::<IFolderView2>(i).and_then(|fv| fv.GetSelection(false).ok()).filter(|a| a.GetCount().is_ok_and(|n| n == 1));
    let same = arr.and_then(|a| {
        let it = a.GetItemAt(0).ok()?;
        let dir = it.GetAttributes(SFGAO_FOLDER).ok()?.0 & SFGAO_FOLDER.0 != 0;
        same_type(&pwstr(it.GetDisplayName(SIGDN_PARENTRELATIVEPARSING).ok()?), dir)
    });
    let Ok(menu) = CreatePopupMenu() else { return };
    for (k, (en, ko, ja, spec)) in FILE_KINDS.iter().enumerate() {
        let flags = if cur == *spec { MF_STRING | MF_CHECKED } else { MF_STRING };
        let shown = spec.split(';').take(2).collect::<Vec<_>>().join(", ");
        _ = AppendMenuW(menu, flags, k + 1, &HSTRING::from(format!("{}\t{shown}, …", tr(en, ko, ja))));
    }
    _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    let same_label = tr("Same type as the selected file", "선택한 파일과 같은 형식", "選択したファイルと同じ種類");
    match &same {
        Some(s) => _ = AppendMenuW(menu, MF_STRING, 100, &HSTRING::from(format!("{same_label} ({})", s.replace('&', "&&")))),
        None => _ = AppendMenuW(menu, MF_STRING | MF_GRAYED, 100, &HSTRING::from(same_label)),
    }
    let clear = if cur.is_empty() && text_of(f).is_empty() { MF_STRING | MF_GRAYED } else { MF_STRING };
    _ = AppendMenuW(menu, clear, 101, &HSTRING::from(tr("Clear filter", "필터 지우기", "フィルターをクリア")));
    let mut rc = RECT::default();
    _ = GetClientRect(h, &mut rc);
    let pg = pane_geo_of(h, rc.right, rc.bottom, app(|a| a.dpi));
    let mut at = POINT { x: pg.filt.left, y: pg.filt.bottom };
    _ = ClientToScreen(h, &mut at);
    let pick = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, at.x, at.y, None, h, None).0 as usize;
    _ = DestroyMenu(menu);
    let spec = match pick {
        1..=6 => Some(FILE_KINDS[pick - 1].3.to_string()),
        100 => same,
        101 => Some(String::new()),
        _ => None,
    };
    if let Some(s) = spec {
        _ = SetWindowTextW(f, &HSTRING::from(s));
        apply_filter(i);
    }
    focus_view(i);
}

pub(crate) unsafe fn apply_filter(i: usize) {
    let Some(f) = pane_ctl(i, ID_FILTER) else { return };
    let spec = filter_spec(&text_of(f));
    let changed = FILTERS.lock().map(|mut fl| std::mem::replace(&mut fl[i], spec.clone()) != spec).unwrap_or(false);
    if let Some(sv) = view_of::<IShellView>(i) {
        if let Ok(h) = sv.GetWindow() {
            note_filter_view(h, &spec);
        }
    }
    if let (true, Some(sv)) = (changed, view_of::<IShellView>(i)) {
        _ = sv.Refresh(); // re-enumerates, asking IncludeObject again
    }
    if let (true, Some(pane)) = (changed, app(|a| a.panes[i].as_ref().map(|p| p.hwnd))) {
        _ = InvalidateRect(Some(pane), None, false); // filter badge on/off
    }
    if changed {
        note_save();
    }
}

thread_local! {
    pub(crate) static KEEP_VIEWS: Cell<bool> = const { Cell::new(false) };
}

// View mode and columns (order, widths, sort). With per-folder memory off, they stick to the pane:
// read them before leaving a folder, put them on the next one. Shell places keep their own columns.
// With memory on, a change is stored for the folder instead, and the pane's session view is left
// alone so a tab switch does not look like an unsaved layout edit.
pub(crate) unsafe fn save_view(i: usize) {
    if KEEP_VIEWS.with(|c| c.get()) {
        return;
    }
    if app(|a| a.panes[i].as_ref().is_none_or(|p| p.pending.is_some() || p.settle.is_some())) {
        return; // columns are about to be put on, or still settling: don't overwrite them
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let lv = view_list(&fv);
    free_min_widths(lv); // a list made after the folder change gets it here
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    let mode_ok = fv.GetViewModeAndIconSize(&mut m, &mut z).is_ok();
    let cols = read_cols(&fv, lv);
    let group = read_group(&fv);
    let remember = app(|a| a.s.remember_views);
    if !remember {
        if mode_ok {
            app(|a| a.s.views[i] = (m.0, z));
        }
        if let Some(c) = cols.filter(|_| app(|a| is_fs(&a.s.paths[i]))) {
            app(|a| a.s.cols[i] = c);
        }
        return;
    }
    if !mode_ok {
        return;
    }
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    let Some(key) = crate::folderview::normalize_folder_key(&path) else { return };
    let Some(mut watch) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).and_then(|p| p.view_watch.clone())) else {
        return;
    };
    if watch.key != key {
        return;
    }
    let mut live = crate::folderview::ViewSnap { mode: m.0, icon: z, columns: cols.unwrap_or_default(), group };
    // The built-in Name / Date modified / Size set is not a user edit. If the shell put Type back, try again.
    if crate::folderview::settle_builtin_columns(&mut watch, &live.columns) {
        if let Some(fv) = view_of::<IFolderView2>(i) {
            let lv = view_list(&fv);
            free_min_widths(lv);
            apply_cols(&fv, &watch.applied.columns, lv);
        }
        app(|a| {
            if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                p.view_watch = Some(watch);
            }
        });
        return;
    }
    live.columns = crate::folderview::align_columns(&watch.applied.columns, &live.columns);
    let save = crate::folderview::observe(&mut watch, &live);
    app(|a| {
        if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
            p.view_watch = Some(watch);
        }
    });
    if save && crate::folderview::remember(&key, live) {
        note_folder_views();
    }
}

/// Coalesce folder-view edits into one write of folder-views.json.
pub(crate) unsafe fn note_folder_views() {
    let main = app(|a| a.main);
    if main.0.is_null() {
        crate::folderview::flush();
        return;
    }
    SetTimer(Some(main), TIMER_FVIEWS, FVIEW_SAVE_MS, None);
}

/// Last chance before the window goes away: the debounce timer may not have fired.
pub(crate) unsafe fn flush_folder_views() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        app(|a| {
            if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                p.settle = None;
                if let Some(w) = p.view_watch.as_mut() {
                    w.quiet = 0;
                }
            }
        });
        save_view(i);
    }
    crate::folderview::flush();
}

/// Open folders start watching for edits. What is already on screen is the baseline, not a new memory.
pub(crate) unsafe fn arm_folder_watches() {
    let n = app(|a| a.s.panes());
    for i in 0..n {
        let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
        let Some(key) = crate::folderview::normalize_folder_key(&path) else { continue };
        let Some(live) = read_snap(i) else { continue };
        app(|a| {
            if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                p.view_watch = Some(crate::folderview::ViewWatch {
                    key,
                    applied: live.clone(),
                    baseline: Some(live),
                    quiet: 0,
                });
            }
        });
    }
}

pub(crate) unsafe fn set_default_folder_view() {
    let i = app(|a| a.s.active);
    let Some(snap) = read_snap(i) else {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    if snap.mode == 0 && snap.columns.is_empty() {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    }
    crate::folderview::set_default(snap);
    crate::folderview::flush();
}

pub(crate) unsafe fn reset_active_folder_view() {
    let i = app(|a| a.s.active);
    let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    let Some(key) = crate::folderview::normalize_folder_key(&path) else {
        let _ = MessageBeep(MB_ICONWARNING);
        return;
    };
    crate::folderview::forget(&path);
    if let Some(snap) = crate::folderview::default_view() {
        apply_snap_to_pane(i, &snap, &key);
    } else if let Some(live) = read_snap(i) {
        // Nothing to put back. Keep what is on screen, and do not write it as a new memory.
        app(|a| {
            if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                p.view_watch = Some(crate::folderview::ViewWatch {
                    key,
                    applied: live.clone(),
                    baseline: Some(live),
                    quiet: 0,
                });
            }
        });
    }
    crate::folderview::flush();
}

pub(crate) unsafe fn clear_remembered_folder_views() {
    crate::folderview::clear_folders();
    let n = app(|a| a.s.panes());
    for i in 0..n {
        let path = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
        let Some(key) = crate::folderview::normalize_folder_key(&path) else { continue };
        let Some(live) = read_snap(i) else { continue };
        app(|a| {
            if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
                p.view_watch = Some(crate::folderview::ViewWatch {
                    key,
                    applied: live.clone(),
                    baseline: Some(live),
                    quiet: 0,
                });
            }
        });
    }
    crate::folderview::flush();
}

unsafe fn read_snap(i: usize) -> Option<crate::folderview::ViewSnap> {
    let fv = view_of::<IFolderView2>(i)?;
    let (mut m, mut z) = (FOLDERVIEWMODE(0), 0);
    fv.GetViewModeAndIconSize(&mut m, &mut z).ok()?;
    let columns = read_cols(&fv, view_list(&fv)).unwrap_or_default();
    Some(crate::folderview::ViewSnap { mode: m.0, icon: z, columns, group: read_group(&fv) })
}

unsafe fn apply_snap_to_pane(i: usize, snap: &crate::folderview::ViewSnap, key: &str) {
    let mut snap = snap.clone();
    if snap.columns.is_empty() && (snap.mode == 0 || snap.mode == 4) && is_fs(key) {
        snap.columns = crate::folderview::default_details_columns();
    }
    let Some(fv) = view_of::<IFolderView2>(i) else { return };
    let lv = view_list(&fv);
    free_min_widths(lv);
    if snap.mode != 0 {
        let _ = fv.SetViewModeAndIconSize(FOLDERVIEWMODE(snap.mode), snap.icon);
    }
    if !snap.columns.is_empty() {
        apply_cols(&fv, &snap.columns, lv);
    }
    if let Some(g) = &snap.group {
        apply_group(&fv, g);
    }
    let h = app(|a| {
        if let Some(p) = a.panes.get_mut(i).and_then(|p| p.as_mut()) {
            if !snap.columns.is_empty() {
                p.settle = Some((snap.columns.clone(), 4));
            }
            p.view_watch = Some(crate::folderview::ViewWatch {
                key: key.to_string(),
                applied: snap.clone(),
                baseline: None,
                quiet: crate::folderview::watch_quiet(&snap.columns),
            });
            return Some(p.hwnd);
        }
        None
    });
    if let Some(h) = h {
        if !snap.columns.is_empty() {
            SetTimer(Some(h), TIMER_SETTLE, 50, None);
        }
    }
}

unsafe fn read_group(fv: &IFolderView2) -> Option<String> {
    let mut key = PROPERTYKEY::default();
    let mut asc = BOOL(0);
    if fv.GetGroupBy(&mut key, Some(&mut asc)).is_err() {
        return None;
    }
    if key.pid == 0 && key.fmtid == GUID::zeroed() {
        return Some(String::new());
    }
    Some(format!("{:?}/{}={}", key.fmtid, key.pid, if asc.as_bool() { 1 } else { -1 }))
}

unsafe fn apply_group(fv: &IFolderView2, group: &str) {
    if group.is_empty() {
        let null = PROPERTYKEY::default();
        let _ = fv.SetGroupBy(&null, false);
        return;
    }
    let (_, sort) = parse_cols(&format!(";{group}"));
    if let Some((key, dir)) = sort.into_iter().next() {
        let _ = fv.SetGroupBy(&key, dir >= 0);
    }
}

// Column names in on-screen order, from the pane's list header: dragging a column to a new place
// changes only the header's order, which the view's own column list doesn't follow.
pub(crate) unsafe fn header_names(lv: HWND) -> Vec<String> {
    let n = list_titles(lv).len();
    let mut order = vec![0i32; n];
    SendMessageW(lv, LVM_GETCOLUMNORDERARRAY, Some(WPARAM(n)), Some(LPARAM(order.as_mut_ptr() as isize)));
    let titles = list_titles(lv);
    order.iter().filter_map(|&k| titles.get(k as usize).cloned()).collect()
}

// Column titles of a list, by column index.
pub(crate) unsafe fn list_titles(lv: HWND) -> Vec<String> {
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
pub(crate) unsafe fn view_list(fv: &IFolderView2) -> HWND {
    let view = fv.cast::<IShellView>().and_then(|v| v.GetWindow()).unwrap_or_default();
    FindWindowExW(Some(view), None, w!("SysListView32"), PCWSTR::null()).unwrap_or_default()
}

// Explorer puts a floor under some columns (Size stops at 90 px) so numbers never get cut, and puts it
// back whenever a column changes (through an interface, not messages we could catch). In a pane the
// user decides: the floor is lowered to a sliver right before each resize, a drag on the header or a
// width set on the list.
pub(crate) unsafe fn free_min_widths(lv: HWND) {
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

pub(crate) unsafe fn lower_floors(lv: HWND) {
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

pub(crate) unsafe extern "system" fn list_floor_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _: usize, _: usize) -> LRESULT {
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

pub(crate) const WM_NARROW_AGAIN: u32 = WM_APP + 41;
pub(crate) const WM_DROP_OPEN: u32 = WM_APP + 42;
pub(crate) static NARROWING: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(-1); // column << 32 | width


// The view's visible columns with widths, put in on-screen order by name, then its sort columns.
// Widths come from the list: one narrowed below the view's floor is only there (the view still says 90).
pub(crate) unsafe fn read_cols(fv: &IFolderView2, lv: HWND) -> Option<String> {
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
pub(crate) unsafe fn put_widths(fv: &IFolderView2, text: &str, lv: HWND) {
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

pub(crate) unsafe fn apply_cols(fv: &IFolderView2, text: &str, lv: HWND) {
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
pub(crate) unsafe fn settle_cols(h: HWND) {
    _ = KillTimer(Some(h), TIMER_SETTLE);
    let Some(i) = pane_of(h) else { return };
    let Some((cols, left)) = app(|a| a.panes[i].as_ref().and_then(|p| p.settle.clone())) else { return };
    if let Some(fv) = view_of::<IFolderView2>(i) {
        let lv = view_list(&fv); // the new folder's list exists by now
        free_min_widths(lv);
        // The built-in set has to be put back whole: the shell rebuilds Name, Date modified, Type, Size.
        if crate::folderview::is_builtin_default(&cols) && !crate::folderview::same_column_keys(&cols, &read_cols(&fv, lv).unwrap_or_default()) {
            apply_cols(&fv, &cols, lv);
        } else {
            put_widths(&fv, &cols, lv);
        }
    }
    let left = left - 1;
    app(|a| {
        if let Some(p) = a.panes[i].as_mut() {
            p.settle = (left > 0).then(|| (cols, left));
        }
    });
    if left > 0 {
        SetTimer(Some(h), TIMER_SETTLE, [1000, 400, 150][left as usize - 1], None);
    } else {
        note_save();
    }
}

pub(crate) unsafe fn on_nav(i: usize, pidl: *const ITEMIDLIST) {
    // A failed browse can complete with a null PIDL. Naming it, or building an IShellItem
    // from it, is what takes the process down.
    if pidl.is_null() || app(|a| i >= a.s.paths.len() || i >= a.panes.len() || i >= a.s.views.len() || i >= a.s.cols.len()) {
        return;
    }
    let parse = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
    let disp = name(pidl, SIGDN_DESKTOPABSOLUTEEDITING);
    if let Some(missing) = app(|a| a.panes.get_mut(i).and_then(|p| p.as_mut()).and_then(|p| rec_step(&mut p.recover, Nav::Complete, &parse))) {
        let opened = short_name(&parse);
        app(|a| gone_note(a, &missing, &opened));
        update_status();
    }
    let Some((h, eb, view, cols, group, ticket)) = app(|a| {
        let fs = is_fs(&parse);
        let old = std::mem::replace(&mut a.s.paths[i], parse.clone());
        let path = a.s.paths[i].clone();
        visit(&mut a.s.recent, &old, &path);
        let remember = a.s.remember_views;
        let choice = if remember { crate::folderview::choose(&path) } else { crate::folderview::Choice::None };
        let (pane_view, own) = (a.s.views[i], a.s.cols[i].clone());
        let folder = match choice {
            crate::folderview::Choice::Saved(v) | crate::folderview::Choice::Default(v) => Some(v),
            crate::folderview::Choice::None => None,
        };
        a.panes[i].as_mut().map(|p| {
            p.disp = disp.clone();
            // The previous folder's icon goes as soon as the browse starts. A new lookup replaces it.
            (p.icon, p.ticket) = (-1, next_ticket(&ICON_TICKET));
            let pending = p.pending.take().filter(|c| !c.is_empty());
            // A saved folder view, else the user default, else this pane's own columns.
            // A filesystem folder with no remembered columns gets Name, Date modified, Size
            // (IColumnManager::SetColumns of PKEY_ItemNameDisplay, PKEY_DateModified, PKEY_Size).
            // Shell places still refuse the pane's filesystem columns when nothing was saved for them.
            let saved = folder.as_ref().map(|v| v.columns.as_str());
            let mode = folder.as_ref().map(|v| v.mode).unwrap_or(pane_view.0);
            let details = mode == 0 || mode == 4;
            let fallback = if folder.is_some() || !fs { String::new() } else { pending.or_else(|| (!own.is_empty()).then_some(own)).unwrap_or_default() };
            let cols = crate::folderview::pick_folder_columns(saved, &fallback, details, fs);
            let view = folder.as_ref().map(|v| (v.mode, v.icon)).unwrap_or(pane_view);
            let group = folder.as_ref().and_then(|v| v.group.clone());
            if remember {
                p.view_watch = crate::folderview::normalize_folder_key(&path).map(|key| {
                    let mut applied = folder.clone().unwrap_or(crate::folderview::ViewSnap {
                        mode: view.0,
                        icon: view.1,
                        columns: String::new(),
                        group: None,
                    });
                    if applied.columns.is_empty() {
                        if let Some(c) = &cols {
                            applied.columns = c.clone();
                        }
                    }
                    let quiet = crate::folderview::watch_quiet(&applied.columns);
                    crate::folderview::ViewWatch { key, applied, baseline: None, quiet }
                });
            } else {
                p.view_watch = None;
            }
            (p.hwnd, p.eb.clone(), view, cols, group, p.ticket)
        })
    }) else {
        tabs_invalidate();
        return;
    };
    if let Ok(e) = GetDlgItem(Some(h), ID_EDIT) {
        _ = SetWindowTextW(e, &HSTRING::from(disp.as_str()));
        show_path_end(e, &disp);
    }
    sync_addr(h);
    request_icon(h, ticket, pidl);
    if app(|a| a.s.pane_tab_list(i).len() > 1) {
        repaint_pane_tabs(i); // the current pane tab's name is this folder's
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
        if let Some(g) = group {
            apply_group(&fv, &g);
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
    // set_layout asked to focus a pane that was not open yet. Once, and only if nothing else took focus.
    if app(|a| a.focus_req == h && a.s.active == i && std::mem::take(&mut a.focus_req) == h) {
        let f = GetFocus();
        if f.0.is_null() || f == app(|a| a.main) {
            focus_view(i);
        }
    }
    hook_ctx(i);
    let here = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
    pane_here(i, &here, classify_ui(&here));
    hook_tips(i);
    find_retarget(i);
    if let Some(sv) = view_of::<IShellView>(i) {
        if let Ok(hw) = sv.GetWindow() {
            note_filter_view(hw, &filter_of(i));
        }
    }
    tabs_invalidate();
    note_save();
    if crate::folderview::is_dirty() {
        note_folder_views();
    }
}

#[implement(IExplorerBrowserEvents)]
pub(crate) struct Events(pub(crate) Arc<AtomicUsize>);

impl IExplorerBrowserEvents_Impl for Events_Impl {
    fn OnNavigationPending(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        // Refuse a null PIDL before the shell view binds it. History uses a null list only
        // as the argument to BrowseToIDList; the pending callback still carries the target.
        if pidl.is_null() {
            return Err(E_INVALIDARG.into());
        }
        let i = self.0.load(Ordering::SeqCst);
        pane_left(i);
        unsafe {
            // The shell is browsing. A share resolve still out for this pane is stale.
            if let Some(h) = pane_hwnd(i) {
                end_share(i, h);
            }
            let path = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
            // Cancel before explorerframe binds a folder that is already gone (back, forward,
            // the tree, session restore). A share is not stat'd here.
            if local_path_gone(&path) {
                return Err(Error::from(HRESULT::from_win32(ERROR_CANCELLED.0)));
            }
            rec_event(i, Nav::Pending, pidl);
            save_view(i);
        }
        Ok(())
    }
    fn OnViewCreated(&self, _: Ref<IShellView>) -> Result<()> {
        Ok(())
    }
    fn OnNavigationComplete(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        unsafe { on_nav(self.0.load(Ordering::SeqCst), pidl) };
        Ok(())
    }
    fn OnNavigationFailed(&self, pidl: *const ITEMIDLIST) -> Result<()> {
        NAV_GEN.fetch_add(1, Ordering::SeqCst);
        if pidl.is_null() {
            return Ok(());
        }
        let i = self.0.load(Ordering::SeqCst);
        unsafe {
            rec_event(i, Nav::Failed, pidl);
            let cur = app(|a| a.s.paths.get(i).cloned().unwrap_or_default());
            pane_here(i, &cur, classify_ui(&cur));
            let path = name(pidl, SIGDN_DESKTOPABSOLUTEPARSING);
            // The shell did not open it, so this path is not a history entry. The address
            // keeps it and the list shows the error over the folder that is still current.
            if !path.is_empty() && share_path(&path) {
                if let Some(h) = pane_hwnd(i) {
                    show_link_fail(i, h, 0, &path, false);
                }
            }
        }
        Ok(())
    }
}

/// Double-click / Enter. While Panegrid is the folder handler, a plain activation of a
/// folder, a folder shortcut, a library, or a zip-like folder browses this pane.
/// Returning false lets the view run the verb: files, and Ctrl / middle on a folder
/// (that launch is a new tab). With the setting off, the view already navigates.
unsafe fn take_default(i: usize, sv: Option<&IShellView>) -> bool {
    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
    let middle = GetKeyState(VK_MBUTTON.0 as i32) < 0;
    let registered = crate::folderapp::shown_on();
    let blank = crate::folderapp::OpenIn { registered, ctrl, middle, folder: false, stream: false, browsable: false };
    if !registered || ctrl || middle {
        return crate::folderapp::list_open(blank) == crate::folderapp::OpenWhere::Pane;
    }
    let Some(sv) = sv else { return false };
    let Ok(fv) = sv.cast::<IFolderView2>() else { return false };
    let idx = fv.GetFocusedItem().unwrap_or(-1);
    if idx < 0 {
        return false;
    }
    let Ok(item) = fv.GetItem::<IShellItem>(idx) else { return false };
    let flags = item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM | SFGAO_BROWSABLE).unwrap_or_default();
    let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok().map(|p| pwstr(p)).filter(|s| !s.is_empty()).unwrap_or_else(|| shell_parse(&item));
    // A `.lnk` is a file. Its target is what navigates. A junction is already a folder
    // (it can also carry SFGAO_LINK) and falls through to the flag check below.
    if crate::folderapp::link_file(&path) {
        let Some(dir) = folder_link_target(&path) else { return false };
        let ask = crate::folderapp::OpenIn { folder: true, ..blank };
        if crate::folderapp::list_open(ask) != crate::folderapp::OpenWhere::Pane {
            return false;
        }
        let _ = navigate(i, &dir);
        focus_view(i);
        return true;
    }
    let folder = flags.contains(SFGAO_FOLDER);
    let stream = flags.contains(SFGAO_STREAM);
    let browsable = flags.contains(SFGAO_BROWSABLE);
    let ask = crate::folderapp::OpenIn { folder, stream, browsable, ..blank };
    if crate::folderapp::list_open(ask) != crate::folderapp::OpenWhere::Pane {
        return false;
    }
    browse_item(i, &item);
    focus_view(i);
    true
}

// Site of each Explorer control. It asks the site for ICommDlgBrowser, whose IncludeObject is the name filter.
#[implement(IServiceProvider, ICommDlgBrowser)]
pub(crate) struct Host(pub(crate) Arc<AtomicUsize>);

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
    fn OnDefaultCommand(&self, sv: Ref<IShellView>) -> Result<()> {
        // S_OK: we browsed. S_FALSE: the view runs the verb (a file, or Ctrl / middle on a folder).
        if unsafe { take_default(self.0.load(Ordering::SeqCst), sv.as_ref()) } { Ok(()) } else { Err(S_FALSE.into()) }
    }
    fn OnStateChange(&self, _: Ref<IShellView>, change: u32) -> Result<()> {
        if change == CDBOSC_SELCHANGE {
            SEL_REV[self.0.load(Ordering::SeqCst)].fetch_add(1, Ordering::Relaxed); // same count, other items: the status line still updates
        }
        Ok(())
    }
    fn IncludeObject(&self, sv: Ref<IShellView>, pidl: *const ITEMIDLIST) -> Result<()> {
        // The view hwnd map is process-wide. filter_of() is only the window this thread last bound.
        let spec = unsafe {
            match sv.ok().ok().and_then(|v| v.GetWindow().ok()) {
                Some(h) => filter_for_view(h).unwrap_or_default(),
                None => filter_of(self.0.load(Ordering::SeqCst)),
            }
        };
        if spec.is_empty() {
            return Ok(());
        }
        unsafe {
            let sf: IShellFolder = sv.ok()?.cast::<IFolderView>()?.GetFolder()?;
            let mut attr = SFGAO_FOLDER.0 | SFGAO_STREAM.0;
            if sf.GetAttributesOf(&[pidl], &mut attr).is_err() {
                return Ok(()); // unreadable: keep the item
            }
            if real_folder(attr) {
                return Ok(()); // real folders stay, so a filtered pane can still be navigated
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

// A folder dropped on a pane's top bar (its fields and buttons too: OLE walks up to the pane) opens there.
// Never MOVE: a shell source deletes what it moved. LINK or COPY leaves the source alone.
pub(crate) fn drop_pick(allowed: DROPEFFECT) -> DROPEFFECT {
    [DROPEFFECT_LINK, DROPEFFECT_COPY].into_iter().find(|e| allowed.0 & e.0 != 0).unwrap_or(DROPEFFECT_NONE)
}

pub(crate) unsafe fn drop_items(d: &Ref<IDataObject>) -> Option<IShellItemArray> {
    let arr: IShellItemArray = SHCreateShellItemArrayFromDataObject(d.as_ref()?).ok()?;
    (arr.GetCount().unwrap_or(0) > 0).then_some(arr)
}

#[implement(IDropTarget)]
pub(crate) struct PaneDrop {
    pub(crate) slot: Arc<AtomicUsize>,
    pub(crate) hwnd: HWND,
    pub(crate) ok: Cell<bool>, // the data being dragged holds shell items
    pub(crate) helper: Option<IDropTargetHelper>,
}

impl PaneDrop {
    // sets the effect this drop would get, and tints the header only while it isn't NONE
    pub(crate) unsafe fn pick(&self, effect: *mut DROPEFFECT) {
        *effect = if self.ok.get() { drop_pick(*effect) } else { DROPEFFECT_NONE };
        self.hover(*effect != DROPEFFECT_NONE);
    }
    pub(crate) unsafe fn hover(&self, on: bool) {
        let h = self.hwnd;
        let changed = app(|a| {
            let was = a.drop_hover == h;
            if on {
                a.drop_hover = h;
            } else if was {
                a.drop_hover = HWND::default();
            }
            was != on
        });
        if changed {
            repaint_header(h);
        }
    }
    // the folder (or a file's folder) is queued, not opened: a drag from this pane's own list is still running
    pub(crate) unsafe fn queue(&self, d: &Ref<IDataObject>) -> bool {
        let Some(item) = drop_items(d).and_then(|a| a.GetItemAt(0).ok()) else { return false };
        let is_dir = item.GetAttributes(SFGAO_FOLDER).is_ok_and(|f| f.0 & SFGAO_FOLDER.0 != 0);
        let Some(folder) = (if is_dir { Some(item) } else { item.GetParent().ok() }) else { return false };
        _ = app(|a| a.drop_open.replace((self.hwnd, self.slot.load(Ordering::SeqCst), folder)));
        if PostMessageW(Some(self.hwnd), WM_DROP_OPEN, WPARAM(0), LPARAM(0)).is_ok() {
            return true;
        }
        _ = app(|a| a.drop_open.take());
        false
    }
}

impl IDropTarget_Impl for PaneDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.ok.set(drop_items(&d).is_some());
            self.pick(effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragEnter(self.hwnd, d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            self.pick(effect);
            if let Some(hp) = &self.helper {
                _ = hp.DragOver(&POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe {
            self.hover(false);
            if let Some(hp) = &self.helper {
                _ = hp.DragLeave();
            }
        }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, _: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            let allowed = *effect;
            *effect = DROPEFFECT_NONE;
            self.hover(false);
            let eff = drop_pick(allowed);
            if eff != DROPEFFECT_NONE && self.queue(&d) {
                *effect = eff;
            }
            if let Some(hp) = &self.helper {
                _ = hp.Drop(d.as_ref(), &POINT { x: pt.x, y: pt.y }, *effect);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod missing_tests {
    use super::{
        addr_keep, drive_root, edit_cur_ok, edit_target, fail_hit, fail_layout, fav_act, folder_step, local_hit, near_dir, net_bar_text, net_fail_buttons, net_fail_message, net_fail_text, net_wait_secs,
        net_wait_text, path_place, pidl_browse_ok, rec_step, retry_path, share_kind, unc_path, FailHit, FavAct, FolderStep, LocalHit, Nav, NetBook, NetOpen, NetWait, PathPlace, NET_WAIT_MS,
    };
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND};
    use windows::Win32::UI::Shell::{SBSP_ABSOLUTE, SBSP_NAVIGATEBACK, SBSP_NAVIGATEFORWARD, SBSP_PARENT};
    use crate::i18n::UI;
    use std::io;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::time::Instant;

    #[test]
    fn nearest_existing_parent() {
        let tree = |p: &Path| match p.to_string_lossy().replace('/', "\\").as_str() {
            r"C:\a\b" => Err(io::Error::new(io::ErrorKind::NotFound, "gone")),
            r"C:\a" => Ok(true),
            r"C:\" => Ok(true),
            _ => Err(io::Error::other("denied")),
        };
        assert_eq!(near_dir(r"C:\a\b\c", tree).as_deref(), Some(r"C:\a"));
        let denied = |p: &Path| {
            if p.ends_with("a") {
                Err(io::Error::other("denied"))
            } else {
                Err(io::Error::new(io::ErrorKind::NotFound, "x"))
            }
        };
        assert_eq!(near_dir(r"C:\a\b", denied), None);
    }

    fn open(hwnd: isize, id: usize, path: &str, run: Option<(&str, &str)>, dir: Option<&str>, dir_share: bool, at: Instant) -> NetOpen {
        NetOpen {
            hwnd,
            id,
            path: path.to_string(),
            edit: run.is_some(),
            run: run.map(|(p, a)| (p.to_string(), a.to_string())),
            dir: dir.map(str::to_string),
            dir_share,
            at,
        }
    }

    #[test]
    fn unc_and_drive_roots() {
        assert!(unc_path(r"\\10.255.255.2\x"));
        assert!(unc_path(r"\\server\share"));
        assert!(unc_path("//server/share/a"));
        assert!(unc_path(r"\\?\UNC\server\share"));
        assert!(unc_path(r"\\?\unc\Server\Share\"));
        assert!(!unc_path(r"\\?\C:\Windows"));
        assert!(!unc_path(r"\\.\PIPE\x"));
        assert!(!unc_path(r"C:\Windows"));
        assert!(!unc_path(r"C:/Windows"));
        assert!(!unc_path(r"::{20D04FE0-3AEA-1069-A2D8-08002B30309D}"));
        assert!(!unc_path(r"\server\share"));
        assert_eq!(drive_root(r"C:\a").as_deref(), Some(r"C:\"));
        assert_eq!(drive_root("d:/x").as_deref(), Some(r"D:\"));
        assert_eq!(drive_root(r"\\?\C:\Windows").as_deref(), Some(r"C:\"));
        assert_eq!(drive_root(r"\\server\share"), None);
        assert_eq!(drive_root(r"\\?\UNC\server\share"), None);
        assert!(share_kind(true, None));
        assert!(share_kind(false, Some(4)));
        assert!(!share_kind(false, Some(3)));
        assert!(!share_kind(false, Some(2)));
        assert!(!share_kind(false, None));
    }

    #[test]
    fn edit_target_skips_the_stat_on_a_share() {
        assert!(!edit_cur_ok(false, false, None));
        assert!(edit_cur_ok(true, true, None));
        assert!(edit_cur_ok(true, false, Some(true)));
        assert!(!edit_cur_ok(true, false, Some(false)));
        assert_eq!(edit_target("docs", Some(r"\\s\share\docs"), true, false), r"\\s\share\docs");
        assert_eq!(edit_target("docs", Some(r"C:\a\docs"), false, true), r"C:\a\docs");
        assert_eq!(edit_target("docs", Some(r"C:\a\docs"), false, false), "docs");
        assert_eq!(edit_target(r"\\s\x", None, false, false), r"\\s\x");
    }

    #[test]
    fn net_text_follows_the_language() {
        let prev = UI.load(Ordering::Relaxed);
        UI.store(0, Ordering::Relaxed);
        assert_eq!(NET_WAIT_MS, 8000);
        assert_eq!(net_wait_text(), "Connecting…");
        assert_eq!(net_fail_text(), "Unreachable");
        assert_eq!(net_bar_text(false), net_wait_text());
        assert_eq!(net_bar_text(true), net_fail_text());
        UI.store(1, Ordering::Relaxed);
        assert_eq!(net_wait_text(), "연결 중…");
        assert_eq!(net_fail_text(), "연결 실패");
        UI.store(2, Ordering::Relaxed);
        assert_eq!(net_wait_text(), "接続中…");
        assert_eq!(net_fail_text(), "接続失敗");
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn share_failure_keeps_the_path_and_says_why() {
        let prev = UI.load(Ordering::Relaxed);
        let path = r"\\server\share";
        assert_eq!(net_wait_secs(), 8);
        UI.store(1, Ordering::Relaxed);
        assert_eq!(net_fail_message(path, Some(8)), r"\\server\share에 연결하지 못했습니다 (시간 초과 8초)");
        assert_eq!(net_fail_message(path, None), r"\\server\share에 연결하지 못했습니다");
        assert_eq!(net_fail_buttons(), ("다시 시도", "이전 폴더로"));
        UI.store(2, Ordering::Relaxed);
        assert_eq!(net_fail_message(path, Some(8)), "\\\\server\\share に接続できませんでした（タイムアウト 8秒）");
        assert_eq!(net_fail_message(path, None), "\\\\server\\share に接続できませんでした");
        assert_eq!(net_fail_buttons(), ("再試行", "前のフォルダー"));
        UI.store(0, Ordering::Relaxed);
        assert_eq!(net_fail_message(path, Some(net_wait_secs())), r"Couldn't connect to \\server\share (timed out after 8 seconds)");
        assert_eq!(net_fail_message(path, None), r"Couldn't connect to \\server\share");
        assert_eq!(net_fail_buttons(), ("Retry", "Previous folder"));
        assert_eq!(retry_path(path, "  "), path);
        assert_eq!(retry_path(path, r"  \\server\other  "), r"\\server\other");
        assert_eq!(addr_keep(path, "", false), Some(path));
        assert_eq!(addr_keep(path, "", true), None);
        assert_eq!(addr_keep(path, path, false), None);
        // Back/Forward while the error is up stay on the folder that is already open.
        assert_eq!(folder_step(true, true), FolderStep::Stay);
        assert_eq!(folder_step(true, false), FolderStep::History);
        assert_eq!(folder_step(false, true), FolderStep::History);
        UI.store(prev, Ordering::Relaxed);
    }

    #[test]
    fn failure_buttons_sit_in_the_list_and_do_not_overlap() {
        let lay = fail_layout(480, 320, 96, 48, 96);
        assert!(lay.retry.t >= lay.message.b);
        assert!(lay.back.t == lay.retry.t && lay.back.l >= lay.retry.r);
        assert!(lay.retry.r <= 480 && lay.back.r <= 480);
        assert!(lay.retry.b <= 320 && lay.back.b <= 320);
        assert_eq!(fail_hit(&lay, lay.retry.l + 1, lay.retry.t + 1), FailHit::Retry);
        assert_eq!(fail_hit(&lay, lay.back.l + 1, lay.back.t + 1), FailHit::Back);
        assert_eq!(fail_hit(&lay, 0, 0), FailHit::None);
        assert_eq!(fail_hit(&lay, lay.retry.r, lay.retry.t), FailHit::None);
        let narrow = fail_layout(40, 20, 96, 10, 10);
        assert!(narrow.retry.t >= 0 && narrow.retry.b <= 20);
        assert!(narrow.retry.r <= 40 && narrow.back.r <= 40);
        assert!(narrow.back.l >= narrow.retry.r);
        assert_eq!(fail_hit(&narrow, -1, -1), FailHit::None);
    }

    #[test]
    fn stale_share_result_is_dropped() {
        let mut b = NetBook { slots: Vec::new() };
        let t0 = Instant::now();
        let wait = std::time::Duration::from_millis(u64::from(NET_WAIT_MS));
        let id = 7usize;
        assert!(b.begin(open(1, id, r"\\s\x", None, None, false, t0)).is_none());
        assert!(matches!(b.poll(1, id, t0, wait), NetWait::Early(_)));
        assert!(b.publish(1, id, 0x100, 0));
        assert!(matches!(b.poll(1, id, t0 + wait, wait), NetWait::Ready));
        assert!(!b.publish(1, id, 0x200, 0));
        let got = b.take(1, id).unwrap();
        assert_eq!(got.pidl, 0x100);
        assert!(b.take(1, id).is_none());
        let id2 = 8usize;
        b.begin(open(1, id2, r"\\s\y", Some(("cmd", "")), Some(r"\\s\y"), true, t0));
        assert!(!b.publish(1, id, 0x100, 0));
        match b.poll(1, id2, t0 + wait, wait) {
            NetWait::Stop(s) => {
                assert_eq!(s.path, r"\\s\y");
                assert_eq!(s.run.unwrap().0, "cmd");
                assert!(s.dir.is_none());
            }
            _ => panic!("timeout should stop the wait"),
        }
        assert!(!b.publish(1, id2, 0x300, 0));
        assert!(matches!(b.poll(1, id2, t0 + wait, wait), NetWait::Idle));
        let id3 = 9usize;
        b.begin(open(2, id3, r"Z:\a", None, Some(r"C:\"), false, t0));
        match b.force_stop(2, id3) {
            Some(s) => assert_eq!(s.dir.as_deref(), Some(r"C:\")),
            None => panic!("force stop"),
        }
        assert!(b.force_stop(2, id3).is_none());
    }

    #[test]
    fn recovery_waits_for_the_parent() {
        let mut rec = Some((r"C:\gone".into(), r"C:\".into()));
        assert!(rec_step(&mut rec, Nav::Pending, r"D:\").is_none() && rec.is_none());
        rec = Some((r"C:\gone".into(), r"C:\".into()));
        assert!(rec_step(&mut rec, Nav::Pending, r"C:\").is_none() && rec.is_some());
        assert_eq!(rec_step(&mut rec, Nav::Complete, r"C:\").as_deref(), Some(r"C:\gone"));
        assert!(rec.is_none());
        rec = Some((r"C:\gone".into(), r"C:\".into()));
        assert!(rec_step(&mut rec, Nav::Failed, r"C:\").is_none() && rec.is_none());
    }

    #[test]
    fn a_missing_local_path_is_not_opened_and_a_share_is_not_a_local_miss() {
        assert_eq!(local_hit(true, 0), LocalHit::Here);
        assert_eq!(local_hit(false, ERROR_FILE_NOT_FOUND.0), LocalHit::Gone);
        assert_eq!(local_hit(false, ERROR_PATH_NOT_FOUND.0), LocalHit::Gone);
        assert_eq!(local_hit(false, 5), LocalHit::Other); // access denied is not "missing"
        assert_eq!(path_place(false, false, true), PathPlace::Local);
        assert_eq!(path_place(true, false, true), PathPlace::Share);
        assert_eq!(path_place(false, true, true), PathPlace::Share);
        assert_eq!(path_place(false, false, false), PathPlace::Other);
        assert_eq!(fav_act(PathPlace::Local, Some(LocalHit::Gone)), FavAct::Missing);
        assert_eq!(fav_act(PathPlace::Local, Some(LocalHit::Here)), FavAct::Open);
        assert_eq!(fav_act(PathPlace::Local, Some(LocalHit::Other)), FavAct::Open);
        assert_eq!(fav_act(PathPlace::Local, None), FavAct::Open);
        // Unreachable network favourites stay; they are never classified as a local delete.
        assert_eq!(fav_act(PathPlace::Share, Some(LocalHit::Gone)), FavAct::Share);
        assert_eq!(fav_act(PathPlace::Share, None), FavAct::Share);
        assert_eq!(fav_act(PathPlace::Other, Some(LocalHit::Gone)), FavAct::Open);
    }

    #[test]
    fn a_null_pidl_is_refused_for_an_absolute_browse() {
        assert!(!pidl_browse_ok(true, SBSP_ABSOLUTE));
        assert!(!pidl_browse_ok(true, 0));
        assert!(pidl_browse_ok(true, SBSP_NAVIGATEBACK));
        assert!(pidl_browse_ok(true, SBSP_NAVIGATEFORWARD));
        assert!(pidl_browse_ok(true, SBSP_PARENT));
        assert!(pidl_browse_ok(false, SBSP_ABSOLUTE));
        assert!(pidl_browse_ok(false, 0));
    }
}

