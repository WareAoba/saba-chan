import { create } from 'zustand';

export const useUIStore = create((set, _get) => ({
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
    contextMenu: null,

    // ── Notice ──
    unreadNoticeCount: 0,

    // ── Background ──
    backgroundDaemonStatus: 'checking',

    // ── Actions ──
    openModal: (config) => set({ modal: config }),
    closeModal: () => set({ modal: null }),

    setProgressBar: (config) => set({ progressBar: config }),
    clearProgressBar: () => set({ progressBar: null }),

    setShowWaitingImage: (val) => set({ showWaitingImage: val }),

    setShowModuleManager: (val) => set({ showModuleManager: val }),

    openSettings: (initialView = null) =>
        set({
            showGuiSettingsModal: true,
            settingsInitialView: initialView,
        }),
    closeSettings: () =>
        set({
            showGuiSettingsModal: false,
            settingsInitialView: null,
        }),

    setShowCommandModal: (val) => set({ showCommandModal: val }),
    setCommandServer: (server) => set({ commandServer: server }),

    setShowDiscordSection: (val) => set({ showDiscordSection: val }),
    setShowBackgroundSection: (val) => set({ showBackgroundSection: val }),
    setShowNoticeSection: (val) => set({ showNoticeSection: val }),

    setContextMenu: (menu) => set({ contextMenu: menu }),

    setUnreadNoticeCount: (count) => set({ unreadNoticeCount: count }),

    setBackgroundDaemonStatus: (status) => set({ backgroundDaemonStatus: status }),

    togglePanel: (panelName) =>
        set((state) => ({
            [panelName]: !state[panelName],
        })),

    // Test-only: reset to initial state
    _resetForTest: () =>
        set({
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
            contextMenu: null,
            unreadNoticeCount: 0,
            backgroundDaemonStatus: 'checking',
        }),
}));
