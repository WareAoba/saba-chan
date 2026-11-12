/**
 * 얇은 클라이언트 점검 4번 (2026-11-12): "시작하는 중"·"정지하는 중"을 데몬이 쥐고 목록의 `transition`으로 준다.
 * 다른 창·CLI·디스코드에서 시킨 것도 같은 모습으로 보인다.
 */
import { act, renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { useServerActions } from '../hooks/useServerActions';
import type { Server } from '../stores/useServerStore';

function setup(list: () => Server[]) {
    window.showToast = vi.fn();
    window.api = { serverList: vi.fn(async () => ({ servers: list() })) } as unknown as typeof window.api;
    let servers: Server[] = [];
    const setServers = (u: Server[] | ((s: Server[]) => Server[])) => {
        servers = typeof u === 'function' ? u(servers) : u;
    };
    const { result } = renderHook(() =>
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
    return { result, current: () => servers };
}

describe('데몬이 쥔 시작·정지 진행', () => {
    it('다른 곳에서 시킨 정지도 이 창에 정지하는 중으로 보인다', async () => {
        let list = [{ id: 'a', name: 'A', module: 'game', status: 'running', pid: 1 }] as unknown as Server[];
        const { result, current } = setup(() => list);
        await act(async () => result.current.fetchServers());
        list = [{ ...list[0], transition: 'stopping' }];
        await act(async () => result.current.fetchServers());
        expect(current()[0].status).toBe('stopping');
        list = [{ ...list[0], status: 'stopped', pid: null, transition: undefined }];
        await act(async () => result.current.fetchServers());
        expect(current()[0].status).toBe('stopped');
    });

    it('꺼진 서버의 시작하는 중도 데몬 값으로', async () => {
        const list = [{ id: 'a', name: 'A', module: 'game', status: 'stopped', transition: 'starting' }] as unknown as Server[];
        const { result, current } = setup(() => list);
        await act(async () => result.current.fetchServers());
        await act(async () => result.current.fetchServers());
        expect(current()[0].status).toBe('starting');
    });
});
