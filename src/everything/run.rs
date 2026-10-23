// WM_COPYDATA client. The reply window is created on the first query, on the thread that asks.
// FindWindow is the only probe, and it runs from that query, not at startup.

use std::cell::{Cell, RefCell};
use std::sync::Once;

use crate::imports::*;
use crate::CopyData;

use super::ipc::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) dir: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoundList {
    pub(crate) hits: Vec<Found>,
    pub(crate) more: bool,
}

pub(crate) fn running() -> bool {
    ipc_hwnd().is_some()
}

/// major.minor.revision.build. None when Everything does not answer with a version.
pub(crate) fn version(hwnd: HWND) -> Option<super::locate::Version> {
    unsafe {
        let part = |id: usize| SendMessageW(hwnd, WM_USER, Some(WPARAM(id)), Some(LPARAM(0))).0 as u32;
        super::locate::Version::from_parts(part(0), part(1), part(2), part(3))
    }
}

pub(crate) fn ipc_hwnd() -> Option<HWND> {
    for class in super::locate::window_classes() {
        let wide: Vec<u16> = class.encode_utf16().chain(std::iter::once(0)).collect();
        if let Ok(h) = unsafe { FindWindowW(PCWSTR(wide.as_ptr()), PCWSTR::null()) } {
            if !h.0.is_null() {
                return Some(h);
            }
        }
    }
    None
}

/// Query2 first (Everything 1.4.1 and later, including 1.5 on the same window class). Query W if that is refused.
/// `None` is a transport failure. An empty hit list is a real answer.
pub(crate) fn search(text: &str, max: u32) -> Option<FoundList> {
    let ipc = ipc_hwnd()?;
    let reply = reply_window()?;
    let bits = reply.0 as usize as u32;
    let cap = max as usize;
    let q2 = query2(bits, REPLY_ID, max, text);
    if let Some(bytes) = send(ipc, reply, &q2) {
        if let Some(list) = parse_list2(&bytes, cap) {
            return Some(found(list));
        }
    }
    let qw = query_w(bits, REPLY_ID, max, text);
    let bytes = send(ipc, reply, &qw)?;
    parse_list_w(&bytes, cap).map(found)
}

fn found(list: IpcList) -> FoundList {
    FoundList {
        hits: list.hits.into_iter().map(|h| Found { name: h.name, path: h.path, dir: h.dir }).collect(),
        more: list.more,
    }
}

fn send(ipc: HWND, reply: HWND, q: &IpcQuery) -> Option<Vec<u8>> {
    REPLY.with(|r| *r.borrow_mut() = None);
    let cd = CopyData { dw: q.copy_data, cb: q.bytes.len() as u32, lp: q.bytes.as_ptr() as _ };
    let sent = unsafe {
        SendMessageTimeoutW(ipc, WM_COPYDATA, WPARAM(reply.0 as usize), LPARAM(&cd as *const CopyData as isize), SMTO_ABORTIFHUNG, 2500, None)
    };
    if sent.0 == 0 {
        return None;
    }
    REPLY.with(|r| r.borrow_mut().take())
}

thread_local! {
    static REPLY: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    static IPC_HWND: Cell<isize> = const { Cell::new(0) };
}

fn reply_window() -> Option<HWND> {
    static CLASS: Once = Once::new();
    CLASS.call_once(|| unsafe {
        let hinst = GetModuleHandleW(None).unwrap_or_default();
        _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(ipc_proc),
            hInstance: hinst.into(),
            lpszClassName: w!("GridsEvIpc"),
            ..Default::default()
        });
    });
    IPC_HWND.with(|slot| {
        let cur = HWND(slot.get() as _);
        if !cur.0.is_null() && unsafe { IsWindow(Some(cur)).as_bool() } {
            return Some(cur);
        }
        let created = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("GridsEvIpc"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None).ok()?
        };
        unsafe { _ = ChangeWindowMessageFilterEx(created, WM_COPYDATA, MSGFLT_ALLOW, None); }
        slot.set(created.0 as isize);
        Some(created)
    })
}

unsafe extern "system" fn ipc_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let cd = &*(lp.0 as *const CopyData);
        if cd.dw == REPLY_ID as usize && !cd.lp.is_null() && cd.cb > 0 {
            let bytes = std::slice::from_raw_parts(cd.lp as *const u8, cd.cb as usize).to_vec();
            REPLY.with(|r| *r.borrow_mut() = Some(bytes));
            return LRESULT(1);
        }
    }
    DefWindowProcW(h, msg, wp, lp)
}
