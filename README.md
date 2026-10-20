# qdir-rs

Q-Dir 스타일 다중 패널 파일 관리자 (Rust, Win32). 각 패널이 Windows 탐색기 컨트롤(IExplorerBrowser)이라 우클릭 메뉴·드래그 앤 드롭·썸네일은 Windows 그대로입니다. 설치 없이 exe 하나로 실행되고, 설정은 exe 옆 `qdir.ini`에 저장됩니다.

- 왼쪽 폴더 트리 + 1×1 ~ 4×4 그리드, 경계 드래그로 크기 조절
- 패널별 주소창·뒤로/앞으로/상위·이름 필터, ⇄ 다른 패널에서 열기
- 세션 자동 저장, 이름 붙인 세션(.qdr), 단일 인스턴스, 상태바
- 조약돌 테마: 둥근 흰 플레이트 + 그림자, 활성 패널은 액센트 링 하나, 레이아웃은 그리드 피커로 선택
- 글꼴: Pretendard Regular를 exe에 내장해 설치 안 된 PC에서도 같은 모양. 보기 → 글꼴로 변경, exe 옆 `fonts\`에 넣은 글꼴도 설치 없이 사용

Pretendard © Kil Hyung-jin, [SIL Open Font License 1.1](assets/Pretendard-LICENSE.txt).
- 단축키: 보기 → 단축키

## 빌드

Rust GNU 툴체인 + msys2(`C:\msys64\mingw64\bin`: windres·dlltool·as)가 필요합니다.

```
build.cmd
```

`target\release\qdir.exe`를 빌드해 루트에 `qdir.exe`로 복사합니다.
