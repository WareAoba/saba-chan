/**
 * 작은 창(트레이 팝업·펫)의 서버 목록: 이벤트로 부른 읽기와 주기 읽기가 겹쳐도
 * 늦게 온 옛 목록이 새 목록을 덮지 않는다.
 */
import { act, renderHook } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { useQuickServers } from '../components/shell/ServerQuickList';

afterEach(() => {
    for (const k of ['daemonStatus', 'daemonTarget', 'serverList', 'onDaemonEvent'])
        delete (window.api as unknown as Record<string, unknown>)[k];
});

it('먼저 보낸 읽기의 답이 늦게 와도 나중 읽기의 목록을 남긴다', async () => {
    const answers: ((v: unknown) => void)[] = [];
    let emit: (ev: unknown) => void = () => {};
    window.api.daemonStatus = vi.fn().mockResolvedValue({ running: true });
    window.api.daemonTarget = vi.fn().mockResolvedValue({ target: null });
    window.api.serverList = vi.fn(() => new Promise((r) => answers.push(r))) as never;
    window.api.onDaemonEvent = vi.fn((cb) => {
        emit = cb;
        return () => {};
    }) as never;

    const { result } = renderHook(() => useQuickServers(60_000));
    await act(async () => {});
    await act(async () => emit({ type: 'instance.started', data: { instance_id: 'a' } }));
    expect(answers).toHaveLength(2);
    await act(async () => answers[1]({ servers: [{ id: 'a', name: '새', status: 'running' }] }));
    await act(async () => answers[0]({ servers: [{ id: 'a', name: '옛', status: 'stopped' }] }));
    expect(result.current.servers.map((s) => s.name)).toEqual(['새']);
});
