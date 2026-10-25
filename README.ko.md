<h1 align="center">Grids</h1>

<p align="center">Windows용 빠르고 가벼운 다중 패널 파일 관리자.</p>

<p align="center">
  <a href="https://github.com/Samek86/grids"><img alt="플랫폼: Windows" src="https://img.shields.io/badge/platform-Windows-0078D4?logo=windows&logoColor=white"></a>
  <a href="LICENSE"><img alt="라이선스: MIT" src="https://img.shields.io/badge/license-MIT-blue"></a>
  <a href="https://github.com/Samek86/grids"><img alt="언어: Rust" src="https://img.shields.io/badge/language-Rust-000000?logo=rust&logoColor=white"></a>
  <a href="https://github.com/Samek86/grids/commits"><img alt="최근 커밋" src="https://img.shields.io/github/last-commit/Samek86/grids"></a>
</p>

<p align="center">
  <a href="README.md">English</a> | 한국어
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/hero_dark.ko.png">
    <img alt="Grids" src="docs/images/hero_light.ko.png">
  </picture>
</p>

각 패널은 진짜 Windows 탐색기 화면입니다. 탐색기 우클릭 메뉴 전체, 끌어다 놓기, 썸네일, Windows 표준 복사·이동 창이 그대로 동작합니다. 한 창에 패널을 최대 16개 배치하고, 그대로 세션으로 저장하세요.

## 기능

### 🧩 다중 패널 레이아웃과 세션

최대 4열 × 4칸. 레이아웃 선택기를 누르거나 끌어서 배치하고, 경계를 끌어 크기를 바꿉니다. 배치·폴더·열을 이름 붙인 세션으로 저장해 한 번에 불러옵니다.

<table><tr><td><img alt="layout" src="docs/images/layout.ko.png"></td><td><img alt="sessions" src="docs/images/sessions.ko.png"></td></tr></table>

### 🗂️ 이름이 저절로 붙는 탭

크롬 스타일 창 탭과 패널 안의 탭. 탭 이름은 활성 폴더를 따라가고(경로는 툴팁), 더블클릭이나 우클릭 → 이름 바꾸기로 직접 이름을 정하고 탭 색도 고릅니다.

### ⭐ 탐색기 고정 항목까지 담은 즐겨찾기 팝업

Ctrl+B 또는 ★로 테마에 맞춘 즐겨찾기 팝업을 엽니다. 그룹, 중첩, 끌어서 순서 바꾸기, 그리고 탐색기 즐겨찾기 고정 항목까지 한곳에.

<p align="center"><img width="760" alt="favorites" src="docs/images/favorites.ko.png"></p>

### 👁️ 빠른 미리보기

스페이스나 F3으로 사진, PDF, Office, HTML, 마크다운, 코드, CSV, ZIP 등을 봅니다. Ctrl+휠 확대(10–800%), 끌어서 이동, 휠 스크롤, ←/→ 다음 파일. 창 크기를 바꾸면 기억합니다.

<p align="center"><img width="760" alt="preview" src="docs/images/preview.ko.png"></p>

### 🔎 Everything으로 패널 검색

어느 패널에서나 Ctrl+F로 입력하는 대로 걸러지고, 하위 폴더를 켜면 voidtools Everything이 즉시 찾아 줍니다(Everything이 없으면 디스크를 훑습니다).

<p align="center"><img width="760" alt="search" src="docs/images/search.ko.png"></p>

### 🖱️ 어디서나 진짜 탐색기

셸 확장까지 포함한 탐색기 우클릭 메뉴 전체(빈 곳에는 속성도), 그리고 Windows 표준 IFileOperation 창으로 다른 패널에 복사·이동(최대 10,000개).

### 🎨 테마와 강조색

Windows를 따르는 라이트·다크, 테마마다 강조색 6가지, 패널별 색, English·한국어·日本語 설정 창.

<p align="center"><img width="760" alt="settings" src="docs/images/settings.ko.png"></p>

📦 **설치 없는 exe 하나, 약 1.2 MB.**

## 설치

[Releases](https://github.com/Samek86/grids/releases/latest)에서 최신 zip을 받아 `grids.exe`를 아무 폴더에 풀고 실행합니다. 설치 없이 쓰는 exe 하나이고, 크기는 약 1.2 MB입니다.

코드 서명이 없는 exe라 처음 실행할 때 Windows SmartScreen 경고가 뜰 수 있습니다. **추가 정보 → 실행**을 누르세요.

**요구 사항:** Windows 10(1903 이상) 또는 Windows 11, x64.

설정은 `%APPDATA%\grids\grids.json`에 있습니다. `grids.exe` 옆에 `portable.txt`라는 파일을 두면 `grids.json`을 exe 옆에 둡니다. 이름 붙인 세션은 exe 옆 `sessions` 폴더의 `.grids` 파일로 남습니다.

## 소스에서 빌드

Rust GNU 툴체인(`x86_64-pc-windows-gnu`)과 MSYS2의 MinGW 도구(`C:\msys64\mingw64\bin`의 `windres`, `dlltool`, `as`)가 필요합니다.

```
build.cmd
```

`target\release\grids.exe`를 빌드해 `build\grids.exe`로 복사합니다. 앱 아이콘(`app.ico`)은 `icon.py`로 다시 그립니다.

## 단축키

기본값입니다. 설정 → 단축키에서 모두 바꿀 수 있습니다.

| 분류 | 키 | 동작 |
| --- | --- | --- |
| **탭** | Ctrl+T | 새 탭 |
|  | Ctrl+W | 탭 닫기 |
|  | Ctrl+Tab / Ctrl+Shift+Tab | 다음 / 이전 탭 |
|  | Ctrl+Shift+T | 닫은 탭 다시 열기 |
|  | 탭 더블클릭, 또는 우클릭 → 이름 바꾸기 | 탭 이름 바꾸기 (우클릭에 탭 색도 있음) |
| **패널** | Ctrl+PgDn / Ctrl+PgUp | 다음 / 이전 패널 |
|  | Ctrl+1 … 9 | 1~9번 패널로 |
|  | Alt+Shift+방향키 | 그 방향 패널로 이동 |
|  | Ctrl+Alt+Shift+방향키 | 그 방향 패널과 자리 바꾸기 |
|  | Ctrl+Shift+M | 패널 크게 / 복원 |
| **패널 탭** | Ctrl+Alt+T / Ctrl+Alt+W | 새 / 닫기 패널 탭 |
|  | Ctrl+Alt+PgDn / PgUp | 다음 / 이전 패널 탭 |
| **이동** | Alt+← / Alt+→ | 뒤로 / 앞으로 |
|  | Alt+↑ / Alt+↓ | 상위 폴더 / 최근 폴더 |
|  | Ctrl+L, Alt+D, F4 | 주소창 |
| **보기** | Ctrl+F | 이 폴더 검색 (하위 폴더 → Everything) |
|  | Space / F3 | 빠른 미리보기 |
|  | F6 / Shift+F6 | 다음 / 이전 포커스 |
|  | Ctrl+H | 숨김 파일 |
|  | Ctrl+Shift+E | 왼쪽 패널 |
|  | Ctrl+Shift+O | 열린 탐색기 폴더 가져오기 |
| **미리보기** | Ctrl+휠 | 확대 10–800% |
|  | Ctrl+0 | 확대 초기화 |
|  | 끌기 | 확대한 사진 끌어서 이동 |
|  | 휠, ↑ / ↓, PgUp / PgDn, Home / End | 스크롤 |
|  | ← / → | 이전 / 다음 파일 |
|  | Esc / Space / F3 | 닫기 |
| **파일** | Ctrl+Shift+C | 전체 경로 복사 |
|  | Num+ or Ctrl+Shift+A / Num− / Num* | 패턴으로 선택 / 해제 / 반전 |
|  | Ctrl+Alt+N | 새 텍스트 문서 |
|  | Ctrl+Shift+N, F7 | 새 폴더 |
|  | F2, Del, Ctrl+C / X / V, F5 | 탐색기와 동일 (그림을 Ctrl+V 하면 PNG로 저장) |
| **기타** | Ctrl+B | 즐겨찾기 팝업 |
|  | Ctrl+S | 세션 저장 |
|  | Ctrl+, | 설정 |
|  | F1 / Ctrl+/ | 단축키 도움말 / 목록 |
|  | Ctrl+N | 새 창 |
|  | Win+Alt+E | Grids 앞으로 가져오기 |

## 설정

탭 줄의 톱니가 보기 메뉴를 엽니다. 설정은 그 메뉴에 있습니다(`Ctrl+,`).

설정은 `%APPDATA%\grids\grids.json`에 저장됩니다. `grids.exe` 옆에 `portable.txt`를 두면 `grids.json`도 exe 옆에 둡니다. 이름 붙인 세션은 exe 옆 `sessions` 폴더의 `.grids` 파일입니다. 기존 `grids.ini`는 한 번 읽은 뒤 `grids.ini.bak`으로 바꿉니다. 예전 `grids.ini`의 `lang=ko`도 한국어로 읽힙니다.

파일은 원자적으로 바뀝니다. 저장이 실패하면 5초 뒤에 다시 시도합니다. 읽지 못하는 파일은 기본값으로 덮지 않고, 다시 시도하거나 종료합니다. 모르는 키는 그대로 둡니다. 최소화해도 최대화였으면 최대화로 기억합니다.

기본값은 Windows 표시 언어(한국어, 일본어, 영어)를 따르고, 그 외에는 영어입니다. 자동, English, 한국어, 日本語를 고를 수 있고, `"lang": "auto"`, `"en"`, `"ko"`, `"ja"`로 저장됩니다. 패널 안의 탐색기 부분(열 이름, 우클릭 메뉴)은 Windows 표시 언어를 따릅니다. 글꼴은 그 언어의 기본 글꼴(Segoe UI, 맑은 고딕, Yu Gothic UI)에서 시작하고, 고르면 창 전체에 바로 적용됩니다(`font_face`). 글꼴 크기 설정은 없으며, 예전 `font` 값은 무시합니다.

보기 메뉴의 파일 확장명 표시는 탐색기와 같은 Windows 설정이고, 설정 창에도 있습니다. 새로 설치하면 보기 메뉴의 Windows 라이트·다크 설정 따르기가 그 체크입니다. 다크 테마나 라이트 테마를 고르면 직접 바꾸고 따라가기를 끕니다. 목록은 그 자리에서 다시 칠하므로, 이동 기록, 필터, 선택, 스크롤, 입력 중인 주소가 남습니다.

| 키 | 설명 |
| --- | --- |
| `side_split` | 폴더 트리와 즐겨찾기 사이 구분선. 다시 열 때 돌아옵니다. 왼쪽 패널 전체는 설정에서 숨길 수 있습니다. |
| `use_everything` | `"use_everything": false`가 없으면 켜져 있습니다. 켜져 있으면 하위 폴더 검색이, Everything이 실행 중일 때 Everything에 묻습니다. 설정 → 일반에는 설치되어 실행 중, 설치되어 있지만 실행 중이 아님, 설치되어 있지 않음이 나오고, Everything이 버전을 알려 주면 함께 표시됩니다. Everything 시작은 설치된 `Everything.exe`를 실행합니다. 설치는 winget이 있으면 `winget install --id voidtools.Everything -e`를 실행하고, 없으면 voidtools 다운로드 페이지를 엽니다. |
| `ev_asked` | Everything 설치를 한 번만 물은 답을 기억합니다. Everything이 없을 때 처음 하위 폴더 검색을 하면 설치할지 묻습니다. |
| `drive_buttons` | 키가 없으면 켜짐. 설정 → 모양 → 드라이브 버튼 표시를 끄면 버튼이 숨고 그 자리를 탭이 씁니다. 창이 좁으면 뒤쪽 글자부터 숨깁니다. |
| `layout_new` | 첫 실행과 새 창의 기본 레이아웃. 좌우 2개, 위아래 2개, 2×2, 1개 중 하나입니다. 다른 실행에서 넘겨받은 경로는 1패널 탭으로 열립니다. 창 탭을 새로 만들면(+ 또는 Ctrl+T) 기본 레이아웃 대신 지금 탭의 배치를 가져옵니다. |
| `hotkey` | 줄이 없으면 켜짐. Win+Alt+E로 Grids를 앞으로 가져오고, 다시 누르면 최소화합니다. Grids 대화 상자가 열려 있으면 그 창이 앞으로 옵니다. 보기 메뉴의 체크는 키가 실제로 등록됐을 때만 켜지므로, 시작에 실패해도 한 번 누르면 다시 시도합니다. 단축키는 설정에서 바꿀 수 있습니다. |
| `space_preview` | 키가 없으면 켜짐. 설정 → 패널·보기 → 스페이스로 미리보기를 끄면 스페이스로는 열리지 않습니다. 글자를 입력한 뒤 1초 안의 스페이스나, 한글·일본어를 조합하는 중의 스페이스는 이름의 일부로 들어갑니다. Ctrl+Space와 Shift+Space는 그대로 둡니다. |
| `folders` | 패널 안의 탭으로 연 폴더. 패널마다 저장되고, 이름 붙인 세션에도 들어갑니다. 예전 `pane_tabs` 값은 무시됩니다. |
| `favorites_tree` | 즐겨찾기 트리. 예전 목록은 맨 위로 옮겨집니다. |
| `one_window` | 창 하나만 쓰기. 줄이 없으면 꺼져 있습니다. 켜면 다시 실행할 때 최소화된 창을 되돌리고, 경로 인자는 그 창의 탭으로 엽니다. Ctrl+N, 탭을 밖으로 끌기, 새 창으로 이동, 이미 앞에 있을 때의 Win+Alt+E는 새 창 대신 이 창의 탭을 엽니다. |
| `lang` | `"auto"`, `"en"`, `"ko"`, `"ja"`. 키가 없거나 `"auto"`이면 Windows 표시 언어(한국어, 일본어, 영어)를 따르고, 그 외에는 영어입니다. |
| `font_face` | 창 전체의 글꼴. 글꼴 크기는 저장하지 않습니다. 예전 `font` 값은 무시됩니다. |

## 로드맵

- [ ] `grids.exe`에 코드 서명을 넣어, 처음 실행할 때 SmartScreen 경고가 나오지 않게 합니다
- [ ] English, 한국어, 日本語 이외의 표시 언어

## 감사

Grids는 efww의 [grids](https://github.com/efww/grids)를 포크한 프로그램입니다.

[Q-Dir](https://www.softwareok.com/?seite=Freeware/Q-Dir)와 [voidtools Everything](https://www.voidtools.com/)에서 영감을 받았습니다. Grids는 Q-Dir 및 그 제작자와 관계없는 독립 프로젝트입니다.

## 라이선스

[MIT](LICENSE)
