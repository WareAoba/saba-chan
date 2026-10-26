import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';

// 디스코드·CLI로 켜고 끈 서버를 목록 폴링이 "사바쨩 밖에서 시작"·"예기치 않게 종료"로 알렸다.
// 데몬 이벤트(instance.started·stopped·exit)가 출처를 싣고 오므로, 이벤트를 받는 앱에서는 폴링이 알리지 않는다
describe('목록 폴링의 상태 변화 알림', () => {
    let servers;
    const setServers = vi.fn((update) => {
        servers = typeof update === 'function' ? update(servers) : update;
    });
    const srv = { id: 's1', name: 'Survival', module: 'game', status: 'stopped' };
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
    const list = (patch) => window.api.serverList.mockResolvedValue({ servers: [{ ...srv, ...patch }] });
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        servers = [];
        window.api = { serverList: vi.fn() };
        list({});
        window.showToast = vi.fn();
    });
    afterEach(() => vi.useRealTimers());

    const flip = async (result) => {
        await act(async () => result.current.fetchServers());
        await act(async () => vi.advanceTimersByTime(31_000));
        list({ status: 'running', last_api_action: Date.now() });
        await act(async () => result.current.fetchServers());
        list({ status: 'stopped', last_api_action: Date.now() });
        await act(async () => result.current.fetchServers());
    };

    it('데몬 이벤트를 받으면 폴링은 켜짐·꺼짐을 알리지 않는다', async () => {
        window.api.onDaemonEvent = vi.fn(() => () => {});
        const { result } = hook();
        await flip(result);
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('이벤트가 없어도 사바쨩을 거쳐 켜고 끈 것은 밖에서 켰다거나 예기치 않게 꺼졌다고 하지 않는다', async () => {
        const { result } = hook();
        await flip(result);
        expect(window.showToast).not.toHaveBeenCalled();
    });
});
