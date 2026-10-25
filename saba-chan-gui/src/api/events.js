/**
 * Rust 이벤트(계약 §3)를 설치 때 한 번만 구독하고 렌더러 콜백으로 나눠 준다.
 * preload의 ipcRenderer.on / removeAllListeners 의미를 따른다.
 */
export const EVENT_CHANNELS = [
    'status:update',
    'events:message',
    'updates:available',
    'updates:completed',
    'app:closeRequest',
    'console:popoutOpened',
    'console:popoutClosed',
    'console:popinRequest',
    'app:openServer',
];

export function createEventHub() {
    const channels = new Map(EVENT_CHANNELS.map((name) => [name, new Set()]));
    // 늦게 붙는 onStatusUpdate도 놓치지 않게 마지막 초기화 상태를 기억한다
    let lastStatus = null;
    // 구독자가 없을 때 온 닫기 요청 (로드 중 Alt+F4). 첫 구독자에게 넘겨야 셸이 받음 확인을 받는다
    let pendingClose = false;
    // 화면이 뜨기 전에 온 "이 서버를 열어라" (`--open-server`로 뜬 GUI). 첫 구독자에게 넘긴다
    let pendingOpenServer = null;

    const listenersOf = (name) => {
        if (!channels.has(name)) channels.set(name, new Set());
        return channels.get(name);
    };

    function dispatch(name, ...args) {
        for (const cb of [...listenersOf(name)]) {
            try {
                cb(...args);
            } catch (e) {
                console.error(`[Bridge] ${name} listener failed:`, e);
            }
        }
    }

    /** Rust에서 온 이벤트 본문 → 콜백 인자 */
    function receive(name, payload) {
        if (name === 'status:update') lastStatus = payload;
        if (name === 'console:popinRequest') {
            dispatch(name, payload?.instanceId, payload?.serverName);
        } else if (name === 'app:closeRequest') {
            if (listenersOf(name).size === 0) pendingClose = true;
            else dispatch(name);
        } else if (name === 'app:openServer' && listenersOf(name).size === 0) {
            pendingOpenServer = payload;
        } else {
            dispatch(name, payload);
        }
    }

    function on(name, cb) {
        const set = listenersOf(name);
        set.add(cb);
        if (name === 'status:update' && lastStatus) {
            const status = lastStatus;
            queueMicrotask(() => {
                if (set.has(cb)) cb(status);
            });
        }
        if (name === 'app:closeRequest' && pendingClose) {
            pendingClose = false;
            queueMicrotask(() => {
                if (set.has(cb)) cb();
            });
        }
        if (name === 'app:openServer' && pendingOpenServer) {
            const payload = pendingOpenServer;
            pendingOpenServer = null;
            queueMicrotask(() => {
                if (set.has(cb)) cb(payload);
            });
        }
        return () => set.delete(cb);
    }

    /** 모든 채널을 구독한다 (listen: (name, cb) => Promise<unlisten>) */
    async function start(listen) {
        await Promise.all(EVENT_CHANNELS.map((name) => listen(name, (payload) => receive(name, payload))));
    }

    return { on, dispatch, start };
}
