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
    /** 'saba-chan' 또는 서버 이름 */
    source?: string;
    /** 알림 모음에 남길 때 토스트 문구 대신 쓸 칸 (서버별로 묶을 짧은 줄·이유·서버 id) */
    notice?: Partial<NoticeInput>;
}

/** 알림 모음 한 칸 (components/Modals/NoticeModal) */
export interface Notice {
    id: number;
    message: string;
    type: 'info' | 'success' | 'error';
    /** 묶음 이름: 'saba-chan', 'Updater', 또는 서버 이름(알릴 때의 이름) */
    source: string;
    /** 서버 알림이면 그 서버 id: 이것으로 묶고, 묶음 이름은 지금 이름을 쓴다 */
    instanceId?: string;
    /** 둘째 줄 (이유 등) */
    detail?: string;
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
    instanceId?: string;
    detail?: string;
    action?: string | null;
    /** 같은 source·action(없으면 같은 문구)이 있으면 새로 만들지 않고 그것을 고친다 */
    dedup?: boolean;
    data?: unknown;
    /** 이미 본 일 (앱에서 직접 한 일): 읽지 않은 수에 넣지 않는다 */
    read?: boolean;
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
