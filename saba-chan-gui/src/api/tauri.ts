/**
 * window.__TAURI__ 전역(withGlobalTauri) 위의 얇은 감싸개. npm 의존 없이 invoke·listen만 쓴다.
 */
import type { Invoke } from './daemon';

/** 이벤트 하나를 구독하고 해제 함수의 Promise를 돌려준다 */
export type Listen = (name: string, cb: (payload: unknown) => void) => Promise<() => void>;

/** withGlobalTauri가 거는 window.__TAURI__ 중 쓰는 것 */
export interface TauriGlobal {
    core: { invoke: Invoke };
    event: { listen: (name: string, cb: (event: { payload?: unknown }) => void) => Promise<() => void> };
}

export function tauriBindings(tauri: TauriGlobal): { invoke: Invoke; listen: Listen } {
    return {
        invoke: (cmd, args) => tauri.core.invoke(cmd, args),
        // 콜백에는 이벤트 본문(payload)만 넘긴다. 해제 함수의 Promise를 돌려준다
        listen: (name, cb) => tauri.event.listen(name, (event) => cb(event?.payload)),
    };
}
