<h1 align="center">Grids</h1>

<p align="center">A fast, lightweight multi-pane file manager for Windows.</p>

<p align="center">
  <a href="https://github.com/Samek86/grids"><img alt="Platform: Windows" src="https://img.shields.io/badge/platform-Windows-0078D4?logo=windows&logoColor=white"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/Samek86/grids"><img alt="Language: Rust" src="https://img.shields.io/badge/language-Rust-000000?logo=rust&logoColor=white"></a>
  <a href="https://github.com/Samek86/grids/commits"><img alt="Latest commit" src="https://img.shields.io/github/last-commit/Samek86/grids"></a>
</p>

<p align="center">
  English | <a href="README.ko.md">한국어</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/hero_dark.png">
    <img alt="Grids" src="docs/images/hero_light.png">
  </picture>
</p>

Every pane is a real Windows Explorer view, so right-click menus, drag and drop, thumbnails, rename, and copy progress all work as they do in Explorer. Up to 16 panes in one window, a folder tree on the left, and layouts you change with one drag.

## Features

- 🪟 **Up to 16 panes.** Column order, widths, and sort stay with the pane from folder to folder, and any column can be narrower than Explorer allows (the Size column stops at 90 px there). The status line sums selected folders in the background. F5 sums again, and shows "partial" when some items could not be read.
- 🧩 **Free layouts.** Up to 4 columns, each with 1 to 4 panes. Click Layout for an even grid, or press and drag across it to set each column. Six presets. A new tab copies the current tab's layout and split ratios.
- ↔️ **Drag to resize.** Drag a boundary between panes, the sidebar edge, or the divider between the folder tree and favourites. Double-click a boundary to even it out. Each part of the left panel keeps a minimum height.
- ⭐ **Tree and favourites.** The folder tree follows the active pane and keeps the current folder's name in view, even in a deep folder. Star a folder from the pane header, or right-click and choose Add to favourites, directly above Properties. Make groups, nest them, and drag items between them. Double-click a folder to open it here, or a file to open it with its usual app.
- 🧭 **A toolbar on every pane.** Address bar, back / forward / up, and a name filter (`*.pdf;*.txt`, or any part of a name). The address bar also takes `..`, a subfolder, a file to open, or a command such as `cmd`, `wt`, or `powershell` in that pane's folder. Open the selected folder in another pane and its columns come along, or copy / move the selection (up to 10,000). Drop a folder on a pane's top bar to open it there.
- 🔎 **Everything search.** A subfolder search asks [voidtools Everything](https://www.voidtools.com/) when it is running, and walks the disk otherwise. Nothing from Everything is loaded until you search or open Settings, and `Everything64.dll` is not used. The same box filters the current folder (`Ctrl+F`).
- 🗂️ **Tabs, drives, and sessions.** The row has window tabs, drive buttons (C: D: E: …), Layout, Save session, and Load session. The active pane's drive is underlined, buttons come and go as drives are plugged in, and a chevron lists every tab when they do not fit. Sessions save the layout, folders, and each pane's columns. Cards preview a layout: hover for a ✕ that deletes it, or right-click to rename in place (a name that's taken is refused, never overwritten). A green dot on Save shows unsaved layout changes. A deleted local folder opens the nearest folder that is still there, and the status line says which one was missing. Pane tabs hold more folders inside one pane. One folder hides the strip; two or more show it under the header.
- 👁️ **Quick look.** F3 or Space. A header shows the icon, a bold name, and the facts. Text, tables, and Office or HTML sit on a white plate. Pictures and PDF pages sit on the stage. PDF pages come from Windows' own engine, up to 50, with no reader toolbars. Text and source have line numbers, Markdown keeps its structure, CSV / TSV is a table, and ZIP and folder lists are zebra-striped. Office, HTML, Outlook mail, and fonts use Explorer's preview handlers. Without a handler, Word / Excel / PowerPoint / OpenDocument show their text or first sheet, HWP / HWPX their built-in preview, and .eml its headers and body. Anything else is a card. Documents over 10 MB are skipped. None of this loads until you open a preview, and Esc or Space closes it at once, even while it is opening.
- 🎨 **Light, dark, and pane colour.** Six selection colours on each theme. The active pane is outlined and its header tinted. In the dark theme, group titles such as "Devices and drives" are drawn in a light colour with a thin rule. New installs follow Windows. Dark theme or Light theme picks one by hand and turns following off. Right-click the empty part of a pane header, or its number, and pick one of the six colours (or none). The colour is saved with the session, separate from the tab colour, and the ⇄ menu shows its name.
- 📦 **One small exe.** About 450 KB, nothing to install. Ctrl+N opens another window in the same process, and running `grids.exe` again does too. Each window has its own tabs. Drag a tab off the strip to move it. Closing the last window saves every window for the next launch. Single-window mode keeps a single window. Win+Alt+E brings Grids forward from any app; press it again to minimize.

<p align="center">
  <a href="docs/demo.mp4"><img alt="Grids in motion" src="docs/demo.gif"></a><br>
  <a href="docs/demo.mp4">Watch the full launch video with sound</a> (35 s)
</p>

| Layout picker | Favourites | Sessions |
| --- | --- | --- |
| ![Layout picker](docs/layout-picker.png) | ![Favourites](docs/favourites.png) | ![Sessions](docs/sessions.png) |

## Install

Download `grids.exe` from [https://github.com/Samek86/grids](https://github.com/Samek86/grids), put it in any folder, and run it. One portable exe, about 450 KB. Nothing to install.

The exe is not code-signed, so Windows SmartScreen may warn the first time. Choose **More info → Run anyway**.

**Requirements:** Windows 10 (1903 or later) or Windows 11, x64.

Settings live in `%APPDATA%\grids\grids.json`. Put a file named `portable.txt` next to `grids.exe` to keep `grids.json` beside the exe instead. Named sessions stay `.grids` files in the `sessions` folder next to the exe.

## Build from source

Needs the Rust GNU toolchain (`x86_64-pc-windows-gnu`) and MSYS2's MinGW tools (`windres`, `dlltool`, `as` in `C:\msys64\mingw64\bin`).

```
build.cmd
```

This builds `target\release\grids.exe` and copies it to `build\grids.exe`. `icon.py` redraws the app icon (`app.ico`).

## Keyboard shortcuts

These are the defaults. Settings → Shortcuts can change any of them; the new key works immediately. Typing in the address bar, the filter, or a tab name is left alone. F6 still leaves those fields. Coming back from another app restores keyboard focus to the list, the address, the filter, the folder tree, or favourites. A click restores focus only when that click did not land on a control. The address and filter keep their text and caret.

| Keys | Action |
| --- | --- |
| Ctrl+T | New tab, with the current tab's layout |
| Ctrl+W | Close the tab (a locked tab stays) |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+Shift+T | Reopen the last closed tab |
| Ctrl+PageDown / Ctrl+PageUp | Next / previous pane |
| Ctrl+1 … Ctrl+9 | Go to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes) |
| Alt+Shift+← / → / ↑ / ↓ | Pane to the left / right / above / below |
| Ctrl+Alt+Shift+← / → / ↑ / ↓ | Swap the active pane with the one in that direction (folder, history, filter and columns stay; also ⇄ → Swap places with). The save dot stays until you save or swap back, including after a restart |
| Alt+← / Alt+→ | Back / forward |
| Alt+↑ | Parent folder |
| Alt+↓ | Recent folders (all panes, newest first; right-click ← / → too) |
| Ctrl+L, Alt+D, F4 | Address bar, with the current folder's icon on the left when the field is wide enough. Idle, it shows the path as crumbs (`C: › Users › Public`): click a crumb to open it, click a › to list the folders inside (the one this pane shows is bold), `…` lists the folded parents, and the current folder or an empty part of the bar edits the path. A slow folder is listed off the UI thread, and the menu is placed again just before it opens. Resizing or a DPI change cancels it. Also `..`, a subfolder name, a file to open, or a command such as `cmd`, `wt`, `powershell` (runs in this pane's folder). A fragment such as `down` opens the best recent folder, or a favourite folder from the tree |
| Ctrl+F | Search this folder (the same box still filters the list). An empty box shows “Search · Ctrl+F” until you type or focus it. A word matches anywhere in the name, `*` and `?` are wildcards, and `;` means either term; case does not matter. Matches stream in while you type, a new keystroke or Esc cancels the search. A folder walk stops at 300 and says there are more; Everything stops at 5,000. Enter or double-click opens a file (and moves the pane to the folder that holds it) or opens a folder in this pane. Subfolders, on the result list, searches below this folder. When Everything is running and Use Everything for search is on (Settings → General, on by default), that search asks Everything and the list says Everything; otherwise it walks and the list says Folders. Everything's own syntax is passed through (`ext:`, `size:`, `dm:`, `regex:`, `path:`). This folder or All drives on that list, or a query starting with `all:`, chooses this folder or every indexed drive. ZIP, 7z, RAR and CAB match by name, the same as other files. Folders stay in the filtered list. The icon, or Alt+↓ while the box has focus, opens file kinds. The shortcut can be changed in Settings → Shortcuts. A screen reader hears “Pane N address” and “Pane N search” |
| Num+ or Ctrl+Shift+A / Num− / Num* | Select, deselect, or invert files by a pattern such as `*.jpg`. The count stays on the status line for 5 seconds |
| F6 / Shift+F6 | Next / previous focus: file list, address, filter, folder tree, favourites |
| F3 / Space | Quick look at the focused file: pictures, text and code (line numbers), Markdown, CSV tables, Office / PDF / HTML / mail up to 10 MB, HWP / HWPX, ZIP contents and folders. Anything else is a card of facts. Arrows or the wheel move to the next or previous file and the preview stays open. On long text the wheel scrolls first and moves on past the end after it has rested there for 350 ms; tilting the wheel always steps. Page Up / Page Down scroll. Esc, Space or F3 closes it, including while it is still opening, and returns to the list. Holding the key opens it once. A space within a second of typing, or during IME composition, stays in the name. Settings → Panes & view can turn Space off (`space_preview`, on when missing) |
| F7 | New folder through Explorer's own command, selected and ready to rename |
| Ctrl+Alt+T | New pane tab (the current folder), while the focus is in a pane. With one tab, the button to the right of ⇄ does the same |
| Ctrl+Alt+W | Close the pane tab (the last one stays) |
| Ctrl+Alt+Page Down / Ctrl+Alt+Page Up | Next / previous pane tab |
| Enter / Esc | Apply / cancel the address or filter |
| Ctrl+H | Show hidden files |
| Ctrl+Shift+E | Show or hide the left panel (folder tree and favourites). The width comes back as it was |
| Ctrl+Shift+O | Open the folders of open Explorer windows and tabs in panes, from the active pane on (also View menu; panes already showing one of them stay put) |
| Ctrl+S | Save the current session |
| Ctrl+B | Favourites (↑/↓, Enter to open, Del to remove). With the panel or this list focused, Alt+↑/↓ moves the selected row. Dragging in the panel reorders it; letting go outside cancels |
| Ctrl+Shift+M | Maximize / restore the active pane (or double-click its header) |
| Ctrl+Shift+C | Copy the full path of the selected items (or of the current folder) |
| Ctrl+Alt+N | New text file |
| Ctrl+Shift+N | New folder |
| Ctrl+, | Settings |
| Ctrl+/ | Shortcut list, in the app's own colours. Holding the keys opens it once |
| F2, Del, Ctrl+C / X / V, F5 | Same as Explorer. Ctrl+V of a copied picture (screenshot or a browser image) saves a PNG in the folder and leaves the selection, focus and folder as they were. The status line shows the file name |

Double-clicking empty space in a pane goes up a folder (can be turned off in the view menu). A splitter under the pointer shows a thin line in the selection colour, including the left-panel edge and the divider between the folder tree and favourites.

## Settings

The gear on the tab row opens the view menu. Settings is in that menu (`Ctrl+,`).

Grids stores settings in `%APPDATA%\grids\grids.json`. A file named `portable.txt` next to `grids.exe` keeps `grids.json` beside the exe instead. Named sessions are `.grids` files in the `sessions` folder next to the exe. An existing `grids.ini` is read once and renamed to `grids.ini.bak`. An older `lang=ko` in `grids.ini` still loads as Korean.

The file is replaced atomically. A failed write is tried again in 5 seconds. A file that cannot be read is left alone (Retry or Cancel, never overwritten with defaults). Unknown keys are kept. Minimizing keeps a maximized window maximized.

Language is Auto (the Windows display language), English, 한국어, or 日本語, stored as `"lang": "auto"`, `"en"`, `"ko"`, or `"ja"`. Explorer parts inside each pane, such as column names and right-click menus, follow the Windows display language. The font defaults to that language's face (Segoe UI, Malgun Gothic, Yu Gothic UI). A chosen face applies to the whole window at once (`font_face`). There is no font size setting, and an older `font` value is ignored.

View → Show file extensions is Windows' own setting, the same one Explorer uses, and it is on the Settings page too. View → Follow Windows light / dark is the check for a new install. View → Dark theme or Light theme picks light or dark by hand and turns following off. Lists are recoloured in place, so history, the filter, the selection, the scroll position, and an address being typed stay.

| Key | Notes |
| --- | --- |
| `side_split` | Divider between the folder tree and favourites. Restored on startup. Settings can hide the whole left panel. |
| `use_everything` | On unless `"use_everything": false`. Subfolder search then asks Everything when it is running. Settings → General shows Installed and running, Installed but not running, or Not installed, plus the version when Everything answers. Start Everything launches the installed `Everything.exe`. Install runs `winget install --id voidtools.Everything -e` when winget is available, and otherwise opens the voidtools download page. |
| `ev_asked` | Remembers the answer to the one-time install prompt. The first subfolder search without Everything installed asks once. |
| `drive_buttons` | On when the key is missing. Settings → Appearance → Show drive buttons hides them and gives that space to the tabs. A narrow window hides the last letters first. |
| `layout_new` | Default layout for a first start and a new window: two panes side by side, top and bottom, 2×2, or one pane. A path handed over by another launch opens as a one-pane tab. A new window tab (+ or Ctrl+T) copies the current tab's layout instead. |
| `hotkey` | On when the line is missing. Win+Alt+E brings Grids forward; press again to minimize. If a Grids dialog is open, that dialog comes forward. View shows a check only while the key is really registered, so one click retries after a failed start. The shortcut can be changed in Settings. |
| `space_preview` | On when the key is missing. Settings → Panes & view → Preview with Space turns this off. A space within a second of typing a name, or while Korean or Japanese IME text is being composed, stays part of that name. Ctrl+Space and Shift+Space are left alone. |
| `folders` | Folders open as pane tabs, stored on each pane, and in named sessions. An older `pane_tabs` value is ignored. |
| `favorites_tree` | The favourites tree. An older flat list is moved into the root. |
| `one_window` | Single-window mode. Off when the line is missing. Another launch restores a minimized window and opens path arguments as tabs in that window. Ctrl+N, dragging a tab off the strip, Move to new window, and Win+Alt+E while Grids is already in front open a tab here instead of a window. |
| `lang` | `"auto"`, `"en"`, `"ko"`, or `"ja"`. |
| `font_face` | Font for the whole window. No font size is stored. An older `font` value is ignored. |

## Roadmap

- [ ] Code-signed builds, so SmartScreen does not warn the first time you run `grids.exe`
- [ ] A font size setting (only `font_face` is stored today)
- [ ] Interface languages beyond English, 한국어, and 日本語

## Credits

Grids is a fork of [efww/grids](https://github.com/efww/grids) by efww.

Inspired by [Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir) and [voidtools Everything](https://www.voidtools.com/). Grids is an independent project, not affiliated with Q-Dir or its author.

## License

[MIT](LICENSE)
