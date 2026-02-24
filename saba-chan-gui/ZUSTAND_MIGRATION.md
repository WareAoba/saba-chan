# Zustand 도입 개발 지시서

> **목표**: App.js God Component(1,057줄, useState 60+개)를 Zustand store로 분해하여  
> prop drilling 제거, 로직 중복 해소, 유지보수성 확보

---

## 1. 현황 분석

### 1.1 현재 아키텍처의 문제점

| 문제 | 구체적 증상 |
|---|---|
| **God Component** | `App.js` 1,057줄, `useState` 60+개가 단일 함수에 집중 |
| **Prop Drilling** | `DiscordBotModal`에 35개 props (10개 state/setter 쌍), `ServerCard`에 21개 props |
| **로직 중복** | `loadBotConfig()` 함수가 초기 로드 effect와 별도 콜백에 동일 코드로 2번 정의 |
| **Auto-save 산재** | 설정 변경 감지 effect가 5개 이상 분산되어 있고, `prevRef` 패턴이 반복됨 |
| **훅 간 의존 폭발** | `useServerActions`에 12개 파라미터, `useDiscordBot`에 12개 파라미터 전달 |

### 1.2 현재 상태 분류 (App.js 기준)

```
[Core Server]     servers, modules, loading, daemonReady, serversInitializing
[Init]            initStatus, initProgress
[App Settings]    autoRefresh, refreshInterval, ipcPort, consoleBufferSize, modulesPath, settingsPath
[Discord Bot]     discordToken, discordPrefix, discordAutoStart, discordBotMode,
                  discordCloudRelayUrl, discordCloudHostId, discordModuleAliases,
                  discordCommandAliases, discordMusicEnabled, nodeSettings,
                  cloudNodes, cloudMembers, discordBotStatus
[UI / Modal]      modal, progressBar, showModuleManager, showCommandModal, commandServer,
                  showGuiSettingsModal, settingsInitialView, contextMenu,
                  showDiscordSection, showBackgroundSection, showNoticeSection,
                  unreadNoticeCount, showWaitingImage
[Background]      backgroundDaemonStatus
[Module Meta]     moduleAliasesPerModule
[Uptime]          nowEpoch
```

---

## 2. Store 설계

### 2.1 Store 분리 전략

4개 도메인 store + 1개 UI store로 분리한다. 각 store는 독립적으로 동작하며, 필요시 `getState()`로 다른 store를 참조한다.

```
src/stores/
  ├── useSettingsStore.js      # GUI 설정 + 경로
  ├── useDiscordStore.js       # Discord 봇 전체 상태
  ├── useServerStore.js        # 서버 목록 + 모듈 + 상태
  └── useUIStore.js            # 모달, 진행바, 컨텍스트메뉴 등 UI 상태
```

### 2.2 각 Store 상세 설계

#### `useSettingsStore` — GUI 설정

```js
{
  // State
  autoRefresh: true,
  refreshInterval: 2000,
  ipcPort: 57474,
  consoleBufferSize: 2000,
  modulesPath: '',
  settingsPath: '',
  settingsReady: false,

  // Actions
  load(),           // window.api.settingsLoad() → state 반영
  save(),           // 현재 state → window.api.settingsSave()
  update(partial),  // set(partial) + debounced save()
}
```

**제거되는 App.js useState**: `autoRefresh`, `refreshInterval`, `ipcPort`, `consoleBufferSize`, `modulesPath`, `settingsPath`, `settingsReady` (7개)  
**제거되는 effect**: `prevSettingsRef` 기반 auto-save effect 2개

---

#### `useDiscordStore` — Discord 봇

```js
{
  // Bot config
  discordToken: '',
  discordPrefix: '!saba',
  discordAutoStart: false,
  discordMusicEnabled: true,
  discordModuleAliases: {},
  discordCommandAliases: {},

  // Bot mode
  discordBotMode: 'local',
  discordCloudRelayUrl: '',
  discordCloudHostId: '',

  // Node & cloud
  nodeSettings: {},
  cloudNodes: [],
  cloudMembers: {},

  // Runtime status (useDiscordBot 훅에서 이관)
  discordBotStatus: 'stopped',
  botStatusReady: false,
  relayConnected: false,
  relayConnecting: false,

  // Actions
  loadConfig(),         // window.api.botConfigLoad() → state 반영
  saveConfig(),         // 현재 state → window.api.botConfigSave()
  update(partial),      // set(partial) + debounced saveConfig()
  startBot(),           // 봇 시작 로직 (useDiscordBot에서 이관)
  stopBot(),            // 봇 정지 로직
  checkStatus(),        // 상태 폴링 (cloud/local 분기)
  startStatusPolling(), // setInterval 시작
  stopStatusPolling(),  // setInterval 클리어
}
```

**제거되는 App.js useState**: 15개  
**제거되는 훅**: `useDiscordBot` 전체 → store 내부 action으로 흡수  
**제거되는 effect**: `prevPrefixRef`, `prevCloudSettingsRef` 기반 auto-save effect 2개

---

#### `useServerStore` — 서버/모듈

```js
{
  // State
  servers: [],
  modules: [],
  loading: true,
  moduleAliasesPerModule: {},

  // Init state
  daemonReady: false,
  initStatus: 'Initialize...',
  initProgress: 0,
  serversInitializing: true,

  // Uptime (requestAnimationFrame 기반)
  nowEpoch: Math.floor(Date.now() / 1000),

  // Actions
  fetchServers(),       // useServerActions.fetchServers() 이관
  fetchModules(),       // App.js fetchModules() 이관
  startServer(name, module),
  stopServer(name),
  addServer(config),
  deleteServer(server),
  reorderServers(newOrder),
  setInitReady(),
  startUptimeClock(),
  formatUptime(startTime),
}
```

**제거되는 App.js useState**: `servers`, `modules`, `loading`, `daemonReady`, `initStatus`, `initProgress`, `serversInitializing`, `moduleAliasesPerModule`, `nowEpoch` (9개)  
**제거되는 훅**: `useServerActions` 대부분 → store action으로 흡수

---

#### `useUIStore` — UI 상태

```js
{
  // Modal
  modal: null,              // { type, title, message, ... }
  progressBar: null,        // { message, percent, indeterminate }
  showWaitingImage: false,

  // Panel visibility
  showModuleManager: false,
  showGuiSettingsModal: false,
  settingsInitialView: null,
  showCommandModal: false,
  commandServer: null,
  showDiscordSection: false,
  showBackgroundSection: false,
  showNoticeSection: false,
  contextMenu: null,

  // Notice
  unreadNoticeCount: 0,

  // Background
  backgroundDaemonStatus: 'checking',

  // Actions
  openModal(config),
  closeModal(),
  setProgressBar(config),
  clearProgressBar(),
  openSettings(initialView?),
  togglePanel(panelName),
}
```

**제거되는 App.js useState**: 약 15개

---

## 3. 마이그레이션 순서

> **원칙**: 한 번에 하나의 store만 도입 → 테스트 통과 확인 → 다음 store 진행  
> 기존 코드와 공존 가능하므로 점진적으로 진행한다.

### Phase 0: 환경 준비
- [ ] `npm install zustand` (devDependencies 아님, dependencies)
- [ ] `src/stores/` 디렉토리 생성
- [ ] 기존 테스트(`gui-e2e.test.js`) 통과 확인 (baseline)

### Phase 1: `useUIStore` (가장 안전, 부수효과 없음)
- [ ] `src/stores/useUIStore.js` 생성
- [ ] App.js에서 modal, progressBar, panel visibility 관련 useState 제거
- [ ] App.js render 부분에서 `useUIStore()` 로 교체
- [ ] 하위 컴포넌트에서 props 대신 `useUIStore()` 직접 구독으로 전환
  - `SettingsModal`: `onTestModal`, `onTestProgressBar` props 제거
  - `CommandModal`: `onExecute` → `useUIStore.getState().openModal` 직접 호출
- [ ] 테스트 통과 확인

### Phase 2: `useSettingsStore` (IPC 연동, auto-save 캡슐화)
- [ ] `src/stores/useSettingsStore.js` 생성 (debounced save 내장)
- [ ] App.js의 settings load effect → `useSettingsStore.getState().load()` 로 교체
- [ ] auto-save effect 3개 제거 → store 내부 `subscribe` 기반 auto-save
- [ ] `SettingsModal` 리팩토링: props 대신 store 직접 접근
  - `refreshInterval`, `ipcPort`, `consoleBufferSize` 등 props 제거
- [ ] `BackgroundModal`의 `ipcPort` prop → store에서 직접 접근
- [ ] `consoleBufferRef` 제거 → store에서 직접 접근으로 대체 (혹은 store subscribe)
- [ ] 테스트 통과 확인

### Phase 3: `useDiscordStore` (가장 큰 효과)
- [ ] `src/stores/useDiscordStore.js` 생성
- [ ] `useDiscordBot` 훅 전체를 store action으로 이관
  - 상태 폴링 로직 → `startStatusPolling()` / `stopStatusPolling()` action
  - auto-start 로직 → `subscribe` 기반으로 store 내부에서 관리
- [ ] App.js의 discord 관련 useState 15개 제거
- [ ] App.js의 `loadBotConfig()` 중복 제거 → `useDiscordStore.getState().loadConfig()` 단일화
- [ ] `DiscordBotModal` 리팩토링: **35개 props → ~5개 이하**
  - state/setter 10쌍 제거 → store 직접 접근
  - `handleStartDiscordBot`, `handleStopDiscordBot` props 제거 → store action 호출
  - 남는 props: `isOpen`, `onClose`, `isClosing` (모달 제어만)
- [ ] auto-save effect 2개 제거 → store 내부 `subscribe` 기반
- [ ] `useDiscordBot.js` 파일 삭제
- [ ] 테스트 통과 확인

### Phase 4: `useServerStore` (핵심 비즈니스 로직)
- [ ] `src/stores/useServerStore.js` 생성
- [ ] `useServerActions` 훅의 `fetchServers` 로직 → store action으로 이관
  - optimistic status, error toast throttle 등 ref 기반 로직 → store 내부 변수로 이관
- [ ] App.js의 `fetchModules()` → store action으로 이관
- [ ] `ServerCard`: `servers`, `modules`, `handleStart`, `handleStop` 등 props 제거 → store 직접 접근
  - 남는 props: `server`, `index` (아이템 식별), 드래그 관련 props
- [ ] `useDragReorder`: `servers`, `setServers` 파라미터 → store 직접 접근
- [ ] `AddServerModal`: 다수 props → store 직접 접근
- [ ] uptime clock → store 내부 `startUptimeClock()` action
- [ ] `useServerActions.js` 파일 삭제
- [ ] 테스트 통과 확인

### Phase 5: App.js 정리
- [ ] App.js에서 남은 useState/useEffect 정리
- [ ] 커스텀 훅 중 빈 껍데기가 된 것 삭제
- [ ] App.js가 **순수 렌더링 + 라우팅**(popout mode 분기)만 담당하도록 정리
- [ ] 최종 목표: **App.js 200줄 이하**

---

## 4. 구현 규칙

### 4.1 Store 작성 컨벤션

```js
// src/stores/useExampleStore.js
import { create } from 'zustand';

export const useExampleStore = create((set, get) => ({
  // ── State ──
  value: 'default',

  // ── Actions ──
  // set() 만 사용, this 사용 금지
  update: (partial) => set(partial),
  
  // 비동기 action
  load: async () => {
    const data = await window.api.someLoad();
    set({ value: data.value });
  },

  // 다른 store 참조 시 getState() 사용 (import 시점 순환 참조 방지)
  doSomething: () => {
    const { modal } = useUIStore.getState();
    // ...
  },
}));
```

### 4.2 컴포넌트에서 사용 규칙

```js
// ✅ selector로 필요한 것만 구독 (불필요한 리렌더 방지)
const servers = useServerStore(s => s.servers);
const fetchServers = useServerStore(s => s.fetchServers);

// ✅ 여러 값이 필요하면 shallow 비교
import { useShallow } from 'zustand/react/shallow';
const { servers, modules } = useServerStore(useShallow(s => ({
  servers: s.servers,
  modules: s.modules,
})));

// ❌ 전체 store 구독 금지 (모든 변경에 리렌더)
const store = useServerStore();  // BAD

// ✅ 이벤트 핸들러 등 컴포넌트 외부에서는 getState()
useServerStore.getState().fetchServers();
```

### 4.3 Auto-save 패턴

```js
// store 내부에서 subscribe 기반 auto-save
// 위치: store 파일 하단 (모듈 레벨 side-effect)

let saveTimer = null;
useSettingsStore.subscribe(
  (state) => ({
    autoRefresh: state.autoRefresh,
    refreshInterval: state.refreshInterval,
    ipcPort: state.ipcPort,
    consoleBufferSize: state.consoleBufferSize,
  }),
  (current, previous) => {
    // settingsReady 전에는 무시
    if (!useSettingsStore.getState().settingsReady) return;
    clearTimeout(saveTimer);
    saveTimer = setTimeout(() => {
      useSettingsStore.getState().save();
    }, 500);  // 500ms debounce
  },
  { equalityFn: shallow }
);
```

### 4.4 테스트에서 store 모킹

```js
// 테스트 전 store 초기화
import { useUIStore } from '../stores/useUIStore';

beforeEach(() => {
  // store를 기본값으로 리셋
  useUIStore.setState({
    modal: null,
    progressBar: null,
    showModuleManager: false,
    // ...
  });
});
```

---

## 5. 파일 변경 요약 (예상)

### 새로 생성

| 파일 | 설명 |
|---|---|
| `src/stores/useUIStore.js` | UI 상태 store |
| `src/stores/useSettingsStore.js` | 설정 store |
| `src/stores/useDiscordStore.js` | Discord store |
| `src/stores/useServerStore.js` | 서버 store |

### 대폭 수정

| 파일 | 예상 변경 |
|---|---|
| `App.js` | 1,057줄 → ~200줄 (useState 60개 → 0개) |
| `DiscordBotModal.js` | props 35개 → ~5개 |
| `ServerCard.js` | props 21개 → ~5개 |
| `SettingsModal.js` | props 13개 → ~3개 |
| `ServerSettingsModal.js` | 대부분 props store 접근으로 교체 |

### 삭제

| 파일 | 사유 |
|---|---|
| `hooks/useDiscordBot.js` | → `useDiscordStore` 흡수 |
| `hooks/useServerActions.js` | → `useServerStore` 흡수 |
| `hooks/useServerSettings.js` | → 일부 `useServerStore`, 일부 로컬 state로 분리 |
| `utils/settingsManager.js` | → `useSettingsStore` 흡수 |

### 변경 없음 (유지)

| 파일 | 사유 |
|---|---|
| `hooks/useConsole.js` | 콘솔은 인스턴스별 로컬 상태, store 불필요 |
| `hooks/useDragReorder.js` | Phase 4에서 servers 접근만 변경, 훅 자체는 유지 |
| `hooks/useModalClose.js` | 애니메이션 유틸, 변경 없음 |
| `hooks/useWaitingImage.js` | progressBar 구독만 store로 변경 |
| `hooks/useDevMode.js` | 독립적 키보드 훅, 변경 없음 |
| `contexts/ExtensionContext.js` | 익스텐션 시스템은 별도 도메인, 변경 없음 |
| `utils/themeManager.js` | localStorage 기반, 그대로 유지 |

---

## 6. 리스크 및 대응

| 리스크 | 대응 |
|---|---|
| **기존 테스트 깨짐** | 각 Phase 완료 시 `gui-e2e.test.js` 실행. store mock 패턴 확립 (4.4절) |
| **store 간 순환 참조** | import 시 store 참조 금지, `getState()` lazy 참조만 허용 |
| **과도한 리렌더** | selector 사용 필수 (4.2절). 전체 store 구독 코드 리뷰로 차단 |
| **디버깅 어려움** | Zustand devtools middleware 연동 (`devtools()` 래핑). Redux DevTools에서 확인 가능 |
| **비동기 경합** | IPC 호출은 store action 내부에서만. 동시 호출 방지는 `loading` flag로 |
| **Phase 중간에 배포 필요** | 각 Phase가 독립적으로 동작하므로, 어느 Phase에서든 배포 가능 |

---

## 7. 검증 기준

각 Phase 완료 시 아래 항목을 확인한다:

- [ ] `npm test` (vitest) 전체 통과
- [ ] `npm run build` 에러 없음
- [ ] `npm run start` 후 수동 검증:
  - 서버 추가/삭제/시작/정지
  - 설정 변경 → 앱 재시작 후 값 유지
  - Discord 봇 시작/정지
  - 테마/언어 변경
  - 콘솔 열기/닫기/팝아웃
- [ ] App.js가 Phase별로 점진적으로 축소되고 있는지 확인

### 최종 목표 수치

| 지표 | Before | After |
|---|---|---|
| App.js 줄 수 | 1,057 | **< 200** |
| App.js useState 수 | 60+ | **0** |
| DiscordBotModal props | 35 | **< 5** |
| ServerCard props | 21 | **< 8** |
| Auto-save effect 수 (App.js) | 5+ | **0** (store 내부) |
| 중복 loadBotConfig 정의 | 2 | **1** (store 단일) |
| 추가 번들 크기 | — | **~1.1KB** (gzip) |
