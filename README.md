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
- **Folder tree** that follows the active pane and keeps the current folder's name in view, even in deep folders.
- **Per-pane toolbar**: address bar, back / forward / up, name filter (`*.pdf;*.txt` or any part of a name), open the selected folder in another pane (its columns come along, so you can set one pane up and copy it around) or copy / move the selected files to another pane's folder (up to 10,000), favourite toggle; drag a folder onto a pane's top bar to open it there.
- **Sessions**: the layout, folders and each pane's columns (order, widths, sort) are saved automatically. Save named sessions and load them from cards that preview each layout; a green dot on the save button shows unsaved layout changes.
- **Columns are a pane setting**: their order, widths and sort stay as the pane moves from folder to folder, and any column can be made narrower than Explorer allows (Size stops at 90 px there).
- **Folder favourites**: star a folder from its pane header, jump to it from the star list in the toolbar.
- **Folder sizes**: the status line also shows the total size of selected folders, summed in the background (F5 sums again; "partial" when some items couldn't be read).
- **Light and dark themes** with six selection colours each; the active pane is outlined and its header tinted.
- **Single instance**: running `grids.exe C:\a D:\b` or opening a `.grids` session file reuses the open window.
- **Portable**: settings live in `grids.ini` next to the exe, named sessions in the `sessions` folder beside it.
- **English or Korean** (View menu). The Explorer parts inside each pane, such as column names and right-click menus, follow your Windows display language.

| Layout picker | Sessions | Favourites |
| --- | --- | --- |
| ![Layout picker](docs/layout-picker.png) | ![Sessions](docs/sessions.png) | ![Favourites](docs/favourites.png) |

![Grids, dark theme](docs/screenshot-dark.png)

## Download

Get `grids.exe` from [Releases](https://github.com/efww/grids/releases), put it in any folder and run it.

The exe is not code-signed, so Windows SmartScreen may warn the first time: choose **More info → Run anyway**.

**Requirements**: Windows 10 (1903 or later) or Windows 11, x64.

## Keyboard shortcuts

| Keys | Action |
| --- | --- |
| Ctrl+Tab / Ctrl+Shift+Tab | Next / previous pane |
| Ctrl+1 … Ctrl+9 | Go to pane 1–9 (the number at the left of each pane header; hidden in very narrow panes) |
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
