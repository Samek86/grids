# Contributing

## Build from source

Needs the Rust GNU toolchain (`x86_64-pc-windows-gnu`) and MSYS2's MinGW tools (`windres`, `dlltool`, `as` in `C:\msys64\mingw64\bin`).

```
build.cmd
```

This builds `target\release\panegrid.exe` and copies it to `build\panegrid.exe`. The app icon is `app.ico`, from the Panegrid icon kit.

## Release

Upload two GitHub release assets, and do not zip them:

- `panegrid.exe`
- `panegrid.exe.sha256`

Always upload `panegrid.exe.sha256` with the exe. The file is one sha256sum line, `<64 hex>  panegrid.exe` (two spaces). Create it in the folder that contains the exe so the name in the file is `panegrid.exe`:

```
sha256sum panegrid.exe > panegrid.exe.sha256
```

The in-app updater downloads both files, checks the SHA-256, and replaces the exe only when they match. A release with no `panegrid.exe.sha256`, or only a zip, is shown to the user and left uninstalled.

