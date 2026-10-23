// Everything (voidtools), reached only after a subfolder search or the settings page asks.
// Message bytes, the search string and install-path rules are pure and tested here.
// The IPC window, the registry and winget stay in the runtime modules and are not touched at startup.
// Everything64.dll is never loaded.

mod ipc;
mod locate;
mod query;
