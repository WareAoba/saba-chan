/**
 * 이벤트 스트림이 끊긴 동안(셸의 `events:live` false)에는 목록 폴링이 켜짐·꺼짐을 알리고,
 * 다시 붙으면 이벤트에 맡긴다. 셸이 아무 말도 하지 않았으면 붙은 것으로 본다
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { createEventHub } from '../api/events';
import { useServerActions } from '../hooks/useServerActions';
import i18n from '../i18n';
import type { Server } from '../stores/useServerStore';
import { eventsLive, showEventToast } from '../utils/daemonEvents';

const t = i18n.getFixedT('en', 'gui');

describe('이벤트 스트림이 끊긴 동안', () => {
    let servers: Server[];
    let shell: Map<string, (payload: unknown) => void>;
    const setServers = vi.fn((update) => {
        servers = typeof update === 'function' ? update(servers) : update;
    });
    const running = { id: 's1', name: 'Survival', module: 'game', status: 'running' };
    const stopped = { ...running, status: 'stopped' };

    beforeEach(async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        servers = [];
        shell = new Map();
        const hub = createEventHub();
        await hub.start(async (name, cb) => {
            shell.set(name, cb);
            return () => {};
        });
        const { api } = createApi({ invoke: vi.fn() as never, hub });
        window.api = { ...api, serverList: vi.fn().mockResolvedValue({ servers: [running] }) } as unknown as typeof window.api;
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    const hook = () =>
        renderHook(() =>
            useServerActions({
                servers,
                setServers,
                modules: [],
                loading: false,
                setLoading: vi.fn(),
                setModal: vi.fn(),
                setProgressBar: vi.fn(),
                openConsole: vi.fn(),
                setShowModuleManager: vi.fn(),
            }),
        );
    const stopToasts = () => vi.mocked(window.showToast as NonNullable<typeof window.showToast>).mock.calls.filter((c) => c[1] === 'error');
    const pollOnce = async (result: { current: { fetchServers: () => Promise<unknown> } }, list: object[]) => {
        vi.mocked(window.api.serverList).mockResolvedValue({ servers: list } as never);
        await act(async () => result.current.fetchServers());
    };

    it('붙어 있으면 이벤트가 알리고, 끊기면 폴링이 알리며, 다시 붙으면 이벤트에 맡긴다', async () => {
        expect(eventsLive()).toBe(true);
        const { result } = hook();
        await pollOnce(result, [running]);
        await act(async () => vi.advanceTimersByTime(31_000)); // 시작 직후 유예

        await pollOnce(result, [stopped]);
        expect(stopToasts()).toHaveLength(0);

        await pollOnce(result, [running]);
        shell.get('events:live')?.(false);
        expect(eventsLive()).toBe(false);
        await pollOnce(result, [stopped]);
        expect(stopToasts()).toHaveLength(1);

        await pollOnce(result, [running]);
        shell.get('events:live')?.(true);
        expect(eventsLive()).toBe(true);
        await act(async () => vi.advanceTimersByTime(31_000)); // 앞 알림과 짝짓는 시간이 지나도
        await pollOnce(result, [stopped]);
        expect(stopToasts()).toHaveLength(1);
    });

    // 다시 붙으면 데몬이 끊긴 동안의 이벤트를 다시 보낸다(Last-Event-ID). 폴링이 이미 알린 일은 두 번 알리지 않는다
    it('끊긴 동안 폴링이 알린 멈춤·밖의 시작은 다시 붙은 뒤 밀려 온 이벤트가 다시 알리지 않는다', async () => {
        const other = { id: 's2', name: 'Creative', module: 'game', status: 'running' };
        const at = () => Math.floor(Date.now() / 1000);
        const toasts = () => vi.mocked(window.showToast as NonNullable<typeof window.showToast>).mock.calls.length;
        const { result } = hook();
        await pollOnce(result, [other]);
        await act(async () => vi.advanceTimersByTime(31_000)); // 시작 직후 유예

        shell.get('events:live')?.(false);
        const stoppedAt = at();
        await pollOnce(result, [{ ...other, status: 'stopped' }]);
        await act(async () => vi.advanceTimersByTime(5_000));
        const startedAt = at();
        await pollOnce(result, [other]);
        expect(toasts()).toBe(2);

        await act(async () => vi.advanceTimersByTime(60_000));
        shell.get('events:live')?.(true);
        showEventToast({ type: 'instance.exit', data: { instanceId: 's2', instance: 'Creative', kind: 'crash', at: stoppedAt } }, t);
        showEventToast({ type: 'instance.started', data: { instanceId: 's2', instance: 'Creative', unexpected: true, at: startedAt } }, t);
        expect(toasts()).toBe(2);

        // 다시 붙은 뒤 새로 일어난 크래시는 알린다
        showEventToast({ type: 'instance.exit', data: { instanceId: 's2', instance: 'Creative', kind: 'crash', at: at() } }, t);
        expect(toasts()).toBe(3);
    });
});
