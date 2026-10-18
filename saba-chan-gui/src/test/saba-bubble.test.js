import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { SabaBubble } from '../components/shell/SabaBubble';
import { useSabaBubble } from '../stores/useSabaBubble';
import { useServerStore } from '../stores/useServerStore';
import { useUIStore } from '../stores/useUIStore';

function fakeStream() {
    const subs = new Set();
    window.api.onDaemonEvent = vi.fn((cb) => {
        subs.add(cb);
        return () => subs.delete(cb);
    });
    return (ev) =>
        act(async () => {
            for (const cb of subs) cb(ev);
        });
}

const advance = (ms) =>
    act(async () => {
        vi.advanceTimersByTime(ms);
    });

describe('사바쨩 말풍선', () => {
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: false });
        useSabaBubble.setState({ bubble: null });
        useServerStore.setState({ servers: [{ id: 'mc-1', name: 'Survival', status: 'stopped' }] });
        useUIStore.setState({ backgroundDaemonStatus: 'running' });
    });
    afterEach(() => {
        vi.useRealTimers();
        delete window.api.onDaemonEvent;
    });

    it('서버가 넘어지면 복구 중을 띄우고, 다시 켜질 때까지(최소 5초) 붙잡는다', async () => {
        const emit = fakeStream();
        render(<SabaBubble />);
        await emit({
            type: 'instance.exit',
            data: { instanceId: 'mc-1', instance: 'Survival', kind: 'crash', restartInSecs: 5 },
        });
        expect(screen.getByText('Survival fell over! Getting it back on its feet…')).toBeInTheDocument();

        // 5초가 지나도 서버가 아직 안 켜졌으면 그대로
        await advance(8000);
        expect(useSabaBubble.getState().bubble?.kind).toBe('recovering');

        // 켜지면 놓고, 들어가는 움직임 뒤에 사라진다
        await act(async () => {
            useServerStore.setState({ servers: [{ id: 'mc-1', name: 'Survival', status: 'running' }] });
        });
        await advance(400);
        expect(useSabaBubble.getState().bubble).toBeNull();
    });

    it('복구를 포기하면 멈춘 사바쨩을 띄운다', async () => {
        const emit = fakeStream();
        render(<SabaBubble />);
        await emit({
            type: 'instance.exit',
            data: { instanceId: 'mc-1', kind: 'crash', restartInSecs: null, noteCode: 'gave_up' },
        });
        expect(useSabaBubble.getState().bubble?.kind).toBe('stopped');
        expect(screen.getByText("Couldn't get Survival back up… Please check on it.")).toBeInTheDocument();
    });

    it('꺼진 데몬을 깨우면 (확인 중을 거쳐도) 졸린 사바쨩, 처음 켜질 때는 없다', async () => {
        useUIStore.setState({ backgroundDaemonStatus: 'checking' });
        render(<SabaBubble />);
        await act(async () => useUIStore.setState({ backgroundDaemonStatus: 'running' }));
        expect(useSabaBubble.getState().bubble).toBeNull();

        await act(async () => useUIStore.setState({ backgroundDaemonStatus: 'stopped' }));
        await act(async () => useUIStore.setState({ backgroundDaemonStatus: 'checking' }));
        await act(async () => useUIStore.setState({ backgroundDaemonStatus: 'running' }));
        expect(useSabaBubble.getState().bubble?.kind).toBe('sleepy');

        // 5초 뒤 스스로 들어간다
        await advance(5000);
        await advance(400);
        expect(useSabaBubble.getState().bubble).toBeNull();
    });
    it('붙잡힌 복구 중 말풍선을 졸린 사바쨩이 덮지 않고, 멈춤은 덮는다', async () => {
        const emit = fakeStream();
        useUIStore.setState({ backgroundDaemonStatus: 'stopped' });
        render(<SabaBubble />);
        await emit({ type: 'instance.exit', data: { instanceId: 'mc-1', kind: 'crash', restartInSecs: 5 } });
        await act(async () => useUIStore.setState({ backgroundDaemonStatus: 'running' }));
        expect(useSabaBubble.getState().bubble?.kind).toBe('recovering');

        await emit({ type: 'instance.restart_failed', data: { instanceId: 'mc-1' } });
        expect(useSabaBubble.getState().bubble?.kind).toBe('stopped');
    });

    it('넘어지기 전의 실행 상태가 남아 있어도 복구 중을 놓지 않는다', async () => {
        useServerStore.setState({ servers: [{ id: 'mc-1', name: 'Survival', status: 'running', start_time: 100 }] });
        const emit = fakeStream();
        render(<SabaBubble />);
        await emit({ type: 'instance.exit', data: { instanceId: 'mc-1', kind: 'crash', restartInSecs: 5 } });

        // 목록이 아직 넘어지기 전 그대로 다시 들어온다
        await act(async () => {
            useServerStore.setState({
                servers: [{ id: 'mc-1', name: 'Survival', status: 'running', start_time: 100 }],
            });
        });
        await advance(6000);
        await advance(400);
        expect(useSabaBubble.getState().bubble).toMatchObject({ kind: 'recovering', released: false });

        // 새로 켜졌으면(시작 시각이 바뀜) 놓는다
        await act(async () => {
            useServerStore.setState({
                servers: [{ id: 'mc-1', name: 'Survival', status: 'running', start_time: 200 }],
            });
        });
        await advance(400);
        expect(useSabaBubble.getState().bubble).toBeNull();
    });
    it.each([
        'instance.restarted',
        'instance.restart_cancelled',
    ])('%s가 오면 목록을 기다리지 않고 복구 중을 놓는다', async (type) => {
        const emit = fakeStream();
        render(<SabaBubble />);
        await emit({ type: 'instance.exit', data: { instanceId: 'mc-1', kind: 'crash', restartInSecs: 5 } });
        await emit({ type, data: { instanceId: 'mc-1', instance: 'Survival' } });
        expect(useSabaBubble.getState().bubble?.released).toBe(true);
    });
    it('이전 시도의 늦은 이벤트는 새 복구 중 표시를 놓지 않는다', async () => {
        const emit = fakeStream();
        render(<SabaBubble />);
        const crash = { instanceId: 'mc-1', kind: 'crash', restartInSecs: 5 };
        await emit({ type: 'instance.exit', data: { ...crash, restartAttempt: 2 } });
        await emit({ type: 'instance.restarted', data: { instanceId: 'mc-1', restartAttempt: 1 } });
        await emit({ type: 'instance.restart_failed', data: { instanceId: 'mc-1', restartAttempt: 1 } });
        expect(useSabaBubble.getState().bubble).toMatchObject({ kind: 'recovering', released: false });

        await emit({ type: 'instance.restarted', data: { instanceId: 'mc-1', restartAttempt: 2 } });
        expect(useSabaBubble.getState().bubble?.released).toBe(true);
    });

    it('알 수 없는 종료도 다시 띄우면 복구 중이다', async () => {
        const emit = fakeStream();
        render(<SabaBubble />);
        await emit({
            type: 'instance.exit',
            data: { instanceId: 'mc-1', kind: 'unknown_exit', restartInSecs: 5, restartAttempt: 1 },
        });
        expect(useSabaBubble.getState().bubble?.kind).toBe('recovering');
    });
});
