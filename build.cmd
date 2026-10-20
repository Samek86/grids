@rem cargo is not on PATH on this PC; msys2 supplies windres/dlltool/as for the GNU toolchain.
@set PATH=%USERPROFILE%\.cargo\bin;C:\msys64\mingw64\bin;%PATH%
@cd /d "%~dp0"
cargo build --release && copy /y target\release\qdir.exe qdir.exe
