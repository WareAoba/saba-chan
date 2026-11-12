/**
 * window.__TAURI__ 전역(withGlobalTauri) 위의 얇은 감싸개. npm 의존 없이 invoke·listen만 쓴다.
 */
import type { Invoke } from './daemon';

/** 이벤트 하나를 구독하고 해제 함수의 Promise를 돌려준다 */
export type Listen = (name: string, cb: (payload: unknown) => void) => Promise<() => void>;

/** withGlobalTauri가 거는 window.__TAURI__ 중 쓰는 것 */
export interface TauriGlobal {
    core: { invoke: Invoke };
    webviewWindow: {
        getCurrentWebviewWindow: () => {
            listen: (name: string, cb: (event: { payload?: unknown }) => void) => Promise<() => void>;
        };
    };
}

export function tauriBindings(tauri: TauriGlobal): { invoke: Invoke; listen: Listen } {
    return {
        invoke: (cmd, args) => tauri.core.invoke(cmd, args),
        // 이 창에 보낸 것만 받는다: 전역 event.listen은 다른 창(바탕화면 오버레이)에 보낸 것까지 받아 같은 이벤트가 두 번 온다.
        // 콜백에는 이벤트 본문(payload)만 넘긴다. 해제 함수의 Promise를 돌려준다
        listen: (name, cb) => tauri.webviewWindow.getCurrentWebviewWindow().listen(name, (event) => cb(event?.payload)),
    };
}
