/**
 * 꺼진 서버의 콘솔: 지난 로그는 그대로 보이고, 꺼져 있다는 표시가 위에 붙는다.
 * 끄기를 눌러도 콘솔 기록을 비우지 않고, 처음 열 때는 5초를 기다리지 않고 바로 읽는다.
 */
import { act, render, renderHook, screen } from '@testing-library/react';
import i18n from 'i18next';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ServerConsole } from '../components/servers/ServerConsole';
import { useMultiConsole } from '../hooks/useMultiConsole';
import { useServerActions } from '../hooks/useServerActions';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';

const consoleOf = (server, lines) =>
    render(
        <ServerConsole
            server={server}
            state={{ lines, input: '' }}
            quickCommands={[]}
            openConsole={() => {}}
            setConsoleInput={() => {}}
            sendConsoleCommand={() => {}}
        />,
    );

describe('꺼진 서버의 콘솔', () => {
    afterEach(() => {
        useConsoleStore.setState({ consoles: {} });
        vi.useRealTimers();
    });

    it('지난 로그를 보이고 꺼져 있다는 표시를 붙인다', () => {
        consoleOf({ id: 'a', name: 'A', status: 'stopped' }, [{ id: 0, content: 'Done (6.2s)!', key: 1 }]);
        expect(screen.getByText('Done (6.2s)!')).toBeTruthy();
        expect(i18n.t('gui:shell.console_offline_history')).not.toBe('shell.console_offline_history');
        expect(screen.getByRole('status').textContent).toContain(i18n.t('gui:shell.console_offline_history'));
    });

    it('켜져 있으면 꺼짐 표시가 없다', () => {
        consoleOf({ id: 'a', name: 'A', status: 'running' }, [{ id: 0, content: 'tick', key: 1 }]);
        expect(screen.queryByText(i18n.t('gui:shell.console_offline_history'))).toBeNull();
    });

    it('처음 열면 꺼진 서버도 기다리지 않고 지난 로그를 읽는다', async () => {
        vi.useFakeTimers();
        useServerStore.setState({ servers: [{ id: 'a', name: 'A', pid: 0 }] });
        window.api.managedConsole = vi.fn(async () => ({
            source: 'disk',
            lines: [{ id: 0, content: 'old line' }],
        }));
        const { result } = renderHook(() =>
            useMultiConsole({ isPopoutMode: false, popoutParams: null, consoleBufferRef: { current: 2000 } }),
        );
        act(() => result.current.openConsole('a', 'A'));
        await act(async () => vi.advanceTimersByTimeAsync(50));
        expect(useConsoleStore.getState().consoles.a.lines.map((l) => l.content)).toEqual(['old line']);
        result.current.closeConsole('a');
    });

    it('끄기를 눌러도 콘솔을 닫지 않는다 (지난 로그가 남는다)', async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        let servers = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 5 }];
        const closeConsole = vi.fn();
        const setModal = vi.fn();
        window.api = {
            ...window.api,
            serverList: vi.fn().mockResolvedValue({ servers }),
            instanceStop: vi.fn().mockResolvedValue({ success: true }),
            instanceStatus: vi.fn().mockResolvedValue({ status: 'stopped' }),
        };
        window.showToast = vi.fn();
        const { result } = renderHook(() =>
            useServerActions({
                servers,
                setServers: (u) => {
                    servers = typeof u === 'function' ? u(servers) : u;
                },
                modules: [],
                loading: false,
                setLoading: vi.fn(),
                setModal,
                setProgressBar: vi.fn(),
                openConsole: vi.fn(),
                closeConsole,
                setShowModuleManager: vi.fn(),
            }),
        );
        await act(async () => result.current.handleStop('A'));
        await act(async () => setModal.mock.calls.at(-1)[0].onConfirm());
        await act(async () => vi.advanceTimersByTimeAsync(1500));
        expect(window.api.instanceStop).toHaveBeenCalled();
        expect(closeConsole).not.toHaveBeenCalled();
    });
});
