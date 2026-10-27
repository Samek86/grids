// "Always ask" (dropm == 2) shows Copy / Move / Cancel for a file drop inside a pane,
// on the folder tree, and on the middle of a favourite folder. Other drop settings stay
// with the shell (or, for favourites, with the link they already insert).
//
// The copy or move itself is `start_copy` on a worker thread. A move also stamps
// "Performed DropEffect" on the data object before DROPEFFECT_MOVE goes back, so the
// source does not delete the files a second time. If that stamp fails, the effect
// reported back is COPY: the files were still moved, and a second delete would be worse.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL};
use windows::Win32::System::Ole::ReleaseStgMedium;
use windows::Win32::UI::Controls::{TaskDialogIndirect, TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOGCONFIG_1, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION};

use crate::imports::*;
use crate::pwstr;
use crate::app::*;
use crate::i18n::*;
use crate::pane::*;
use crate::session::*;
use super::fileop::{paste_mode, start_copy};

/// What Drop returns to the source. A move is reported as a copy when the source was not
/// told that the move already happened.
pub(crate) fn reported_effect(mv: bool, performed: bool) -> u32 {
    if mv && performed { DROPEFFECT_MOVE.0 } else { DROPEFFECT_COPY.0 }
}

/// Where a pane drop lands. `child` is set only when the row under the cursor is a local
/// directory (a share is not stat'd, so it stays `None` and the current folder is used).
pub(crate) fn drop_folder(current: &str, child: Option<&str>) -> Option<String> {
    if let Some(path) = child.filter(|p| is_fs(p)) {
        return Some((*path).to_string());
    }
    is_fs(current).then(|| current.to_string())
}

/// Middle of a favourite folder row, and only while the drop action is Always ask.
/// Edges stay an insert. Groups, files, and shell places are not copy targets.
pub(crate) fn fav_into_folder(dropm: u8, group: bool, file: bool, path: &str, y: i32, rh: i32) -> bool {
    dropm == 2 && !group && !file && is_fs(path) && rh > 0 && y >= rh / 4 && y < rh * 3 / 4
}

/// Some(true) is Move, Some(false) is Copy, None is Cancel.
pub(crate) unsafe fn ask_copy_or_move(owner: HWND) -> Option<bool> {
    let title = HSTRING::from("Panegrid");
    let head = HSTRING::from(tr(crate::Key::CopyOrMoveTheseItems));
    let copy = HSTRING::from(tr(crate::Key::Copy));
    let mv = HSTRING::from(tr(crate::Key::Move));
    let cancel = HSTRING::from(tr(crate::Key::Cancel));
    let buttons = [
        TASKDIALOG_BUTTON { nButtonID: 100, pszButtonText: PCWSTR(copy.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: 101, pszButtonText: PCWSTR(mv.as_ptr()) },
        TASKDIALOG_BUTTON { nButtonID: 2, pszButtonText: PCWSTR(cancel.as_ptr()) },
    ];
    let cfg = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        hwndParent: owner,
        hInstance: HINSTANCE::default(),
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
        dwCommonButtons: Default::default(),
        pszWindowTitle: PCWSTR(title.as_ptr()),
        Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: PCWSTR::null() },
        pszMainInstruction: PCWSTR(head.as_ptr()),
        pszContent: PCWSTR::null(),
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 100,
        cRadioButtons: 0,
        pRadioButtons: std::ptr::null(),
        nDefaultRadioButton: 0,
        pszVerificationText: PCWSTR::null(),
        pszExpandedInformation: PCWSTR::null(),
        pszExpandedControlText: PCWSTR::null(),
        pszCollapsedControlText: PCWSTR::null(),
        Anonymous2: TASKDIALOGCONFIG_1 { hFooterIcon: HICON::default() },
        pszFooter: PCWSTR::null(),
        pfCallback: None,
        lpCallbackData: 0,
        cxWidth: 0,
    };
    let mut pressed = 0i32;
    if TaskDialogIndirect(&cfg, Some(&mut pressed), None, None).is_err() {
        return None;
    }
    match pressed {
        100 => Some(false),
        101 => Some(true),
        _ => None,
    }
}

/// Tell the source the move already happened, so it does not delete the originals.
pub(crate) unsafe fn mark_move_performed(obj: &IDataObject) -> bool {
    let fmt = RegisterClipboardFormatW(w!("Performed DropEffect"));
    if fmt == 0 {
        return false;
    }
    let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, 4) else { return false };
    let p = GlobalLock(mem) as *mut u32;
    if p.is_null() {
        let _ = GlobalFree(Some(mem));
        return false;
    }
    *p = DROPEFFECT_MOVE.0;
    let _ = GlobalUnlock(mem);
    let etc = FORMATETC {
        cfFormat: fmt as u16,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 { hGlobal: mem },
        pUnkForRelease: std::mem::ManuallyDrop::new(None),
    };
    if obj.SetData(&etc, &medium, true).is_ok() {
        true
    } else {
        let _ = GlobalFree(Some(mem));
        false
    }
}

/// Filesystem paths in the drop. Empty when the drop is missing, mixed, or not files:
/// the caller then leaves the drop with the shell.
pub(crate) unsafe fn drop_fs_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let paths = hdrop_paths(d);
    let paths = if !paths.is_empty() { paths } else { shell_paths(d) };
    if paths.is_empty() || paths.iter().any(|p| !is_fs(p)) {
        Vec::new()
    } else {
        paths
    }
}

unsafe fn hdrop_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let Some(obj) = d.as_ref() else { return Vec::new() };
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let Ok(mut medium) = obj.GetData(&fmt) else { return Vec::new() };
    let drop = HDROP(medium.u.hGlobal.0);
    let mut paths = Vec::new();
    for k in 0..DragQueryFileW(drop, u32::MAX, None) {
        let need = DragQueryFileW(drop, k, None) as usize;
        let mut buf = vec![0u16; need + 1];
        let got = DragQueryFileW(drop, k, Some(&mut buf)) as usize;
        let path = String::from_utf16_lossy(buf.get(..got).unwrap_or(&[]));
        if !path.is_empty() {
            paths.push(path);
        }
    }
    ReleaseStgMedium(&mut medium);
    paths
}

unsafe fn shell_paths(d: &Ref<IDataObject>) -> Vec<String> {
    let Some(arr) = drop_items(d) else { return Vec::new() };
    let n = arr.GetCount().unwrap_or(0).min(10_000);
    let mut out = Vec::new();
    for k in 0..n {
        let Ok(it) = arr.GetItemAt(k) else { return Vec::new() };
        let Ok(name) = it.GetDisplayName(SIGDN_FILESYSPATH) else { return Vec::new() };
        let path = pwstr(name);
        if path.is_empty() {
            return Vec::new();
        }
        out.push(path);
    }
    out
}

/// Start the file operation and the effect the source should see.
pub(crate) unsafe fn run_asked(obj: &IDataObject, srcs: Vec<String>, dst: String, mv: bool) -> u32 {
    let rename = !mv && paste_mode(&srcs, &dst) == Some(true);
    if !start_copy(srcs, dst, rename, mv) {
        return DROPEFFECT_NONE.0;
    }
    let performed = mv && mark_move_performed(obj);
    reported_effect(mv, performed)
}

enum AskWhere {
    Pane(Arc<AtomicUsize>),
    Tree,
}

#[implement(IDropTarget)]
pub(crate) struct AskDrop {
    hwnd: HWND,
    inner: IDropTarget,
    where_: AskWhere,
}

impl AskDrop {
    unsafe fn forward_enter(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) {
        if let Some(obj) = d.as_ref() {
            let _ = self.inner.DragEnter(obj, key, *pt, effect);
        }
    }

    unsafe fn destination(&self, pt: &POINTL) -> Option<String> {
        bind_hwnd(self.hwnd);
        match &self.where_ {
            AskWhere::Pane(slot) => pane_destination(slot),
            AskWhere::Tree => tree_destination(pt),
        }
    }
}

unsafe fn pane_destination(slot: &AtomicUsize) -> Option<String> {
    let i = slot.load(Ordering::SeqCst);
    let cur = app(|a| a.s.paths.get(i).cloned()).unwrap_or_default();
    let child = acc_name_at_cursor().and_then(|name| join_child(&cur, &name)).filter(|p| local_directory(p));
    drop_folder(&cur, child.as_deref())
}

unsafe fn tree_destination(pt: &POINTL) -> Option<String> {
    let tree = app(|a| a.tree.clone())?;
    let at = POINT { x: pt.x, y: pt.y };
    let item = tree.HitTest(&at).ok()?;
    let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
    let path = pwstr(name);
    is_fs(&path).then_some(path)
}

/// Replace the shell view's drop target so Always ask can run first. The shell target
/// still handles the highlight and every drop that is not Always ask.
pub(crate) unsafe fn install_view_drop(hwnd: HWND, inner: IDropTarget, slot: Arc<AtomicUsize>) {
    if hwnd.0.is_null() {
        return;
    }
    let _ = RevokeDragDrop(hwnd);
    let dt: IDropTarget = AskDrop { hwnd, inner: inner.clone(), where_: AskWhere::Pane(slot) }.into();
    if RegisterDragDrop(hwnd, &dt).is_err() {
        let _ = RegisterDragDrop(hwnd, &inner);
    }
}

pub(crate) unsafe fn install_pane_drop(i: usize) {
    let Some(sv) = view_of::<IShellView>(i) else { return };
    let Ok(hwnd) = sv.GetWindow() else { return };
    let Ok(inner) = sv.cast::<IDropTarget>() else { return };
    let Some(slot) = app(|a| a.panes.get(i).and_then(|p| p.as_ref()).map(|p| p.slot.clone())) else { return };
    install_view_drop(hwnd, inner, slot);
}

pub(crate) unsafe fn install_tree_drop(tree: &INameSpaceTreeControl, hwnd: HWND) {
    let Ok(inner) = tree.cast::<IDropTarget>() else { return };
    let child = FindWindowExW(Some(hwnd), None, w!("SysTreeView32"), PCWSTR::null()).unwrap_or_default();
    let target = if child.0.is_null() { hwnd } else { child };
    let _ = RevokeDragDrop(target);
    let dt: IDropTarget = AskDrop { hwnd: target, inner: inner.clone(), where_: AskWhere::Tree }.into();
    if RegisterDragDrop(target, &dt).is_err() {
        let _ = RegisterDragDrop(target, &inner);
    }
}

impl IDropTarget_Impl for AskDrop_Impl {
    fn DragEnter(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { self.forward_enter(d, key, pt, effect) };
        Ok(())
    }
    fn DragOver(&self, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { let _ = self.inner.DragOver(key, *pt, effect); }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        unsafe { let _ = self.inner.DragLeave(); }
        Ok(())
    }
    fn Drop(&self, d: Ref<IDataObject>, key: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe {
            bind_hwnd(self.hwnd);
            let ask = app(|a| a.s.dropm) == 2;
            let srcs = if ask { drop_fs_paths(&d) } else { Vec::new() };
            let dst = if ask && !srcs.is_empty() { self.destination(pt) } else { None };
            if let (Some(obj), Some(dst)) = (d.as_ref(), dst) {
                let _ = self.inner.DragLeave();
                let owner = app(|a| a.main);
                if let Some(mv) = ask_copy_or_move(owner) {
                    if !effect.is_null() {
                        *effect = DROPEFFECT(run_asked(obj, srcs, dst, mv));
                    }
                } else if !effect.is_null() {
                    *effect = DROPEFFECT_NONE;
                }
                return Ok(());
            }
            if let Some(obj) = d.as_ref() {
                let _ = self.inner.Drop(obj, key, *pt, effect);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_folder, fav_into_folder, reported_effect};
    use windows::Win32::System::Ole::{DROPEFFECT_COPY, DROPEFFECT_MOVE};

    #[test]
    fn asked_move_is_a_copy_until_the_source_is_told() {
        assert_eq!(reported_effect(false, false), DROPEFFECT_COPY.0);
        assert_eq!(reported_effect(false, true), DROPEFFECT_COPY.0);
        assert_eq!(reported_effect(true, true), DROPEFFECT_MOVE.0);
        assert_eq!(reported_effect(true, false), DROPEFFECT_COPY.0);
    }

    #[test]
    fn pane_drop_lands_in_the_folder_under_the_cursor() {
        assert_eq!(drop_folder(r"C:\pics", Some(r"C:\pics\holiday")), Some(r"C:\pics\holiday".into()));
        assert_eq!(drop_folder(r"C:\pics", None), Some(r"C:\pics".into()));
        assert_eq!(drop_folder("::{thispc}", None), None);
        assert_eq!(drop_folder("::{thispc}", Some(r"D:\work")), Some(r"D:\work".into()));
    }

    #[test]
    fn favourite_folder_middle_is_a_copy_target_only_when_asking() {
        assert!(fav_into_folder(2, false, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, false, false, r"C:\pics", 1, 20));
        assert!(!fav_into_folder(2, false, false, r"C:\pics", 16, 20));
        assert!(!fav_into_folder(0, false, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(1, false, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, true, false, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, false, true, r"C:\pics", 10, 20));
        assert!(!fav_into_folder(2, false, false, "::{thispc}", 10, 20));
    }
}
