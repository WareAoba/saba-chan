/**
 * 앱이 window에 거는 전역. window.api는 api/install(Tauri)·dev/mockApi(브라우저 미리보기)·테스트가 건다.
 */
import type * as ReactNamespace from 'react';
import type * as ReactDOMClient from 'react-dom/client';
import type { SabaApi } from '../api/createApi';
import type { TauriGlobal } from '../api/tauri';

export type ToastType = 'info' | 'success' | 'warning' | 'error';

export interface ToastOptions {
    /** 알림 안의 버튼 하나 (예: 되돌리기). 누르면 실행하고 알림을 닫는다 */
    action?: { label: string; onClick: () => void };
}

/** 알림함 한 칸 (데몬 GET /api/notices → utils/noticeInbox) */
export interface Notice {
    id: number;
    message: string;
    type: 'info' | 'success' | 'error';
    /** 묶음 이름: 서버 이름(알릴 때의 이름) 또는 데몬이 준 "사바쨩" */
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

declare global {
    interface Window {
        api: SabaApi;
        /** Tauri 셸(withGlobalTauri)에서만 있다 */
        __TAURI__?: TauriGlobal;
        /** Toast가 그려진 뒤에 생긴다. 토스트 id를 돌려준다 */
        showToast?: (message: string, type?: ToastType, duration?: number, options?: ToastOptions) => number;
        updateToast?: (id: number, message: string, type: ToastType, duration?: number) => void;
        __sabaReadyReceived?: boolean;
        /** 익스텐션 UMD 번들이 쓰는 전역 (index) */
        React: typeof ReactNamespace;
        ReactDOM: typeof ReactDOMClient;
        /** 익스텐션 UMD 번들이 거는 전역: SabaExt{PascalId} */
        [extensionGlobal: `SabaExt${string}`]: unknown;
    }
}
