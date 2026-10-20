import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import i18n from '../i18n';
import { showEventToast } from '../utils/daemonEvents';

const t = i18n.getFixedT('en', 'gui');

// G-S6: 크래시 한 번을 이벤트(instance.exit)와 목록 폴링(실행 → 멈춤)이 따로 알려 알림이 두 번 떴다
describe('크래시 알림은 한 번만', () => {
    let servers;
    const setServers = vi.fn((update) => {
        servers = typeof update === 'function' ? update(servers) : update;
    });
    const running = { id: 'c1', name: 'Crashy', module: 'game', status: 'running' };
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
                closeConsole: vi.fn(),
                setShowModuleManager: vi.fn(),
                formatUptime: vi.fn(),
            }),
        );
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        servers = [];
        window.api = { serverList: vi.fn().mockResolvedValue({ servers: [running] }) };
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    const errorToasts = () => window.showToast.mock.calls.filter((c) => c[1] === 'error' || c[1] === 'warning');

    it('이벤트가 먼저 알렸으면 폴링은 다시 알리지 않는다', async () => {
        const { result } = hook();
        await act(async () => result.current.fetchServers());
        await act(async () => vi.advanceTimersByTime(31_000));
        showEventToast({ type: 'instance.exit', data: { instanceId: 'c1', instance: 'Crashy', kind: 'crash' } }, t);
        window.api.serverList.mockResolvedValue({ servers: [{ ...running, status: 'stopped' }] });
        await act(async () => result.current.fetchServers());
        expect(errorToasts()).toHaveLength(1);
    });

    it('폴링이 먼저 알렸으면 늦게 온 이벤트는 다시 알리지 않는다', async () => {
        const { result } = hook();
        await act(async () => result.current.fetchServers());
        await act(async () => vi.advanceTimersByTime(31_000));
        window.api.serverList.mockResolvedValue({ servers: [{ ...running, id: 'c2', status: 'stopped' }] });
        servers = servers.map((s) => ({ ...s, id: 'c2' }));
        await act(async () => result.current.fetchServers());
        showEventToast({ type: 'instance.exit', data: { instanceId: 'c2', instance: 'Crashy', kind: 'crash' } }, t);
        expect(errorToasts()).toHaveLength(1);
    });
});
