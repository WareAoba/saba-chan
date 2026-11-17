/**
 * 화면에 보이지 않는 일을 하지 않는다: 숨긴 탭·바뀌지 않은 값·닿지 않는 데몬 때문에 다시 그리거나 묻지 않는다.
 * 반대로 줄이 화면에 없다고 설치 진행 확인까지 멈추지 않는다.
 */
import { act, fireEvent, render, renderHook } from '@testing-library/react';
import { type ComponentType, memo, Profiler, type ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../components/ExtensionSlot', () => ({ default: () => null, SlotView: () => null, useSlotPages: () => [] }));
vi.mock('../components/servers/ServerConsole', () => ({ ServerConsole: () => null }));
vi.mock('../components/Modals/CommandModal', () => ({ default: () => null }));

import { useSystemMetrics } from '../components/monitor/SystemMonitor';
import { ServersPage as ServersPageImpl } from '../components/servers/ServersPage';
import { ActivityBar } from '../components/shell/ActivityBar';
import { ContextMenuProvider } from '../components/ui/ContextMenu';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';
import { useMaintenanceStore } from '../hooks/useInstanceMaintenance';
import useExtensionInitStatus from '../hooks/useExtensionInitStatus';
import { PageActiveContext } from '../hooks/usePageActive';
import { useWindowAtLeast } from '../hooks/useWindowAtLeast';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { useUIStore } from '../stores/useUIStore';

const ServersPage = ServersPageImpl as unknown as ComponentType<Record<string, unknown>>;
const resizeTo = (width: number) =>
    act(() => {
        Object.defineProperty(window, 'innerWidth', { value: width, configurable: true });
        fireEvent(window, new Event('resize'));
    });

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
    vi.useRealTimers();
    Object.defineProperty(window, 'innerWidth', { value: 1024, configurable: true });
});

describe('설치 진행 확인', () => {
    it('좁은 창에서 상세만 보여 줄이 없어도 진행률을 받는다', async () => {
        localStorage.clear();
        useMaintenanceStore.setState({ byId: {} });
        useUIStore.getState()._resetForTest();
        useServerStore.getState()._resetForTest();
        useSettingsStore.setState({ settingsReady: true, closedGuides: ['firstRun'], systemMonitorMode: 'off' });
        useServerStore.setState({ modulesLoaded: true });
        window.api = {
            instanceProvisionProgress: vi
                .fn()
                // 첫 답은 상세로 바뀌기 전 잠깐 그려진 줄이 받을 수도 있다: 그 뒤의 답이 화면에 실려야 한다
                .mockResolvedValueOnce({ active: true, percent: 40, done: false })
                .mockResolvedValue({ active: true, percent: 80, done: false }),
            onDaemonEvent: vi.fn(() => () => {}),
        } as unknown as typeof window.api;
        Object.defineProperty(window, 'innerWidth', { value: 800, configurable: true });
        useServerStore.getState().setServers([{ id: 's1', name: 'pal', module: 'game', status: 'stopped', provisioning: true }] as never);
        useUIStore.getState().revealServer('s1');
        render(
            <ContextMenuProvider allowNative={false}>
                <ServersPage
                    servers={useServerStore.getState().servers}
                    modules={[]}
                    onAddServer={() => {}}
                    drag={{ cardRefs: { current: {} }, skipNextClick: { current: false }, handleCardPointerDown: () => {} }}
                    consoleProps={{ consoles: {}, popoutOpen: {}, consolePopoutIds: [] }}
                />
            </ContextMenuProvider>,
        );
        await act(async () => vi.advanceTimersByTimeAsync(15_000));
        expect(document.querySelector('.server-row')).toBeNull();
        expect(useMaintenanceStore.getState().byId.s1?.provisionProgress).toMatchObject({ percent: 80 });
    });
});

describe('다시 그리기', () => {
    it('익스텐션 값이 그대로면 Provider가 다시 그려져도 쓰는 곳은 다시 그리지 않는다', async () => {
        window.api = { ...window.api, extensionList: vi.fn().mockResolvedValue({ extensions: [] }) } as unknown as typeof window.api;
        let renders = 0;
        const Consumer = memo(function Consumer() {
            useExtensions();
            renders++;
            return null;
        });
        const consumer = <Consumer />;
        const tree = (n: number) => (
            <div data-n={n}>
                <ExtensionProvider>{consumer}</ExtensionProvider>
            </div>
        );
        const { rerender } = render(tree(0));
        await act(async () => vi.advanceTimersByTimeAsync(100));
        const settled = renders;
        rerender(tree(1));
        rerender(tree(2));
        expect(renders).toBe(settled);
    });

    it('창 너비는 경계를 넘을 때만 알린다', () => {
        Object.defineProperty(window, 'innerWidth', { value: 1000, configurable: true });
        let renders = 0;
        const { result } = renderHook(() => {
            renders++;
            return useWindowAtLeast(900);
        });
        expect(result.current).toBe(true);
        const settled = renders;
        resizeTo(1100);
        resizeTo(950);
        expect(renders).toBe(settled);
        resizeTo(800);
        expect(result.current).toBe(false);
    });

    it('활동 줄은 보일 명령이 없으면 시간이 흘러도 다시 그리지 않는다', async () => {
        window.api = {
            ...window.api,
            auditList: vi.fn().mockResolvedValue({ entries: [] }),
            onDaemonEvent: vi.fn(() => () => {}),
        } as unknown as typeof window.api;
        const onRender = vi.fn();
        render(
            <Profiler id="bar" onRender={onRender}>
                <ActivityBar />
            </Profiler>,
        );
        await act(async () => vi.advanceTimersByTimeAsync(100));
        const settled = onRender.mock.calls.length;
        await act(async () => vi.advanceTimersByTimeAsync(20_000));
        expect(onRender.mock.calls.length).toBe(settled);
    });
});

describe('묻지 않기', () => {
    it('숨긴 탭의 시스템 모니터는 데몬에 묻지 않고, 탭이 보이면 바로 묻는다', async () => {
        window.api = { ...window.api, systemMetrics: vi.fn().mockResolvedValue({}) } as unknown as typeof window.api;
        let active = false;
        const wrapper = ({ children }: { children: ReactNode }) => (
            <PageActiveContext.Provider value={active}>{children}</PageActiveContext.Provider>
        );
        const { rerender } = renderHook(() => useSystemMetrics(2000), { wrapper });
        await act(async () => vi.advanceTimersByTimeAsync(10_000));
        expect(window.api.systemMetrics).not.toHaveBeenCalled();
        active = true;
        rerender();
        await act(async () => vi.advanceTimersByTimeAsync(0));
        expect(window.api.systemMetrics).toHaveBeenCalledTimes(1);
    });

    it('데몬이 떠 있지 않으면 익스텐션 초기화 상태를 묻지 않는다', async () => {
        window.api = { ...window.api, extensionInitStatus: vi.fn().mockResolvedValue({ daemon_unreachable: true }) } as unknown as typeof window.api;
        const { rerender } = renderHook(({ up }) => useExtensionInitStatus(up), { initialProps: { up: false } });
        await act(async () => vi.advanceTimersByTimeAsync(10_000));
        expect(window.api.extensionInitStatus).not.toHaveBeenCalled();
        rerender({ up: true });
        await act(async () => vi.advanceTimersByTimeAsync(0));
        expect(window.api.extensionInitStatus).toHaveBeenCalledTimes(1);
    });
});
