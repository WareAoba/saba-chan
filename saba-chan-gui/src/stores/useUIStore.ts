import { create } from 'zustand';

/** 상단 페이지 */
/** 상단 페이지. `ext:<id>`는 익스텐션 화면 (`App.page` slot) */
export type ActiveTab = 'servers' | 'discord' | 'activity' | 'settings' | `ext:${string}`;

/** 디스코드 페이지에 "이 화면을 열어 달라"는 요청 (화면 주소 링크). seq는 같은 화면을 다시 눌러도 다시 열게 한다 */
export interface DiscordViewRequest {
    kind: 'connection' | 'saba' | 'node';
    /** 디스코드 서버 id. *는 연결된 디스코드 서버가 하나일 때 그 서버 */
    id?: string;
    tab?: string;
    sub?: string;
    seq: number;
}

/**
 * App이 그리는 모달 하나. type이 모달 종류(success·failure·question·agreements…)이고 나머지 칸은 그 모달의 props다.
 */
export interface ModalConfig {
    type: string;
    title?: string;
    message?: string;
    // biome-ignore lint/suspicious/noExplicitAny: 모달 종류마다 props가 다르다
    [key: string]: any;
}

export interface ProgressBarConfig {
    message?: string;
    percent?: number;
    indeterminate?: boolean;
}

/** 콘솔 명령 기록 한 줄 (CommandModal) */
// biome-ignore lint/suspicious/noExplicitAny: 기록 모양은 CommandModal이 정한다
export type CommandHistoryEntry = Record<string, any>;

type Panel = 'showModuleManager' | 'showDiscordSection' | 'showBackgroundSection' | 'showNoticeSection';

interface UIState {
    activeTab: ActiveTab;
    activeServerId: string | null;
    requestedServerTab: string | null;
    requestedDiscordView: DiscordViewRequest | null;
    /** 디스코드 페이지에 지금 열린 디스코드 서버 (명령 팔레트가 빈 검색의 기준으로 쓴다) */
    openDiscordNodeId: string | null;
    /** 명령 팔레트가 연 화면에서 찾아 줄 서버 설정 칸 (고급 묶음이면 펼친다). seq가 바뀌면 다시 */
    findSettingField: { name: string; seq: number } | null;
    revealServerCount: number;
    modal: ModalConfig | null;
    progressBar: ProgressBarConfig | null;
    showWaitingImage: boolean;
    showModuleManager: boolean;
    settingsInitialView: string | null;
    showCommandModal: boolean;
    // biome-ignore lint/suspicious/noExplicitAny: 명령 창이 다루는 서버 (서버 목록 항목에 화면 칸을 더한 것)
    commandServer: any;
    showDiscordSection: boolean;
    showBackgroundSection: boolean;
    showNoticeSection: boolean;
    unreadNoticeCount: number;
    commandHistoryMap: Record<string, CommandHistoryEntry[]>;
    /** checking · running · stopped · … */
    backgroundDaemonStatus: string;

    openModal: (config: ModalConfig) => void;
    closeModal: () => void;
    setProgressBar: (config: ProgressBarConfig | null) => void;
    clearProgressBar: () => void;
    setShowWaitingImage: (val: boolean) => void;
    setShowModuleManager: (val: boolean) => void;
    openSettings: (initialView?: string | null) => void;
    closeSettings: () => void;
    showServerTab: (serverId: string, tab: string) => void;
    clearRequestedServerTab: () => void;
    showDiscordView: (view: Omit<DiscordViewRequest, 'seq'>) => void;
    clearRequestedDiscordView: () => void;
    revealServer: (serverId: string) => void;
    setOpenDiscordNodeId: (id: string | null) => void;
    requestFindSettingField: (name: string) => void;
    setShowCommandModal: (val: boolean) => void;
    setCommandServer: (server: UIState['commandServer']) => void;
    getCommandHistory: (serverId: string) => CommandHistoryEntry[];
    pushCommandHistory: (serverId: string, entry: CommandHistoryEntry) => void;
    setActiveTab: (tab: ActiveTab) => void;
    setActiveServerId: (id: string | null) => void;
    setShowDiscordSection: (val: boolean) => void;
    setShowBackgroundSection: (val: boolean) => void;
    setShowNoticeSection: (val: boolean) => void;
    setUnreadNoticeCount: (count: number) => void;
    setBackgroundDaemonStatus: (status: string) => void;
    togglePanel: (panelName: Panel) => void;
    _resetForTest: () => void;
}

let discordViewSeq = 0;

export const useUIStore = create<UIState>()((set, _get) => ({
    // ── Navigation ──
    // activeTab = 상단 페이지 (servers | discord | settings), activeServerId = 서버 페이지에서 고른 서버
    activeTab: 'servers',
    activeServerId: null,
    // 서버 상세에 "이 탭을 열어 달라"는 요청 (우클릭 메뉴 등). 상세가 읽고 비운다.
    requestedServerTab: null,
    // 디스코드 페이지에 "이 화면을 열어 달라"는 요청 (화면 주소 링크). 페이지가 읽고 비운다.
    requestedDiscordView: null,
    openDiscordNodeId: null,
    findSettingField: null,
    // 바깥(CLI의 `gui`)에서 서버를 연 횟수. 좁은 창에서도 목록 대신 그 서버 상세를 보인다
    revealServerCount: 0,

    // ── Modal ──
    modal: null,
    progressBar: null,
    showWaitingImage: false,

    // ── Panel visibility ──
    showModuleManager: false,
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
            settingsInitialView: initialView,
        }),
    closeSettings: () =>
        set({
            activeTab: 'servers',
            settingsInitialView: null,
        }),

    showServerTab: (serverId, tab) =>
        set({ activeTab: 'servers', activeServerId: serverId, requestedServerTab: tab }),
    clearRequestedServerTab: () => set({ requestedServerTab: null }),
    showDiscordView: (view) => {
        discordViewSeq += 1;
        set({ activeTab: 'discord', requestedDiscordView: { ...view, seq: discordViewSeq } });
    },
    clearRequestedDiscordView: () => set({ requestedDiscordView: null }),
    revealServer: (serverId) =>
        set((state) => ({
            activeTab: 'servers',
            activeServerId: serverId,
            revealServerCount: state.revealServerCount + 1,
        })),

    setOpenDiscordNodeId: (id) => set({ openDiscordNodeId: id }),
    requestFindSettingField: (name) =>
        set((state) => ({ findSettingField: { name, seq: (state.findSettingField?.seq ?? 0) + 1 } })),

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

    // 설정을 떠나면 "이 탭을 열어 달라"는 요청도 지운다: 다음에 설정을 열 때 따라오지 않게
    setActiveTab: (tab) => set(tab === 'settings' ? { activeTab: tab } : { activeTab: tab, settingsInitialView: null }),
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
            requestedDiscordView: null,
            openDiscordNodeId: null,
            findSettingField: null,
            modal: null,
            progressBar: null,
            showWaitingImage: false,
            showModuleManager: false,
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
