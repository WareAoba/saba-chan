/**
 * 릴레이 /remote/ws 세션 (server-chan relay-server/src/services/webHub.ts, 계약은 docs/architecture/05 §4.4(f)).
 *
 * 첫 프레임은 {t:'hello', link}. 열리면 {t:'ready', sid, key, expiresAt, hostId}.
 * 네트워크로 끊기면 30초 안에 {t:'resume', sid, key}로 다시 붙는다. 릴레이가 끝낸 세션(closed·error)은 다시 붙지 않는다.
 */

/** 릴레이가 끊긴 세션을 기다리는 시간(30초)보다 조금 짧게 */
export const RECONNECT_WINDOW_MS = 25_000;
/** 릴레이는 2분이 지나면 504로 답한다. 그 답도 오지 않을 때만 여기서 끊는다 */
const REQUEST_LIMIT_MS = 2 * 60_000 + 10_000;

export function createSession({ url, link, WebSocketImpl = globalThis.WebSocket, onState, onEvent }) {
    let state = { phase: 'connecting', reason: null, expiresAt: null, hostId: null, warning: false };
    let ws = null;
    let sid = null;
    let key = null;
    let ended = false;
    let lostAt = 0;
    let retry = null;
    let nextId = 1;
    const pending = new Map();

    const set = (patch) => {
        state = { ...state, ...patch };
        onState?.(state);
    };

    const send = (frame) => {
        if (ws?.readyState === 1) ws.send(JSON.stringify(frame));
    };

    const failPending = (code) => {
        for (const [id, p] of pending) {
            clearTimeout(p.timer);
            p.resolve({ status: 0, code, error: code });
            pending.delete(id);
        }
    };

    const end = (reason) => {
        if (ended) return;
        ended = true;
        clearTimeout(retry);
        failPending('ECONNRESET');
        set({ phase: 'closed', reason, warning: false });
    };

    function connect() {
        const sock = new WebSocketImpl(url);
        ws = sock;
        sock.onopen = () => {
            if (sid) sock.send(JSON.stringify({ t: 'resume', sid, key }));
            else sock.send(JSON.stringify({ t: 'hello', link }));
        };
        sock.onmessage = (msg) => {
            let frame;
            try {
                frame = JSON.parse(msg.data);
            } catch {
                return;
            }
            switch (frame.t) {
                case 'ready':
                    sid = frame.sid;
                    key = frame.key;
                    lostAt = 0;
                    set({ phase: 'ready', expiresAt: frame.expiresAt, hostId: frame.hostId ?? state.hostId });
                    break;
                case 'res': {
                    const p = pending.get(frame.id);
                    if (!p) break;
                    clearTimeout(p.timer);
                    pending.delete(frame.id);
                    p.resolve({ status: frame.status, data: frame.body });
                    break;
                }
                case 'event':
                    onEvent?.({ type: frame.kind, data: frame.data ?? {} });
                    break;
                case 'expiring':
                    set({ warning: true, expiresAt: frame.expiresAt });
                    break;
                case 'extended':
                    set({ warning: false, expiresAt: frame.expiresAt });
                    break;
                case 'closed':
                case 'error':
                    end(frame.reason || frame.code || 'closed');
                    break;
            }
        };
        sock.onclose = () => {
            if (ws !== sock || ended) return;
            failPending('ECONNRESET');
            // 처음 여는 중에 끊겼다: 링크는 이미 썼을 수 있어 다시 열 수 없다
            if (!sid) return end('connection_lost');
            if (!lostAt) lostAt = Date.now();
            if (Date.now() - lostAt >= RECONNECT_WINDOW_MS) return end('connection_lost');
            set({ phase: 'reconnecting' });
            retry = setTimeout(connect, 1000);
        };
    }

    onState?.(state);
    connect();
    // 다시 붙는 동안 시간이 다 되면 끝낸다 (소켓이 열리지도 닫히지도 않고 걸려 있을 때)
    const watchdog = setInterval(() => {
        if (ended) return clearInterval(watchdog);
        if (lostAt && Date.now() - lostAt >= RECONNECT_WINDOW_MS) {
            ws?.close?.();
            end('connection_lost');
        }
    }, 1000);

    return {
        /** GUI 브리지 daemon_request 와 같은 모양: {status, data} 또는 연결 실패 {status:0, code} */
        request(method, path, body, timeoutMs = REQUEST_LIMIT_MS) {
            if (state.phase !== 'ready' || ws?.readyState !== 1) {
                return Promise.resolve({ status: 0, code: 'ECONNREFUSED', error: 'not connected' });
            }
            const id = nextId++;
            return new Promise((resolve) => {
                const timer = setTimeout(
                    () => {
                        pending.delete(id);
                        resolve({ status: 0, code: 'ETIMEDOUT', error: 'timeout' });
                    },
                    Math.min(timeoutMs, REQUEST_LIMIT_MS),
                );
                pending.set(id, { resolve, timer });
                const frame = { t: 'req', id, method: method.toUpperCase(), path };
                if (body !== undefined) frame.body = body;
                send(frame);
            });
        },
        extend() {
            send({ t: 'extend' });
        },
        leave() {
            send({ t: 'bye' });
            ended = true;
            clearTimeout(retry);
            ws?.close(1000);
            failPending('ECONNRESET');
            set({ phase: 'closed', reason: 'left', warning: false });
        },
    };
}
