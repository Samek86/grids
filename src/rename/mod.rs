// Batch rename. A separate window, same chrome as Settings: rules stack on top,
// live preview underneath, IFileOperation when the names are clean.

mod engine;
mod exec;
mod presets;
mod win;

pub(crate) use win::{captures, menu_label, on_key, open_active, open_pane, open_paths, refresh};
