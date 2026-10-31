import { create } from 'zustand';

export const useUIStore = create((set, _get) => ({
    // ── Navigation ──
    // activeTab = 상단 페이지 (servers | discord | settings), activeServerId = 서버 페이지에서 고른 서버
    activeTab: 'servers',
    activeServerId: null,
    // 서버 상세에 "이 탭을 열어 달라"는 요청 (우클릭 메뉴 등). 상세가 읽고 비운다.
    requestedServerTab: null,
    // 바깥(CLI의 `gui`)에서 서버를 연 횟수. 좁은 창에서도 목록 대신 그 서버 상세를 보인다
    revealServerCount: 0,

    // ── Modal ──
    modal: null,
    progressBar: null,
    showWaitingImage: false,

    // ── Panel visibility ──
    showModuleManager: false,
    showGuiSettingsModal: false,
    settingsInitialView: null,
    showCommandModal: false,
    commandServer: null,
    showDiscordSection: false,
    showBackgroundSection: false,
    showNoticeSection: false,

    // ── Notice ──
    unreadNoticeCount: 0,

    // ── Command history (per-server, session only) ──
    commandHistoryMap: {},  // { [serverId]: [...entries] }

    // ── Background ──
    backgroundDaemonStatus: 'checking',

    // ── Actions ──
    openModal: (config) => set({ modal: config }),
    closeModal: () => set({ modal: null }),

    setProgressBar: (config) => set({ progressBar: config }),
    clearProgressBar: () => set({ progressBar: null }),

    setShowWaitingImage: (val) => set({ showWaitingImage: val }),

    setShowModuleManager: (val) => set({ showModuleManager: val }),

    // 설정은 이제 페이지다. 이름은 호출부 호환을 위해 그대로 둔다.
    openSettings: (initialView = null) =>
        set({
            activeTab: 'settings',
            showGuiSettingsModal: true,
            settingsInitialView: initialView,
        }),
    closeSettings: () =>
        set({
            activeTab: 'servers',
            showGuiSettingsModal: false,
            settingsInitialView: null,
        }),

    showServerTab: (serverId, tab) =>
        set({ activeTab: 'servers', activeServerId: serverId, requestedServerTab: tab }),
    clearRequestedServerTab: () => set({ requestedServerTab: null }),
    revealServer: (serverId) =>
        set((state) => ({
            activeTab: 'servers',
            activeServerId: serverId,
            revealServerCount: state.revealServerCount + 1,
        })),

    setShowCommandModal: (val) => set({ showCommandModal: val }),
    setCommandServer: (server) => set({ commandServer: server }),

    getCommandHistory: (serverId) => _get().commandHistoryMap[serverId] || [],
    pushCommandHistory: (serverId, entry) =>
        set((state) => ({
            commandHistoryMap: {
                ...state.commandHistoryMap,
                [serverId]: [...(state.commandHistoryMap[serverId] || []), entry],
            },
        })),

    setActiveTab: (tab) => set({ activeTab: tab }),
    setActiveServerId: (id) => set({ activeServerId: id }),

    setShowDiscordSection: (val) => set({ showDiscordSection: val }),
    setShowBackgroundSection: (val) => set({ showBackgroundSection: val }),
    setShowNoticeSection: (val) => set({ showNoticeSection: val }),

    setUnreadNoticeCount: (count) => set({ unreadNoticeCount: count }),

    setBackgroundDaemonStatus: (status) => set({ backgroundDaemonStatus: status }),

    togglePanel: (panelName) =>
        set((state) => ({
            [panelName]: !state[panelName],
        })),

    // Test-only: reset to initial state
    _resetForTest: () =>
        set({
            activeTab: 'servers',
            activeServerId: null,
            revealServerCount: 0,
            requestedServerTab: null,
            modal: null,
            progressBar: null,
            showWaitingImage: false,
            showModuleManager: false,
            showGuiSettingsModal: false,
            settingsInitialView: null,
            showCommandModal: false,
            commandServer: null,
            showDiscordSection: false,
            showBackgroundSection: false,
            showNoticeSection: false,
            unreadNoticeCount: 0,
            backgroundDaemonStatus: 'checking',
        }),
}));

// ── Vite HMR: preserve store state across hot module replacement ──
if (import.meta.hot) {
    import.meta.hot.dispose((data) => {
        const s = useUIStore.getState();
        data.prevState = {
            progressBar: s.progressBar,
            showModuleManager: s.showModuleManager,
            showDiscordSection: s.showDiscordSection,
            showBackgroundSection: s.showBackgroundSection,
            showNoticeSection: s.showNoticeSection,
            unreadNoticeCount: s.unreadNoticeCount,
            backgroundDaemonStatus: s.backgroundDaemonStatus,
        };
    });
    if (import.meta.hot.data?.prevState) {
        useUIStore.setState(import.meta.hot.data.prevState);
    }
}
