use crate::imports::*;
use super::*;
use crate::app::*;
use crate::shell::*;

// ---------- shell tree (sidebar) ----------

#[implement(INameSpaceTreeControlEvents)]
pub(crate) struct TreeEvents;

impl INameSpaceTreeControlEvents_Impl for TreeEvents_Impl {
    fn OnItemClick(&self, psi: Ref<IShellItem>, hit: u32, click: u32) -> Result<()> {
        // Re-clicking the already selected folder fires no selection change; navigate here instead.
        let on_item = hit & NSTCEHT_ONITEM.0 as u32 != 0 && click & NSTCECT_LBUTTON.0 as u32 != 0;
        if let (true, Some(si), Some(t)) = (on_item, psi.as_ref(), app(|a| a.tree.clone())) {
            if unsafe { t.GetItemState(si, NSTCIS_SELECTED.0 as u32) }.unwrap_or(0) != 0 {
                unsafe { browse_item(app(|a| a.s.active), si) };
            }
        }
        // Double-click is also the tree's open. While we are the folder handler that
        // open is our own exe, which the running window turns into a new tab.
        // S_OK: only the namespace tree handles the click (it still expands). The
        // TreeView does not, and that is what runs the verb. Ctrl and middle stay S_FALSE.
        // BUTTON is a mask: a right click is 3, so it must be compared, not tested as a bit.
        let left = click & NSTCECT_BUTTON.0 as u32 == NSTCECT_LBUTTON.0 as u32;
        let dbl = click & NSTCECT_DBLCLICK.0 as u32 != 0;
        let on_label = hit & NSTCEHT_ONITEM.0 as u32 != 0;
        if dbl && left && on_label && tree_keeps_click() {
            if let Some(si) = psi.as_ref() {
                unsafe { browse_item(app(|a| a.s.active), si) };
            }
            return Ok(());
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
    fn OnKeyboardInput(&self, msg: u32, wp: WPARAM, _: LPARAM) -> Result<()> {
        // Enter on the selected folder. S_OK would let the tree run that verb (a new tab).
        // Any other code means the key was handled. Ctrl+Enter stays S_FALSE.
        let enter = msg == WM_KEYDOWN && wp.0 as u16 == VK_RETURN.0;
        if enter && tree_keeps_click() {
            unsafe { browse_selected_tree() };
            return Err(E_FAIL.into());
        }
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
    fn OnAfterContextMenu(&self, psi: Ref<IShellItem>, pcmin: Ref<IContextMenu>, riid: *const GUID, ppv: *mut *mut core::ffi::c_void) -> Result<()> {
        let (Some(item), Some(inner)) = (psi.as_ref(), pcmin.as_ref()) else {
            return Err(E_NOTIMPL.into());
        };
        match unsafe { item_is_container(item) } {
            Some(true) => {}
            // Attributes were not ready. The tree's own Open can still launch us.
            None => {
                crate::folderapp::note_activate(app(|a| a.main.0 as isize), active_pane(), false);
                return Err(E_NOTIMPL.into());
            }
            Some(false) => return Err(E_NOTIMPL.into()),
        }
        let path = shell_parse(item);
        if path.is_empty() {
            return Err(E_NOTIMPL.into());
        }
        let menu: IContextMenu = FavCtx { inner: inner.clone(), path, cmd: Cell::new(u32::MAX), item: Cell::new(usize::MAX), popup: Cell::new(0) }.into();
        unsafe {
            *ppv = std::ptr::null_mut();
            if menu.query(riid, ppv).is_ok() && !(*ppv).is_null() {
                return Ok(());
            }
            *ppv = std::ptr::null_mut();
        }
        // keep the tree's own menu if it asked for an interface we don't wrap
        Err(E_NOTIMPL.into())
    }
    fn OnBeforeStateImageChange(&self, _: Ref<IShellItem>) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnGetDefaultIconIndex(&self, _: Ref<IShellItem>, _: *mut i32, _: *mut i32) -> Result<()> {
        Err(E_NOTIMPL.into())
    }
}

fn tree_keeps_click() -> bool {
    let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
    let middle = unsafe { GetKeyState(VK_MBUTTON.0 as i32) < 0 };
    // Ctrl and middle stay with the tree, which runs the verb. That is a new tab.
    // A plain click is recorded too, so a verb that still runs opens in this pane.
    let main = app(|a| a.main.0 as isize);
    crate::folderapp::note_activate(main, active_pane(), ctrl || middle);
    crate::folderapp::tree_activate(crate::folderapp::shown_on(), ctrl, middle, true) == crate::folderapp::OpenWhere::Pane
}

unsafe fn browse_selected_tree() {
    let Some(t) = app(|a| a.tree.clone()) else { return };
    let Ok(items) = t.GetSelectedItems() else { return };
    if let Ok(item) = items.GetItemAt(0) {
        browse_item(app(|a| a.s.active), &item);
    }
}

pub(crate) unsafe fn create_tree(main: HWND) -> Result<(INameSpaceTreeControl, HWND)> {
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
    install_tree_drop(&t, h);
    Ok((t, h))
}


