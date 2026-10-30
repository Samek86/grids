//! Server connections. SMB and WebDAV use Windows UNC. SFTP and FTP mount with rclone.
//! Passwords stay in Credential Manager. This module does not touch the UI thread's `app()`.

mod cred;
mod model;
mod mount;
mod obscure;
mod tools;

pub(crate) use cred::{delete_secret, read_secret, write_secret};
pub(crate) use model::*;
pub(crate) use mount::{begin_connect, install_then_connect, mount_roots, path_on_live_mount, take_done, unmount_all, unmount_link, Done, WM_REMOTE};
pub(crate) use tools::{rclone_ok, winfsp_ok};
