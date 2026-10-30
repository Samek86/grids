<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/panegrid-lockup-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="assets/panegrid-lockup-light.png">
    <img alt="Panegrid" src="assets/panegrid-lockup-light.png" width="560">
  </picture>
</p>

<h1 align="center">Panegrid</h1>

<p align="center">A fast, lightweight multi-pane file manager for Windows.</p>

> **Panegrid is a fork of [efww/grids](https://github.com/efww/grids)** by efww. It uses its own exe name (`panegrid.exe`), window class, settings folder and hotkey (Win+Shift+E), so it can run next to the original Grids without touching its settings.

<p align="center">
  <a href="https://github.com/Samek86/panegrid"><img alt="Platform: Windows" src="https://img.shields.io/badge/platform-Windows-0078D4?logo=windows&logoColor=white"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/Samek86/panegrid"><img alt="Language: Rust" src="https://img.shields.io/badge/language-Rust-000000?logo=rust&logoColor=white"></a>
  <a href="https://github.com/Samek86/panegrid/commits"><img alt="Latest commit" src="https://img.shields.io/github/last-commit/Samek86/panegrid"></a>
</p>

<p align="center">
  English | <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/hero_dark.png">
    <img alt="Panegrid" src="docs/images/hero_light.png">
  </picture>
</p>

Every pane is a real Windows Explorer view: the full right-click menu, drag and drop, thumbnails, and Windows' own copy / move dialog all just work. Lay out up to 16 of them in one window, then save the current tab as a session.

## Features

### 🧩 Multi-pane layouts & sessions

Up to 4 columns × 4 panes. Click or drag across the layout picker, drag any boundary to resize, and drag a pane number to rearrange panes (Ctrl+drag copies a pane tab into another cell). Save the current tab — layout, folders, columns, and colour labels — as a named session. Opening one adds a new tab beside the current one.

With a single pane, the ⇄ button splits the tab: the selected folder (or the current one, if nothing or only files are selected) opens in a new right pane with the same columns and view. With two or more panes, ⇄ keeps its menu.

<table><tr><td><img alt="layout" src="docs/images/layout.png"></td><td><img alt="sessions" src="docs/images/sessions.png"></td></tr></table>

### 🗂️ Tabs that name themselves

Chrome-style window tabs plus tabs inside each pane. A tab is named after its active folder (path in the tooltip); double-click or right-click → Rename to pin your own name, lock a tab, duplicate it (Ctrl+Alt+K), move it to a new window, save it as a session, and pick a tab colour. Window tabs are wider by default (135 px), so more of the name shows.

<p align="center"><img width="560" alt="tabs" src="docs/images/tabs.png"></p>

### ⭐ Favourites popup with Explorer pins

The ★ on the toolbar opens a themed favourites popup: your Explorer Quick-access pins at the top, then your own groups, with nesting and drag to reorder. Hover a row to see its full location, and drop files or folders from a pane or Explorer onto the panel to add them as favourites. Ctrl+B opens the quick favourites list.

<p align="center"><img width="760" alt="favorites" src="docs/images/favorites.png"></p>

### 🌐 Server connections

Right-click the favourites panel background, or a group, and choose **Add server connection…**. SMB and WebDAV open as normal Windows folders (UNC, signed in with `WNetAddConnection2`). SFTP and FTP are mounted with rclone and WinFsp, then browse like any other folder — including preview, the context menu, and fast copy. Passwords stay in Windows Credential Manager and are never written to `panegrid.json`.

### 👁️ Quick preview

Space or F3 previews pictures, PDF, Office, HTML, Markdown, code, CSV, ZIP, HWP/HWPX (Hancom) and more. Ctrl+wheel zooms (10–800%), drag pans, the wheel scrolls, ←/→ step through files. Resizable, and it remembers its size.

<p align="center"><img width="760" alt="preview" src="docs/images/preview.png"></p>

### 🔎 Pane search via Everything

Ctrl+F in any pane filters as you type; tick Subfolders and it asks voidtools Everything for instant results (or walks the disk if Everything isn't running).

<p align="center"><img width="760" alt="search" src="docs/images/search.png"></p>

### 🖱️ The real Explorer, everywhere

The full Explorer context menu (shell extensions included, plus Properties on empty space), and copy / move through Windows' standard IFileOperation dialog — up to 10,000 items to another pane.

**Fast copy.** Paste and drag to or from a network folder can use robocopy (Settings → File operations, off by default). Right-click → Fast paste always uses it, with Panegrid's own progress and overwrite / skip / cancel. A move copies first and deletes the originals only after that copy succeeds.

<p align="center"><img width="760" alt="explorer" src="docs/images/explorer.png"></p>

### ⚡ Small details

- **Full path on hover.** When a pane's address bar is too narrow and shows `…`, hover it to see the whole path. Favourites rows and locked tabs have hover tooltips too.
- **Per-folder view memory.** Sort, columns, view mode, icon size and grouping come back when you open that folder again (saved in `folder-views.json` next to the settings; on by default).
- **Mouse side buttons everywhere.** Back and forward work in folder panes, also over Remote Desktop.
- **Right-click menu without the first-click lag.** Shell context menus are warmed up in the background after launch and after a long idle.
- **Theme switch keeps history.** Changing light / dark rebuilds the views without wiping back / forward or the folder you were in.
- **Optional update check.** On startup Panegrid can check GitHub Releases for a newer `panegrid.exe` (SHA-256 verified) and offer to install it.
- **Crash report on the next launch.** If a third-party shell extension takes Panegrid down, the report names it, and you can block that DLL inside Panegrid only (`blocked_shell_extensions` in `panegrid.json`).

### 🎨 Themes & accent

Light and dark that follow Windows, six accent colours per theme, per-pane colours, and a settings window in English, 한국어 and 日本語.

<p align="center"><img width="760" alt="settings" src="docs/images/settings.png"></p>

📦 **A small single exe.** Nothing to install.

## Install

Download the latest `panegrid.exe` from [Releases](https://github.com/Samek86/panegrid/releases/latest). Put it in any folder and run it. A small single exe — nothing to install. Releases ship that raw exe and a `panegrid.exe.sha256` checksum, not a zip.

The exe is not code-signed, so Windows SmartScreen may warn the first time. Choose **More info → Run anyway**.

**Requirements:** Windows 10 (1903 or later) or Windows 11, x64.

Settings live in `%APPDATA%\Panegrid\panegrid.json`. Put a file named `portable.txt` next to `panegrid.exe` to keep `panegrid.json` beside the exe instead. Named sessions stay `.grids` files in the `panegrid-sessions` folder next to the exe (existing `sessions` `.grids` files are copied once).

To build from source, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Keyboard shortcuts

Defaults; every one can be changed in Settings → Shortcuts.

| Group | Keys | Action |
| --- | --- | --- |
| **Tabs** | Ctrl+T | New tab |
|  | Ctrl+W | Close tab |
|  | Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
|  | Ctrl+Shift+T | Reopen closed tab |
|  | Ctrl+Alt+K | Duplicate tab |
|  | Double-click a tab | Rename tab |
| **Panes** | Ctrl+PgDn / Ctrl+PgUp | Next / previous pane |
|  | Ctrl+1 … 9 | Go to pane 1–9 |
|  | Alt+Shift+arrows | Move to the pane in that direction |
|  | Ctrl+Alt+Shift+arrows | Swap with the pane in that direction |
|  | Ctrl+Shift+M | Maximise / restore pane |
| **Pane tabs** | Ctrl+Alt+T / Ctrl+Alt+W | New / close pane tab |
|  | Ctrl+Alt+PgDn / PgUp | Next / previous pane tab |
| **Navigate** | Alt+← / Alt+→ | Back / forward |
|  | Alt+↑ / Alt+↓ | Parent folder / recent folders |
|  | Ctrl+L, Alt+D, F4 | Address bar |
| **View** | Ctrl+F | Search this folder (Subfolders → Everything) |
|  | Space / F3 | Quick preview |
|  | F6 / Shift+F6 | Next / previous focus |
|  | Ctrl+H | Show hidden files |
|  | Ctrl+Shift+E | Left panel |
|  | Ctrl+Shift+O | Open folders from Explorer windows |
| **Preview** | Ctrl+wheel | Zoom 10–800% |
|  | Ctrl+0 | Reset zoom |
|  | Drag | Pan a zoomed picture |
|  | Wheel, ↓, PgDn, Home / End | Scroll |
|  | ← / → | Previous / next file |
|  | Esc / Space / F3 | Close |
| **File** | Ctrl+Shift+C | Copy full path |
|  | Ctrl+Shift+R | Batch rename |
|  | Num+ or Ctrl+Shift+A / Num− / Num* | Select / deselect / invert by pattern |
|  | Ctrl+Alt+N | New text file |
|  | Ctrl+Shift+N, F7 | New folder |
|  | Ctrl+Alt+C | Compare the two panes in WinMerge |
|  | F2, Del, Ctrl+C / V, F5 | Same as Explorer. F5 reloads the active pane from the list, address bar, filter, folder tree, or favourites; a failed network folder is tried again (Ctrl+V of a copied picture saves it here as a PNG) |
| **Other** | Ctrl+B | Quick favourites list |
|  | Ctrl+S | Save this tab |
|  | Ctrl+, | Settings |
|  | F1 / Ctrl+/ | Shortcut help / list |
|  | Ctrl+N | New window |
|  | Win+Shift+E | Bring Panegrid to the front (global) |

## Known issues

**Don't run Panegrid as administrator.** A user reported that PowerToys Mouse Crosshairs (and similar tools) stopped following the cursor and froze where they clicked over Panegrid. It happens only when Panegrid runs as administrator: Windows doesn't let a program without administrator rights receive mouse input over a window that has them (UIPI), so a normal PowerToys gets no mouse movement over an elevated Panegrid. Give both the same rights, either way:

1. **Recommended**: run Panegrid normally. Don't open `panegrid.exe` with right-click → **Run as administrator**, and in the shortcut's or `panegrid.exe`'s **Properties → Compatibility**, clear **Run this program as an administrator**.
2. If you need Panegrid as administrator, run PowerToys as administrator too: **PowerToys Settings → General → Always run as administrator**, then restart PowerToys.

## Settings

The gear on the tab row opens the view menu. Settings is in that menu (`Ctrl+,`).

Language (Auto, English, and every language file), font, light / dark theme, and shortcuts are all set there. Migration from older Grids settings, how the file is written, and the full `panegrid.json` key table are in [docs/settings.md](docs/settings.md).

## Language files

English is compiled into the program and is the fallback. Korean, Japanese, French, and German ship as `lang/lang.ko.ini`, `lang/lang.ja.ini`, `lang/lang.fr.ini`, and `lang/lang.de.ini`. On first run, and again whenever the version changes, Panegrid writes those four files into the `lang` folder next to the settings file (`%APPDATA%\Panegrid\lang`, or beside `panegrid.exe` when `portable.txt` is there). It always overwrites those four. It never writes, overwrites, or deletes a `lang.xx.user.ini`, or any other `lang.xx.ini` you add.

A file is UTF-8 (a leading BOM is ignored). Lines are `key = text`. One space after `=` is the separator. `\\` is a backslash and `\n` is a newline. A line that starts with `#` or `;` is a comment. `[meta]` `name` is the label in the language list. `[meta]` `font` is an optional UI font (Korean and Japanese default to Malgun Gothic and Yu Gothic UI; other languages use Segoe UI). A missing key uses the English text.

`lang.xx.user.ini` (for example `lang.ko.user.ini`) is loaded on top of `lang.xx.ini` and wins. Add `lang.es.ini` and Spanish appears in the list. Auto matches the Windows display language when a file for that ISO code is present, and otherwise uses English. Saved values `en`, `ko`, and `ja` still work.

## Roadmap

- [ ] Code-signed builds, so SmartScreen does not warn the first time you run `panegrid.exe`
- [x] Interface languages beyond English, 한국어, and 日本語

## Credits

Panegrid is a fork of [efww/grids](https://github.com/efww/grids) by efww.

Inspired by [Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir) and [voidtools Everything](https://www.voidtools.com/). Panegrid is an independent project, not affiliated with Q-Dir or its author.

## License

[MIT](LICENSE)
