import { act, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DaemonEvents } from '../components/shell/DaemonEvents';
import { setupServerStore } from './serverStore';
import i18n from '../i18n';
import type { Server } from '../stores/useServerStore';

// 데몬이 Windows 예약 범위를 피해 REST 포트를 옮겼으면(port_moved) 저장값이 조용히 바뀌지 않게 알린다 (M13)
const startWith = async (response: Record<string, unknown>, action = 'start') => {
    const list = [{ id: 'a', name: 'A', module: 'game', status: 'stopped' }] as unknown as Server[];
    window.api = {
        ...window.api,
        serverList: vi.fn(async () => ({ servers: list })),
        instanceStart: vi.fn(async () => response),
        instanceRestart: vi.fn(async () => response),
    } as unknown as typeof window.api;
    const { result, setModal } = setupServerStore({ servers: list });
    if (action === 'restart') {
        await act(async () => result.current.restartServer('a'));
        await act(async () => setModal.mock.calls.at(-1)![0].onConfirm());
    } else {
        await act(async () => result.current.startServer('a'));
    }
    return vi.mocked(window.showToast!).mock.calls.map((c) => c[0]);
};

// 같은 (서버, from, to)는 한 번만 알리므로 시험마다 다른 포트를 쓴다
const movedTo = (to: number) => [{ field: 'rest_port', from: 8212, to }];
const notice = (to: number) => i18n.t('servers.port_moved', { name: 'A', from: 8212, to });

describe('옮긴 포트 안내', () => {
    beforeEach(() => {
        window.showToast = vi.fn();
    });
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).onDaemonEvent;
    });

    it('시작에 성공해도 옮긴 포트를 알린다', async () => {
        const toasts = await startWith({ success: true, port_moved: movedTo(8242) });
        expect(toasts).toContain(notice(8242));
    });

    it('시작에 실패해도 옮긴 포트를 알린다', async () => {
        const toasts = await startWith({ success: false, error: 'boom', port_moved: movedTo(8243) });
        expect(toasts).toContain(notice(8243));
    });

    it('재시작에서도 옮긴 포트를 알린다', async () => {
        const toasts = await startWith({ success: true, port_moved: movedTo(8244) }, 'restart');
        expect(toasts).toContain(notice(8244));
    });

    it('옮기지 않았으면 알리지 않는다', async () => {
        const toasts = await startWith({ success: true });
        expect(toasts.some((m) => /8212/.test(m))).toBe(false);
    });

    it('응답을 아무도 보지 않는 자동 재시작은 이벤트로 알리고, 같은 이동을 응답과 이벤트로 두 번 알리지 않는다', async () => {
        const subs = new Set<(ev: unknown) => void>();
        window.api.onDaemonEvent = vi.fn((cb) => {
            subs.add(cb);
            return () => subs.delete(cb);
        });
        render(<DaemonEvents onServersChanged={() => {}} />);
        const ev = {
            type: 'instance.port_moved',
            data: { instanceId: 'a', instance: 'A', field: 'rest_port', from: 8212, to: 8245 },
        };
        await act(async () => {
            for (const cb of subs) cb(ev);
        });
        expect(vi.mocked(window.showToast!).mock.calls.map((c) => c[0])).toEqual([notice(8245)]);

        const toasts = await startWith({ success: true, port_moved: movedTo(8245) });
        expect(toasts.filter((m) => m === notice(8245))).toHaveLength(1);
    });
});
