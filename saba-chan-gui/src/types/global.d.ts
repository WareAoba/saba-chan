/**
 * 앱이 window에 거는 전역. window.api·window.electron은 api/install(Tauri)·dev/mockApi(브라우저 미리보기)·테스트가 건다.
 */
import type * as ReactNamespace from 'react';
import type * as ReactDOMClient from 'react-dom/client';
import type { SabaApi, SabaElectron } from '../api/createApi';
import type { TauriGlobal } from '../api/tauri';

export type ToastType = 'info' | 'success' | 'warning' | 'error';

export interface ToastOptions {
    /** 알림 모음에도 남긴다 */
    isNotice?: boolean;
    /** 'saba-chan' 또는 서버 모듈 이름 */
    source?: string;
}

/** 알림 모음 한 칸 (components/Modals/NoticeModal) */
export interface Notice {
    id: number;
    message: string;
    type: 'info' | 'success' | 'error';
    source: string;
    /** 누르면 할 일 ('openUpdateModal' 등) */
    action: string | null;
    data: unknown;
    timestamp: string;
    read: boolean;
}

export interface NoticeInput {
    message: string;
    type?: Notice['type'];
    source?: string;
    action?: string | null;
    /** 같은 source·action(없으면 같은 문구)이 있으면 새로 만들지 않고 그것을 고친다 */
    dedup?: boolean;
    data?: unknown;
}

export interface SabaNoticeApi {
    addNotice: (notice: NoticeInput) => Notice;
    removeNotice: (id: number) => void;
    removeNoticesByAction: (source: string, action: string) => void;
    clearAllNotices: () => void;
    getUnreadCount: () => number;
    loadNotices: () => Notice[];
    markAllRead: () => void;
}

declare global {
    interface Window {
        api: SabaApi;
        electron: SabaElectron;
        /** Tauri 셸(withGlobalTauri)에서만 있다 */
        __TAURI__?: TauriGlobal;
        /** Toast가 그려진 뒤에 생긴다. 토스트 id를 돌려준다 */
        showToast?: (message: string, type?: ToastType, duration?: number, options?: ToastOptions) => number;
        updateToast?: (id: number, message: string, type: ToastType, duration?: number) => void;
        showStatus?: (step: string, message: string, duration?: number) => void;
        __sabaNotice?: SabaNoticeApi;
        __sabaReadyReceived?: boolean;
        /** 익스텐션 UMD 번들이 쓰는 전역 (index) */
        React: typeof ReactNamespace;
        ReactDOM: typeof ReactDOMClient;
        /** 익스텐션 UMD 번들이 거는 전역: SabaExt{PascalId} */
        [extensionGlobal: `SabaExt${string}`]: unknown;
    }
}
