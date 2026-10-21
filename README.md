# Grids

A fast, lightweight multi-pane file manager for Windows. Up to 16 Explorer panes in one window, a folder tree on the left, and layouts you change with one drag. One small exe (about 450 KB), nothing to install.

[한국어](README.ko.md)

[![Grids in motion](docs/demo.gif)](docs/demo.mp4)

▶ [Watch the full launch video with sound](docs/demo.mp4) (35 s)

![Grids, light theme](docs/screenshot-light.png)

Every pane is a real Windows Explorer view, so right-click menus, drag and drop, thumbnails, rename and copy progress all work exactly as they do in Explorer.

## Features

- **Free layouts**: up to 4 columns, each with its own number of panes (1 to 4). Click the layout button for an even grid, or press and drag across it to set each column; six presets for common setups.
- **Resizable everything**: drag any boundary between panes or the sidebar; double-click a boundary to even it out.
- **Activity bar**: a narrow column at the far left switches the left panel between the folder tree and favourites. Click the active icon to collapse it. Search is reserved. The choice is remembered.
- **Folder tree** that follows the active pane and keeps the current folder's name in view, even in deep folders.
- **Per-pane toolbar**: address bar, back / forward / up, name filter (`*.pdf;*.txt` or any part of a name), open the selected folder in another pane (its columns come along, so you can set one pane up and copy it around) or copy / move the selected files to another pane's folder (up to 10,000), favourite toggle; drag a folder onto a pane's top bar to open it there.
- **Sessions**: the layout, folders and each pane's columns (order, widths, sort) are saved automatically. Save named sessions and load them from cards that preview each layout; a green dot on the save button shows unsaved layout changes.
- **Columns are a pane setting**: their order, widths and sort stay as the pane moves from folder to folder, and any column can be made narrower than Explorer allows (Size stops at 90 px there).
- **Folder favourites**: star a folder from its pane header, or right-click a folder and choose Add to favourites, directly above Properties (Remove from favourites when it is already there). The same item is on the sidebar menu. Jump to a favourite from the star list in the toolbar (Ctrl+B). Both use the same `fav=` lines in `grids.ini`. The rest of the shell menu stays.
- **Folder sizes**: the status line also shows the total size of selected folders, summed in the background (F5 sums again; "partial" when some items couldn't be read).
- **Light and dark themes** with six selection colours each; the active pane is outlined and its header tinted.
- **Windows**: Ctrl+N opens another window in the same process. Each window has its own tabs. Drag a tab off the strip to move it, or use the tab menu. Closing the last window saves every window for the next launch; closing any other window just closes that one. A second `grids.exe` opens a new window here instead of a second process.
- **Portable**: settings live in `%APPDATA%\grids\grids.json`. Put a file named `portable.txt` next to `grids.exe` to keep `grids.json` beside the exe instead. An existing `grids.ini` is read once and renamed to `grids.ini.bak`. Named sessions stay `.grids` files in the `sessions` folder next to the exe.
- **Settings**: the gear on the toolbar, or View → Settings. Language is Auto (Windows display language), English, 한국어, or 日本語, stored as `"lang": "auto"`, `"en"`, `"ko"`, or `"ja"`. An older `lang=ko` in `grids.ini` still loads as Korean. The Explorer parts inside each pane, such as column names and right-click menus, follow your Windows display language.

| Layout picker | Sessions | Favourites |
| --- | --- | --- |
| ![Layout picker](docs/layout-picker.png) | ![Sessions](docs/sessions.png) | ![Favourites](docs/favourites.png) |

![Grids, dark theme](docs/screenshot-dark.png)

## Download

Get `grids.exe` from [Releases](https://github.com/efww/grids/releases), put it in any folder and run it.

The exe is not code-signed, so Windows SmartScreen may warn the first time: choose **More info → Run anyway**.

**Requirements**: Windows 10 (1903 or later) or Windows 11, x64.

## Keyboard shortcuts

These are the defaults. Settings → Shortcuts can change any of them; the new key works immediately. Typing in the address bar, the filter, or a tab name is left alone.

| Keys | Action |
| --- | --- |
| Ctrl+T | New tab |
| Ctrl+W | Close the tab (a locked tab stays) |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous tab |
| Ctrl+Shift+T | Reopen the last closed tab |
| Ctrl+PageDown / Ctrl+PageUp | Next / previous pane |
| Ctrl+1 … Ctrl+9 | Go to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes) |
| Alt+Shift+← / → / ↑ / ↓ | Pane to the left / right / above / below |
| Alt+← / Alt+→ | Back / forward |
| Alt+↑ | Parent folder |
| Alt+↓ | Recent folders (all panes, newest first; right-click ← / → too) |
| Ctrl+L, Alt+D, F4 | Address bar: also `..` or a subfolder name, a file to open, or a command such as `cmd`, `wt`, `powershell` (runs in this pane's folder) |
| Ctrl+F | Name filter |
| Enter / Esc | Apply / cancel the address or filter |
| Ctrl+H | Show hidden files |
| Ctrl+S | Save the current session |
| Ctrl+B | Favourites list (↑/↓, Enter to open, Del to remove) |
| Ctrl+Shift+M | Maximize / restore the active pane (or double-click its header) |
| Ctrl+Shift+C | Copy the full path of the selected items (or of the current folder) |
| Ctrl+Alt+N | New text file |
| Ctrl+Shift+N | New folder |
| Ctrl+, | Settings |
| F2, Del, Ctrl+C / X / V, F5 | Same as Explorer |

Double-clicking empty space in a pane goes up a folder (can be turned off in the view menu).

## Build

Needs the Rust GNU toolchain (`x86_64-pc-windows-gnu`) and MSYS2's MinGW tools (`windres`, `dlltool`, `as` in `C:\msys64\mingw64\bin`).

```
build.cmd
```

This builds `target\release\grids.exe` and copies it to `grids.exe` in the project folder. `icon.py` redraws the app icon (`app.ico`).

## Credits

Inspired by [Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir). Grids is an independent project, not affiliated with Q-Dir or its author.

## License

[MIT](LICENSE)
