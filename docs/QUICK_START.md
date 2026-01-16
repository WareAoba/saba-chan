# 🚀 Quick Start - 서버 시작하기

## ⚠️ 중요: 첫 실행 시 필수 설정

서버를 시작하기 전에 **반드시** 서버 실행 파일 경로를 설정해야 합니다!

### 📝 instances.json 편집

1. **파일 위치**: `c:\Git\saba-chan\instances.json`
2. **편집기**: VS Code 또는 메모장 (메모장++ 권장)

### 예시: Palworld 서버

```json
[
  {
    "id": "0d733e76-2edc-4413-864c-3b376b255c66",
    "name": "my-palworld-1",
    "module_name": "palworld",
    "executable_path": "D:\\SteamLibrary\\steamapps\\common\\PalServer\\PalServer.exe",
    "working_dir": "D:\\SteamLibrary\\steamapps\\common\\PalServer",
    "auto_detect": true,
    "process_name": "PalServer-Win64-Shipping-Cmd",
    "port": 8211,
    "rcon_port": 25575,
    "rcon_password": null
  }
]
```

### 🔍 Palworld 서버 경로 찾기

#### 방법 1: Steam에서 찾기
1. Steam 라이브러리에서 `Palworld Dedicated Server` 우클릭
2. **관리 → 로컬 파일 보기** 클릭
3. 폴더 경로 복사 (예: `D:\SteamLibrary\steamapps\common\PalServer`)

#### 방법 2: 일반적인 경로
```
C:\Program Files (x86)\Steam\steamapps\common\PalServer\PalServer.exe
D:\SteamLibrary\steamapps\common\PalServer\PalServer.exe
E:\Games\steamapps\common\PalServer\PalServer.exe
```

### ✏️ 경로 수정 방법

1. 위에서 찾은 경로를 복사
2. `instances.json`에서 `executable_path`에 붙여넣기
3. **중요**: 백슬래시를 이중으로 변경
   - ❌ `D:\SteamLibrary\steamapps\...`
   - ✅ `D:\\SteamLibrary\\steamapps\\...`
4. `working_dir`에도 같은 폴더 경로 입력

### 💾 저장 후 GUI 재시작

1. instances.json 저장
2. Electron GUI 재시작
3. Start 버튼 클릭!

---

## 🎮 실행 확인

서버가 정상적으로 시작되면:
- GUI에서 상태가 `running` (녹색)으로 변경
- PID가 표시됨
- 작업 관리자에서 `PalServer-Win64-Shipping-Cmd` 프로세스 확인 가능

---

## ❌ 에러 발생 시

### "server_executable not specified"
→ `executable_path`가 `null`입니다. 위 예시대로 경로를 입력하세요.

### "Executable not found: [경로]"
→ 입력한 경로에 파일이 없습니다. Steam에서 경로를 다시 확인하세요.

### 서버가 바로 종료됨
→ 서버 설정 파일 오류입니다. Palworld 서버 폴더의 로그 파일을 확인하세요.

---

## 📖 더 자세한 정보

- [USAGE_GUIDE.md](USAGE_GUIDE.md) - 전체 사용자 가이드
- [PROJECT_GUIDE.md](PROJECT_GUIDE.md) - 개발자 가이드
- [COMMUNICATION_SPEC.md](COMMUNICATION_SPEC.md) - API 명세

---

**TIP**: 경로를 모르겠다면 Windows 탐색기에서 `PalServer.exe`를 검색하세요!
