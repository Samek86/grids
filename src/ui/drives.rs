// Drive buttons on the tab row, to the left of Layout. The letter and the drive type only: a volume name can
// wait on a sleeping disk or a dead share. Device notices are posted and applied once a
// drag or resize has finished, so the system is not kept waiting on a disk call.

use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};

use crate::imports::*;
use crate::app::*;
use crate::i18n::*;
use crate::pane::child;
use super::*;

pub(crate) const ID_DRIVE: i32 = 1500; // + 0..25: A: … Z:
pub(crate) const WM_DRIVES: u32 = WM_APP + 48; // drives may have come or gone

pub(crate) fn mask_letters(mask: u32) -> Vec<u8> {
    (0..26u8).filter(|k| mask >> k & 1 != 0).map(|k| b'A' + k).collect()
}

/// Drive letter of a folder path (`d:\x` → D). A UNC path, a shell folder, or a search has none.
pub(crate) fn drive_of(path: &str) -> Option<u8> {
    match path.as_bytes() {
        [c, b':', ..] if c.is_ascii_alphabetic() => Some(c.to_ascii_uppercase()),
        _ => None,
    }
}

pub(crate) fn drive_tip(c: u8, kind: u32) -> String {
    let name = match kind {
        2 => tr(crate::Key::RemovableDrive),
        3 => tr(crate::Key::LocalDisk),
        4 => tr(crate::Key::NetworkDrive),
        5 => tr(crate::Key::CdDvdDrive),
        _ => tr(crate::Key::Drive),
    };
    format!("{name} ({}:) — {}", c as char, tr(crate::Key::OpenInActivePane))
}

/// Creates or removes the button for each letter that came or went. True when any did.
/// The caller lays them out. The first call is before the startup layout, so that layout runs once.
pub(crate) unsafe fn refresh_drives() -> bool {
    let mask = GetLogicalDrives();
    let Some((main, hover, gone, added)) = app(|a| {
        if a.main.0.is_null() {
            return None;
        }
        let old = std::mem::replace(&mut a.drive_mask, mask);
        if old == mask {
            return None;
        }
        let (keep, gone): (Vec<_>, Vec<_>) = a.drives.drain(..).partition(|d| mask >> (d.0 - b'A') & 1 != 0);
        a.drives = keep;
        Some((a.main, a.hover, gone, mask_letters(mask & !old)))
    }) else {
        return false;
    };
    for (_, _, h) in gone {
        if hover == h {
            set_hover(HWND::default());
        }
        tip_text(h, "", TTM_DELTOOLW);
        _ = DestroyWindow(h);
    }
    for c in added {
        let root = HSTRING::from(format!("{}:\\", c as char));
        let kind = GetDriveTypeW(&root);
        let name = HSTRING::from(format!("{}:", c as char));
        let h = child(main, w!("BUTTON"), PCWSTR(name.as_ptr()), BS_OWNERDRAW as u32, WINDOW_EX_STYLE(0), ID_DRIVE + (c - b'A') as i32, HFONT::default());
        if h.0.is_null() {
            continue;
        }
        add_tip(h, &drive_tip(c, kind));
        app(|a| a.drives.push((c, kind, h)));
    }
    app(|a| a.drives.sort_by_key(|d| d.0));
    true
}

/// The underline follows the active pane's drive. Called on the status tick and on a pane switch.
pub(crate) unsafe fn mark_drive() {
    let Some(hs) = app(|a| {
        let path = a.s.paths.get(a.s.active).map(|p| p.as_str()).unwrap_or("");
        let m = drive_of(path);
        (a.drive_mark != m).then(|| {
            a.drive_mark = m;
            a.drives.iter().map(|d| d.2).collect::<Vec<_>>()
        })
    }) else {
        return;
    };
    for h in hs {
        _ = InvalidateRect(Some(h), None, false);
    }
}

pub(crate) unsafe fn refresh_drive_tips() {
    for (c, kind, h) in app(|a| a.drives.clone()) {
        tip_text(h, &drive_tip(c, kind), TTM_UPDATETIPTEXTW);
    }
}

#[cfg(test)]
mod tests {
    use super::{drive_of, mask_letters};
    use crate::app::THIS_PC;

    #[test]
    fn letters_and_paths() {
        assert_eq!(mask_letters(0b1100), b"CD");
        assert_eq!(mask_letters(1 | 1 << 25), b"AZ");
        assert!(mask_letters(0).is_empty());
        assert_eq!([drive_of(r"c:\x"), drive_of(r"D:\"), drive_of("e:")], [Some(b'C'), Some(b'D'), Some(b'E')]);
        for p in [r"\\server\share", THIS_PC, "search-ms:query=x", ""] {
            assert_eq!(drive_of(p), None, "{p}");
        }
    }
}
