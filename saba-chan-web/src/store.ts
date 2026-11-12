import { reuseUnchanged } from '@gui/utils/serverView';
import { create } from 'zustand';
import type { Module, Server, SessionState } from './types';

export type Tab = string;
export type ToastTone = 'info' | 'ok' | 'error';

export interface ConfirmRequest {
    title: string;
    message?: string;
    confirmLabel?: string;
    danger?: boolean;
}

interface AppState {
    session: SessionState;
    tab: Tab;
    serverId: string | null;
    /** 화면 주소 링크로 연 서버 탭 (서버 상세가 처음 열 때 읽는다) */
    serverTab: string | null;
    toasts: { id: string; message: string; tone: ToastTone }[];
    confirm: (ConfirmRequest & { done: (ok: boolean) => void }) | null;
    installBandClosed: boolean;
    setSession: (session: SessionState) => void;
    setTab: (tab: Tab) => void;
    openServer: (serverId: string | null, tab?: string | null) => void;
    closeInstallBand: () => void;
    toast: (message: string, tone?: ToastTone) => void;
    ask: (request: ConfirmRequest) => Promise<boolean>;
}

interface DataState {
    servers: Server[];
    serversLoaded: boolean;
    serversError: string | null;
    modules: Module[];
    modulesLoaded: boolean;
    setServers: (servers: Server[], error?: string | null) => void;
    setModules: (modules: Module[]) => void;
}

/** 화면 상태 하나: 세션, 고른 탭·서버, 알림, 확인 창 */
export const useApp = create<AppState>()((set, get) => ({
    session: { phase: 'connecting', reason: null, expiresAt: null, hostId: null, warning: false },
    tab: 'servers',
    serverId: null,
    serverTab: null,
    toasts: [],
    confirm: null,
    /** PC 없는 세션의 설치 안내 띠 — 닫으면 이 세션 동안 다시 나오지 않는다 */
    installBandClosed: false,

    setSession: (session) => set({ session }),
    setTab: (tab) => set({ tab, serverId: null, serverTab: null }),
    openServer: (serverId, tab = null) => set({ serverId, serverTab: tab }),
    closeInstallBand: () => set({ installBandClosed: true }),

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
export const useData = create<DataState>()((set) => ({
    servers: [],
    serversLoaded: false,
    serversError: null,
    modules: [],
    modulesLoaded: false,
    // 내용이 그대로인 서버는 이전 객체를 쓴다: 4초마다 받아도 바뀐 게 없으면 화면을 다시 그리지 않는다
    setServers: (servers, error = null) =>
        set((state) => ({ servers: reuseUnchanged(state.servers, servers), serversError: error, serversLoaded: true })),
    setModules: (modules) => set({ modules, modulesLoaded: true }),
}));
