// WinMerge (folder and file compare), reached from the gear menu, the file-list menu,
// and the folder-compare shortcut. Detection order and the command line are pure and tested.
// The registry and ShellExecute stay here and are not touched at startup.

mod menu;
mod plan;
mod run;

pub(crate) use menu::{ctx_command, gear, insert_ctx, run_folders, sync_gear};
pub(crate) use run::{forget_status, pick_exe, status_now, status_text};
