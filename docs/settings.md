# Panegrid settings

Where the file lives is described under [Install](../README.md#install).

## Migration

On first run, if `panegrid.json` is missing, it is copied from a previous Panegrid or Samek86 Grids file (`%APPDATA%\grids\grids.json`, or `grids.json` next to the exe). Otherwise `grids.ini` or `qdir.ini` is imported read-only. Source files are never renamed, modified, or deleted (no `.bak`). An older `lang=ko` in `grids.ini` still loads as Korean.

## Writing, language, and view

The file is replaced atomically. A failed write is tried again in 5 seconds. A file that cannot be read is left alone (Retry or Cancel, never overwritten with defaults). Unknown keys are kept. Minimizing keeps a maximized window maximized.

The default follows the Windows display language (Korean, Japanese, or English); otherwise English. Auto, English, 한국어, or 日本語 can be chosen, stored as `"lang": "auto"`, `"en"`, `"ko"`, or `"ja"`. Explorer parts inside each pane, such as column names and right-click menus, follow the Windows display language. The font defaults to that language's face (Segoe UI, Malgun Gothic, Yu Gothic UI). A chosen face applies to the whole window at once (`font_face`). There is no font size setting, and an older `font` value is ignored.

View → Show file extensions is Windows' own setting, the same one Explorer uses, and it is on the Settings page too. View → Follow Windows light / dark is the check for a new install. View → Dark theme or Light theme picks light or dark by hand and turns following off. Lists are recoloured in place, so history, the filter, the selection, the scroll position, and an address being typed stay.

## Keys

| Key | Notes |
| --- | --- |
| `side_split` | Divider between the folder tree and favourites. Restored on startup. Settings can hide the whole left panel. |
| `use_everything` | On unless `"use_everything": false`. Subfolder search then asks Everything when it is running. Settings → General shows Installed and running, Installed but not running, or Not installed, plus the version when Everything answers. Start Everything launches the installed `Everything.exe`. Install runs `winget install --id voidtools.Everything -e` when winget is available, and otherwise opens the voidtools download page. |
| `winmerge_path` | Empty means auto-detect, and the key is omitted. Settings → General → WinMerge shows whether WinMerge was found and accepts a custom `WinMergeU.exe` or its folder. Detection looks at that path, then `HKLM`/`HKCU` `Software\Thingamahoochie\WinMerge` (`Executable`), App Paths for `WinMergeU.exe`, Program Files, and `PATH`. Compare folders with Ctrl+Alt+C, the gear menu, or the file-list menu. |
| `ev_asked` | Remembers the answer to the one-time install prompt. The first subfolder search without Everything installed asks once. |
| `drive_buttons` | On when the key is missing. Settings → Appearance → Show drive buttons hides them and gives that space to the tabs. A narrow window hides the last letters first. |
| `layout_new` | Default layout for a first start and a new window: two panes side by side, top and bottom, 2×2, or one pane. A path handed over by another launch opens as a one-pane tab. A new window tab (+ or Ctrl+T) copies the current tab's layout instead. |
| `hotkey` | On when the line is missing. Win+Shift+E brings Panegrid forward; press again to minimize. If a Panegrid dialog is open, that dialog comes forward. View shows a check only while the key is really registered, so one click retries after a failed start. The shortcut can be changed in Settings. |
| `space_preview` | On when the key is missing. Settings → Panes & view → Preview with Space turns this off. A space within a second of typing a name, or while Korean or Japanese IME text is being composed, stays part of that name. Ctrl+Space and Shift+Space are left alone. |
| `folders` | Folders open as pane tabs, stored on each pane, and in named sessions. An older `pane_tabs` value is ignored. |
| `favorites_tree` | The favourites tree. An older flat list is moved into the root. |
| `one_window` | Single-window mode. Off when the line is missing. Another launch restores a minimized window and opens path arguments as tabs in that window. Ctrl+N, dragging a tab off the strip, Move to new window, and Win+Shift+E while Panegrid is already in front open a tab here instead of a window. |
| `lang` | `"auto"`, `"en"`, `"ko"`, or `"ja"`. A missing key, like `"auto"`, follows the Windows display language (Korean, Japanese, or English); otherwise English. |
| `font_face` | Font for the whole window. No font size is stored. An older `font` value is ignored. |
