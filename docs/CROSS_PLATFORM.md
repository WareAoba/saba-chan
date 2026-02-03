# 크로스 플랫폼 지원 가이드

이 문서는 saba-chan의 크로스 플랫폼 지원 현황과 각 OS별 특이사항을 설명합니다.

## 지원 플랫폼

- **Windows** (주 개발 플랫폼)
- **Linux** (Ubuntu, Debian, Fedora 등)
- **macOS** (Intel 및 Apple Silicon)

## 주요 크로스 플랫폼 구현

### 1. 프로세스 모니터링 (`src/process_monitor.rs`)

**이전**: Windows PowerShell 명령어에 의존
```rust
Command::new("powershell")
    .args(["-NoProfile", "-Command", "Get-Process | ..."])
```

**현재**: `sysinfo` 크레이트 사용
```rust
use sysinfo::{System, SystemExt, ProcessExt};

let mut sys = System::new_all();
sys.refresh_all();
sys.processes() // 크로스 플랫폼
```

**장점**:
- 모든 플랫폼에서 동일한 API
- PowerShell 의존성 제거
- 더 나은 성능과 안정성

### 2. 경로 감지 (`src/path_detector.rs`)

각 플랫폼별 게임 설치 경로 자동 감지:

#### Windows
- `%ProgramFiles(x86)%\Steam\steamapps\common`
- `C:\, D:\, E:\, F:\ 드라이브의 SteamLibrary`
- `%USERPROFILE%\Desktop`

#### Linux
- `~/.steam/steam/steamapps/common`
- `~/.local/share/Steam/steamapps/common`
- `~/.var/app/com.valvesoftware.Steam/.steam/steam/steamapps/common` (Flatpak)

#### macOS
- `~/Library/Application Support/Steam/steamapps/common`

### 3. 프로세스 종료 (`src/supervisor/process.rs`)

#### Windows
```rust
use winapi::um::processthreadsapi::TerminateProcess;
```

#### Unix/Linux/macOS
```rust
use nix::sys::signal::{kill, Signal};
kill(Pid::from_raw(pid), Signal::SIGTERM);
```

**우아한 종료 전략**:
1. `SIGTERM` (Unix) / 일반 종료 (Windows)로 먼저 시도
2. 응답 없으면 `SIGKILL` (Unix) / 강제 종료 (Windows)

### 4. Electron GUI (`electron_gui/main.js`)

#### 데몬 프로세스 종료
```javascript
if (process.platform === 'win32') {
    // Windows: taskkill로 프로세스 트리 전체 종료
    execSync(`taskkill /PID ${pid} /F /T`);
} else {
    // Unix: SIGTERM -> SIGKILL 순차 시도
    daemonProcess.kill('SIGTERM');
    setTimeout(() => daemonProcess.kill('SIGKILL'), 2000);
}
```

#### macOS 앱 종료 동작
```javascript
if (process.platform === 'darwin') {
    app.quit(); // 완전 종료
} else {
    // Windows/Linux: 트레이에 남아있음
}
```

## 의존성 관리

### Cargo.toml 구조
```toml
[dependencies]
# 공통 크로스 플랫폼 크레이트
sysinfo = "0.30"  # 프로세스 정보
glob = "0.3"      # 파일 패턴 매칭

[target.'cfg(windows)'.dependencies]
winapi = { version = "0.3", features = ["processthreadsapi", "winnt", "handleapi"] }

[target.'cfg(unix)'.dependencies]
nix = { version = "0.27", features = ["signal"] }
```

## 테스트 가이드

### Windows
```powershell
cargo build --release
cargo test
```

### Linux/macOS
```bash
cargo build --release
cargo test
```

### Electron GUI
```bash
cd electron_gui
npm install
npm test
npm start
```

## 플랫폼별 알려진 제약사항

### Windows
- ✅ 완전 지원
- ⚠️ 관리자 권한 필요할 수 있음 (일부 프로세스 종료 시)

### Linux
- ✅ 대부분 지원
- ⚠️ Steam Flatpak 사용 시 경로 확인 필요
- ⚠️ 권한 문제로 일부 프로세스 접근 불가할 수 있음

### macOS
- ⚠️ 테스트 필요
- ⚠️ Apple Silicon에서 Rosetta 2 필요 여부 확인 필요
- ⚠️ Gatekeeper 보안 설정 확인 필요

## 향후 개선 사항

1. **CI/CD**: GitHub Actions에서 Linux, macOS 빌드 추가
2. **패키징**: 
   - Windows: MSI 인스톨러
   - Linux: .deb, .rpm, AppImage
   - macOS: .dmg
3. **권한 관리**: 각 플랫폼별 권한 요청 UI 개선
4. **경로 설정**: 사용자 정의 게임 경로 설정 UI

## 참고 자료

- [sysinfo 문서](https://docs.rs/sysinfo/)
- [nix 문서](https://docs.rs/nix/)
- [Electron 플랫폼별 가이드](https://www.electronjs.org/docs/latest/tutorial/platform-specific-code)
