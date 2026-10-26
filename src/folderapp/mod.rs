//! Filesystem folders and drives can open in Panegrid. Win+E, the taskbar and
//! virtual folders stay with Explorer: only the Directory and Drive classes are
//! registered, under HKCU\Software\Classes, and a shell: or CLSID path that still
//! arrives is handed back to explorer.exe.

mod apply;
mod plan;

pub(crate) use apply::{reload_shown, repair_moved_exe, set_registered, shown_on, take_launch_args};
