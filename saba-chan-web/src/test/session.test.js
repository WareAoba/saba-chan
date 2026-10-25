import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSession, RECONNECT_WINDOW_MS } from '../session';

/** 릴레이 /remote/ws 흉내: 보낸 프레임을 쌓고, 테스트가 프레임을 밀어 넣는다 */
class FakeSocket {
    static all = [];
    constructor(url) {
        this.url = url;
        this.sent = [];
        this.readyState = 0;
        FakeSocket.all.push(this);
    }
    open() {
        this.readyState = 1;
        this.onopen?.();
    }
    send(text) {
        this.sent.push(JSON.parse(text));
    }
    push(frame) {
        this.onmessage?.({ data: JSON.stringify(frame) });
    }
    close(code = 1000) {
        this.readyState = 3;
        this.onclose?.({ code });
    }
    drop() {
        this.close(1006);
    }
    last(t) {
        return [...this.sent].reverse().find((f) => f.t === t);
    }
}

const latest = () => FakeSocket.all[FakeSocket.all.length - 1];

function open(link = 'L1') {
    const states = [];
    const events = [];
    const session = createSession({
        url: 'wss://example/remote/ws',
        link,
        WebSocketImpl: FakeSocket,
        onState: (s) => states.push(s),
        onEvent: (e) => events.push(e),
    });
    latest().open();
    return { session, states, events, state: () => states[states.length - 1] };
}

function ready(sock, extra = {}) {
    sock.push({ t: 'ready', sid: 's1', key: 'k1', expiresAt: Date.now() + 900_000, hostId: 'h1', ...extra });
}

describe('web session', () => {
    beforeEach(() => {
        FakeSocket.all = [];
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    it('says hello with the link and becomes ready', () => {
        const { state } = open('LINK');
        expect(latest().last('hello')).toEqual({ t: 'hello', link: 'LINK' });
        expect(state().phase).toBe('connecting');
        ready(latest());
        expect(state()).toMatchObject({ phase: 'ready', hostId: 'h1' });
    });

    it('answers a request with the status and body the PC sent', async () => {
        const { session } = open();
        ready(latest());
        const pending = session.request('GET', '/api/servers');
        const req = latest().last('req');
        expect(req).toMatchObject({ method: 'GET', path: '/api/servers' });
        latest().push({ t: 'res', id: req.id, status: 200, body: { servers: [] } });
        await expect(pending).resolves.toEqual({ status: 200, data: { servers: [] } });
    });

    it('a request before the session is ready is a connection failure, not a hang', async () => {
        const { session } = open();
        await expect(session.request('GET', '/api/servers')).resolves.toMatchObject({ status: 0 });
    });

    it('gives up on a request the relay never answers', async () => {
        const { session } = open();
        ready(latest());
        const pending = session.request('POST', '/api/instance/x/start');
        vi.advanceTimersByTime(3 * 60_000);
        await expect(pending).resolves.toMatchObject({ status: 0, code: 'ETIMEDOUT' });
    });

    it('a refused link ends with its reason', () => {
        const { state } = open();
        latest().push({ t: 'error', code: 'link_invalid' });
        latest().close(1008);
        expect(state()).toMatchObject({ phase: 'closed', reason: 'link_invalid' });
    });

    it('shows the end warning and the new end after extending', () => {
        const { session, state } = open();
        ready(latest());
        latest().push({ t: 'expiring', expiresAt: 123 });
        expect(state()).toMatchObject({ warning: true, expiresAt: 123 });
        session.extend();
        expect(latest().last('extend')).toEqual({ t: 'extend' });
        latest().push({ t: 'extended', expiresAt: 456 });
        expect(state()).toMatchObject({ warning: false, expiresAt: 456 });
    });

    it('passes PC events on', () => {
        const { events } = open();
        ready(latest());
        latest().push({ t: 'event', kind: 'instance.exit', data: { id: 'a' } });
        expect(events).toEqual([{ type: 'instance.exit', data: { id: 'a' } }]);
    });

    it('a dropped connection comes back with its key and the new key is kept', () => {
        const { state } = open();
        ready(latest());
        const first = latest();
        first.drop();
        expect(state().phase).toBe('reconnecting');
        vi.advanceTimersByTime(1000);
        const second = latest();
        expect(second).not.toBe(first);
        second.open();
        expect(second.last('resume')).toEqual({ t: 'resume', sid: 's1', key: 'k1' });
        ready(second, { key: 'k2' });
        expect(state().phase).toBe('ready');

        second.drop();
        vi.advanceTimersByTime(1000);
        latest().open();
        expect(latest().last('resume')).toEqual({ t: 'resume', sid: 's1', key: 'k2' });
    });

    it('stops trying after the reconnect window', () => {
        const { state } = open();
        ready(latest());
        latest().drop();
        for (let i = 0; i < 20; i++) {
            vi.advanceTimersByTime(2000);
            latest().drop();
        }
        vi.advanceTimersByTime(RECONNECT_WINDOW_MS);
        expect(state()).toMatchObject({ phase: 'closed', reason: 'connection_lost' });
    });

    it('the relay ending the session does not reconnect', () => {
        const { state } = open();
        ready(latest());
        latest().push({ t: 'closed', reason: 'expired' });
        latest().close(1000);
        vi.advanceTimersByTime(5000);
        expect(FakeSocket.all).toHaveLength(1);
        expect(state()).toMatchObject({ phase: 'closed', reason: 'expired' });
    });

    it('leaving says bye and closes normally', () => {
        const { session } = open();
        ready(latest());
        session.leave();
        expect(latest().last('bye')).toEqual({ t: 'bye' });
        expect(latest().readyState).toBe(3);
    });
});
