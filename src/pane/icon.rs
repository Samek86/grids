// The current folder's shell icon, to the left of the address field.
// Looked up off the UI thread: a slow share must not freeze the window. One request per pane.
// A late result is dropped unless its ticket is still the one the pane is waiting for.

use std::sync::atomic::AtomicU32;
use std::sync::OnceLock;

use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
use windows::Win32::System::Threading::{CreateEventW, SetEvent};
use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, MsgWaitForMultipleObjectsEx, PeekMessageW, TranslateMessage, MWMO_INPUTAVAILABLE, PM_REMOVE, QS_ALLINPUT};

use crate::imports::*;

pub(crate) const WM_PANE_ICON: u32 = WM_APP + 46; // wParam = ticket, lParam = system image-list index (-1 none)

pub(crate) static ICON_TICKET: AtomicU32 = AtomicU32::new(1);

pub(crate) fn next_ticket(t: &AtomicU32) -> u32 {
    loop {
        let n = t.fetch_add(1, Ordering::Relaxed);
        if n != 0 {
            return n;
        }
    }
}

/// A result is kept only for the lookup the pane is still waiting on.
pub(crate) fn take_icon(cur: u32, msg: u32, idx: i32) -> Option<i32> {
    (cur != 0 && cur == msg).then_some(idx)
}

/// (pane hwnd, ticket, PIDL). At most one queued lookup per pane.
pub(crate) type IconReq = (isize, u32, usize);
pub(crate) static ICON_Q: Mutex<Vec<IconReq>> = Mutex::new(Vec::new());
static ICON_WORKER: OnceLock<Option<isize>> = OnceLock::new();

pub(crate) fn icon_q() -> std::sync::MutexGuard<'static, Vec<IconReq>> {
    ICON_Q.lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn enqueue(q: &mut Vec<IconReq>, req: IconReq) -> Option<usize> {
    let old = unqueue(q, req.0);
    q.push(req);
    old
}

pub(crate) fn unqueue(q: &mut Vec<IconReq>, h: isize) -> Option<usize> {
    let k = q.iter().position(|r| r.0 == h)?;
    Some(q.remove(k).2)
}

pub(crate) unsafe fn request_icon(h: HWND, ticket: u32, pidl: *const ITEMIDLIST) {
    let c = ILClone(pidl);
    if c.is_null() {
        return;
    }
    let ev = *ICON_WORKER.get_or_init(|| {
        let Ok(ev) = CreateEventW(None, false, false, PCWSTR::null()) else { return None };
        let raw = ev.0 as isize;
        match std::thread::Builder::new().name("grids-icon".into()).spawn(move || icon_worker(raw)) {
            Ok(_) => Some(raw),
            Err(_) => {
                _ = CloseHandle(ev);
                None
            }
        }
    });
    let Some(ev) = ev else {
        ILFree(Some(c));
        return;
    };
    let old = enqueue(&mut icon_q(), (h.0 as isize, ticket, c as usize));
    if let Some(old) = old {
        ILFree(Some(old as *const ITEMIDLIST));
    }
    _ = SetEvent(HANDLE(ev as *mut _));
}

/// Lives until the process ends. One lookup at a time. It pumps messages so COM on this apartment keeps moving.
pub(crate) fn icon_worker(ev: isize) {
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
        let mut m = MSG::default();
        loop {
            while PeekMessageW(&mut m, None, 0, 0, PM_REMOVE).as_bool() {
                _ = TranslateMessage(&m);
                DispatchMessageW(&m);
            }
            let next = {
                let mut q = icon_q();
                (!q.is_empty()).then(|| q.remove(0))
            };
            let Some((h, ticket, pidl)) = next else {
                _ = MsgWaitForMultipleObjectsEx(Some(&[HANDLE(ev as *mut _)]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
                continue;
            };
            let pidl = pidl as *const ITEMIDLIST;
            let mut sfi = SHFILEINFOW::default();
            let flags = SHGFI_PIDL | SHGFI_SYSICONINDEX | SHGFI_SMALLICON;
            let found = com && SHGetFileInfoW(PCWSTR(pidl as _), FILE_FLAGS_AND_ATTRIBUTES(0), Some(&mut sfi), size_of::<SHFILEINFOW>() as u32, flags) != 0;
            ILFree(Some(pidl));
            let idx = if found { sfi.iIcon.max(-1) } else { -1 };
            _ = PostMessageW(Some(HWND(h as *mut _)), WM_PANE_ICON, WPARAM(ticket as usize), LPARAM(idx as isize));
        }
    }
}

pub(crate) unsafe fn drop_icon_request(h: HWND) {
    if let Some(old) = unqueue(&mut icon_q(), h.0 as isize) {
        ILFree(Some(old as *const ITEMIDLIST));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_ticket_matches_only_the_current_lookup() {
        assert_eq!(take_icon(6, 5, 3), None);
        assert_eq!(take_icon(0, 0, 3), None);
        assert_eq!(take_icon(7, 5, 3), None);
        assert_eq!(take_icon(6, 6, 3), Some(3));
        assert_eq!(take_icon(6, 6, -1), Some(-1));
    }

    #[test]
    fn icon_queue_keeps_one_request_per_pane() {
        let mut q = Vec::new();
        assert!(enqueue(&mut q, (1, 2, 10)).is_none());
        assert_eq!(enqueue(&mut q, (1, 3, 11)), Some(10));
        assert_eq!(q, vec![(1, 3, 11)]);
        assert_eq!(unqueue(&mut q, 1), Some(11));
        assert!(q.is_empty());
        assert!(unqueue(&mut q, 1).is_none());
    }
}
