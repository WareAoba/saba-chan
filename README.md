# 🐟 Saba-chan (サバちゃん)

> **모듈형 게임 서버 관리 플랫폼** - 여러 게임 서버를 하나의 GUI에서 통합 관리

<p align="center">
  <img src="docs/screenshot.png" alt="Saba-chan Screenshot" width="600">
</p>

## ✨ 특징

- 🎮 **다중 게임 지원** - Palworld, Minecraft 등 모듈로 확장 가능
- 🔍 **자동 프로세스 감지** - 실행 중인 게임 서버를 자동으로 탐지
- 📦 **모듈 시스템** - 새 게임 추가 시 Core 재컴파일 불필요
- 🖥️ **Electron GUI** - 직관적인 데스크톱 앱
- 🔒 **안전한 설계** - Daemon 크래시 ≠ 게임 서버 크래시

## 🏗️ 아키텍처

```
┌─────────────────┐     HTTP API      ┌──────────────────┐
│  Electron GUI   │ ◄───────────────► │   Core Daemon    │
│  (React 18)     │   127.0.0.1:57474 │   (Rust/Axum)    │
└─────────────────┘                   └────────┬─────────┘
                                               │
                                    ┌──────────┴──────────┐
                                    │                     │
                              ┌─────▼─────┐        ┌──────▼──────┐
                              │  Modules  │        │  Instances  │
                              │ (게임별)   │        │ (서버 설정)  │
                              └───────────┘        └─────────────┘
```

## 🚀 빠른 시작

### 요구사항
- Windows 10/11
- [Rust](https://rustup.rs/) (Core Daemon 빌드용)
- [Node.js 18+](https://nodejs.org/) (GUI용)

### 설치

```bash
# 1. 저장소 클론
git clone https://github.com/YOUR_USERNAME/saba-chan.git
cd saba-chan

# 2. Core Daemon 빌드
cargo build --release

# 3. GUI 설치 및 실행
cd electron_gui
npm install
npm start
```

## 📁 프로젝트 구조

```
saba-chan/
├── src/                    # Rust Core Daemon
│   ├── main.rs             # 진입점
│   ├── ipc/                # HTTP API 서버
│   ├── supervisor/         # 프로세스 관리
│   └── instance/           # 서버 인스턴스 관리
├── modules/                # 게임별 모듈
│   ├── palworld/           # Palworld 모듈
│   └── minecraft/          # Minecraft 모듈
├── electron_gui/           # Electron + React GUI
└── PROJECT_GUIDE.md        # 개발자 가이드
```

## 🎮 지원 게임

| 게임 | 상태 | 프로세스명 |
|------|------|-----------|
| Palworld | ✅ 지원 | `PalServer-Win64-Shipping-Cmd` |
| Minecraft | 🚧 준비중 | `java` |

## 📖 문서

- [PROJECT_GUIDE.md](PROJECT_GUIDE.md) - 상세 개발 가이드 및 문제 해결
- [API_SPEC.md](API_SPEC.md) - REST API 명세

## 🛠️ 개발

### Core Daemon 빌드
```bash
cargo build --release
```

### GUI 개발 모드
```bash
cd electron_gui
npm start
```

### API 테스트
```powershell
Invoke-RestMethod -Uri "http://127.0.0.1:57474/api/servers" | ConvertTo-Json
```

## 🤝 기여

1. Fork
2. Feature 브랜치 생성 (`git checkout -b feature/amazing-feature`)
3. 커밋 (`git commit -m 'Add amazing feature'`)
4. Push (`git push origin feature/amazing-feature`)
5. Pull Request

## 📜 라이선스

MIT License - 자유롭게 사용하세요!

## 🙏 감사

- [juunini/palworld-discord-bot](https://github.com/juunini/palworld-discord-bot) - Palworld RCON 참고

---

<p align="center">
  Made with ❤️ and 🐟
</p>
