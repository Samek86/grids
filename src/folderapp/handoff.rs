//! The running process remembers which pane was activated, and for how long a
//! Ctrl or middle-click asked for a new tab. The decision itself is `plan::handoff_action`.

use std::cell::RefCell;
use std::time::Instant;

use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

struct Note {
    main: isize,
    pane: usize,
    intent: Option<Instant>,
}

thread_local! {
    static NOTE: RefCell<Note> = const { RefCell::new(Note { main: 0, pane: 0, intent: None }) };
}

/// `new_tab` is a Ctrl or middle-click. A later plain activation does not clear it:
/// the two-second window in `handoff_action` is what lets it expire.
pub(crate) fn note_activate(main: isize, pane: usize, new_tab: bool) {
    NOTE.with(|n| {
        let mut g = n.borrow_mut();
        if main != 0 {
            g.main = main;
            g.pane = pane;
        }
        if new_tab {
            g.intent = Some(Instant::now());
        }
    });
}

/// Window and pane of the last activation. `None` before any activation in this process.
pub(crate) fn noted_pane() -> Option<(isize, usize)> {
    NOTE.with(|n| {
        let g = n.borrow();
        (g.main != 0).then_some((g.main, g.pane))
    })
}

/// Milliseconds since the last Ctrl or middle-click, if one was recorded.
pub(crate) fn intent_elapsed_ms() -> Option<u64> {
    NOTE.with(|n| {
        let t = n.borrow().intent?;
        Some(t.elapsed().as_millis().min(u128::from(u64::MAX)) as u64)
    })
}

/// Parent of this process. `NtQueryInformationProcess` is in ntdll, which is already
/// loaded; no extra Win32 feature is required. `0` when it cannot be read.
pub(crate) fn parent_pid() -> u32 {
    #[repr(C)]
    struct Basic {
        reserved1: usize,
        peb: usize,
        reserved2: [usize; 2],
        pid: usize,
        parent: usize,
    }
    type Query = unsafe extern "system" fn(isize, u32, *mut core::ffi::c_void, u32, *mut u32) -> i32;
    unsafe {
        let Ok(ntdll) = GetModuleHandleW(windows::core::w!("ntdll.dll")) else {
            return 0;
        };
        let Some(sym) = GetProcAddress(ntdll, windows::core::s!("NtQueryInformationProcess")) else {
            return 0;
        };
        let query: Query = core::mem::transmute(sym);
        let mut info = Basic { reserved1: 0, peb: 0, reserved2: [0, 0], pid: 0, parent: 0 };
        let mut len = 0u32;
        // ProcessBasicInformation is 0. GetCurrentProcess() is -1.
        let status = query(-1, 0, &mut info as *mut Basic as *mut _, core::mem::size_of::<Basic>() as u32, &mut len);
        if status < 0 {
            return 0;
        }
        info.parent as u32
    }
}
