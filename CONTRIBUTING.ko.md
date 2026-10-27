# 기여하기

## 소스에서 빌드

Rust GNU 툴체인(`x86_64-pc-windows-gnu`)과 MSYS2의 MinGW 도구(`C:\msys64\mingw64\bin`의 `windres`, `dlltool`, `as`)가 필요합니다.

```
build.cmd
```

`target\release\panegrid.exe`를 빌드해 `build\panegrid.exe`로 복사합니다. 앱 아이콘은 Panegrid 아이콘 키트의 `app.ico`입니다.

## 릴리스

그 exe를 GitHub 릴리스 자산 이름 `panegrid.exe`로 올립니다. zip으로 묶지 않습니다. 앱 안 업데이트는 `panegrid.exe`를 받고 zip은 풀지 않습니다. zip만 있는 릴리스는 안내만 하고 설치하지 않습니다.

