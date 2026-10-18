/**
 * Tauri 셸에서 preload 대신 window.api·window.electron을 만든다.
 * window.__TAURI__가 있고 window.api가 아직 없을 때만 설치한다 (Electron preload·테스트·개발 mock은 그대로).
 * src/index.js가 가장 먼저 import한다.
 */
import { createApi } from './createApi';
import { createEventHub } from './events';
import { tauriBindings } from './tauri';

/** 설치했으면 true. 이벤트 구독과 app_ready까지 끝나면 resolve한다 */
export async function installTauriBridge(target = typeof window !== 'undefined' ? window : undefined) {
    if (!target?.__TAURI__ || target.api) return false;
    const { invoke, listen } = tauriBindings(target.__TAURI__);
    const hub = createEventHub();
    const { api, electron } = createApi({ invoke, hub });
    target.api = api;
    target.electron = electron;

    // 이벤트는 메인 창에만 온다(계약 §3). 콘솔 팝아웃 창은 초기화를 다시 부르지 않는다
    const search = target.location?.search || '';
    if (new URLSearchParams(search).has('console-popout')) return true;

    try {
        await hub.start(listen);
    } catch (e) {
        console.error('[Bridge] Failed to subscribe events:', e);
    }
    try {
        await invoke('app_ready');
    } catch (e) {
        console.error('[Bridge] app_ready failed:', e);
    }
    return true;
}

installTauriBridge();
