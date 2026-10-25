import { create } from 'zustand';

/** 화면 상태 하나: 세션, 고른 탭·서버, 알림, 확인 창 */
export const useApp = create((set, get) => ({
    session: { phase: 'connecting', reason: null, expiresAt: null, hostId: null, warning: false },
    tab: 'servers',
    serverId: null,
    toasts: [],
    confirm: null,

    setSession: (session) => set({ session }),
    setTab: (tab) => set({ tab, serverId: null }),
    openServer: (serverId) => set({ serverId }),

    toast: (message, tone = 'info') => {
        const id = Math.random().toString(36).slice(2);
        set({ toasts: [...get().toasts, { id, message, tone }] });
        setTimeout(() => set({ toasts: get().toasts.filter((t) => t.id !== id) }), tone === 'error' ? 6000 : 3500);
    },

    /** GUI 와 같은 확인 창. 누른 버튼이 확인이면 true */
    ask: ({ title, message, confirmLabel, danger = false }) =>
        new Promise((resolve) => {
            set({
                confirm: {
                    title,
                    message,
                    confirmLabel,
                    danger,
                    done: (ok) => {
                        set({ confirm: null });
                        resolve(ok);
                    },
                },
            });
        }),
}));

/** PC 에서 읽어 온 것 */
export const useData = create((set) => ({
    servers: [],
    serversLoaded: false,
    serversError: null,
    modules: [],
    modulesLoaded: false,
    setServers: (servers, error = null) => set({ servers, serversError: error, serversLoaded: true }),
    setModules: (modules) => set({ modules, modulesLoaded: true }),
}));
