# Contributing

## Build from source

Needs the Rust GNU toolchain (`x86_64-pc-windows-gnu`) and MSYS2's MinGW tools (`windres`, `dlltool`, `as` in `C:\msys64\mingw64\bin`).

```
build.cmd
```

This builds `target\release\panegrid.exe` and copies it to `build\panegrid.exe`. The app icon is `app.ico`, from the Panegrid icon kit.

## Release

Upload that exe as a GitHub release asset named `panegrid.exe`. Do not zip it. The in-app updater downloads `panegrid.exe` and does not extract a zip. A release that only has a zip is shown to the user and left uninstalled.

