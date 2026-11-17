/**
 * Tauri 셸에서 window.api를 만든다.
 * window.__TAURI__가 있고 window.api가 아직 없을 때만 설치한다 (테스트·개발 mock은 그대로).
 * src/index.tsx가 가장 먼저 import한다.
 */
import { createApi } from './createApi';
import { createEventHub } from './events';
import { tauriBindings } from './tauri';

/** 설치했으면 true. 이벤트 구독과 app_ready까지 끝나면 resolve한다 */
export async function installTauriBridge(target = typeof window !== 'undefined' ? window : undefined) {
    if (!target?.__TAURI__ || target.api) return false;
    const { invoke, listen } = tauriBindings(target.__TAURI__);
    const hub = createEventHub();
    const search = new URLSearchParams(target.location?.search || '');
    // 포트 충돌 정지 알림은 메인 창만 받아 띄운다 (새로 연 팝아웃·팝업 창이 지난 알림을 다시 띄우지 않게)
    const mainWindow = !['console-popout', 'tray-popup', 'overlay-menu', 'desktop-overlay'].some((k) => search.has(k));
    target.api = createApi({ invoke, hub, mainWindow }).api;

    // 이벤트는 메인 창에만 온다(계약 §3). 콘솔 팝아웃·트레이 팝업·펫 우클릭 판 창은 초기화를 다시 부르지 않는다.
    // 콘솔 팝아웃은 자기가 연 콘솔 스트림(셸이 그 창에 보내는 daemon:stream)만 받는다
    if (search.has('console-popout')) {
        await hub.start(listen, ['daemon:stream']).catch((e) => console.error('[Bridge] Failed to subscribe streams:', e));
        return true;
    }
    if (search.has('tray-popup') || search.has('overlay-menu')) return true;

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
