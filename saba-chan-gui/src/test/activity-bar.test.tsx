import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ACTIVITY_POLL_MS, ActivityBar, ago } from '../components/shell/ActivityBar';

const entry = (over?: Record<string, unknown>) => ({
    ts: Math.floor(Date.now() / 1000) - 3,
    source: 'discord',
    user: '2840',
    userName: 'kimchi',
    action: 'stop',
    instanceId: 'mc-1',
    instance: 'Survival',
    ok: true,
    status: 200,
    ...over,
});

const flush = () => act(async () => {});

describe('ActivityBar', () => {
    beforeEach(() => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        (window.api as Partial<typeof window.api>).onDaemonEvent = undefined; // 이벤트 스트림이 없을 때의 폴링
    });
    afterEach(() => {
        vi.useRealTimers();
        delete (window.api as Partial<typeof window.api>).onDaemonEvent;
        delete (window.api as Partial<typeof window.api>).auditList;
        delete (window.api as Partial<typeof window.api>).daemonConsole;
    });

    it('최신 감사 1건을 한 줄로 보이고, 다음 폴링에서 바뀐다', async () => {
        (window.api as Partial<typeof window.api>).daemonConsole = undefined; // preload에 아직 없을 때
        window.api.auditList = vi
            .fn()
            .mockResolvedValueOnce({ entries: [entry()] })
            .mockResolvedValue({
                entries: [
                    entry({
                        source: 'gui',
                        userName: undefined,
                        user: undefined,
                        action: 'start',
                        ok: false,
                        status: 500,
                    }),
                ],
            });
        render(<ActivityBar />);
        await flush();
        expect(window.api.auditList).toHaveBeenCalledWith({ limit: 1 });
        expect(screen.getByText('Discord · kimchi · Survival · Stop')).toBeInTheDocument();
        expect(screen.getByText('Succeeded')).toBeInTheDocument();
        // 데몬 콘솔 IPC가 없으면 펼칠 수 없다
        expect(screen.queryByRole('button')).toBeNull();

        await act(async () => {
            vi.advanceTimersByTime(ACTIVITY_POLL_MS);
        });
        await flush();
        expect(screen.getByText('App · Survival · Start')).toBeInTheDocument();
        expect(screen.getByText('Failed')).toBeInTheDocument();
    });

    it('누르면 데몬 콘솔을 펼치고, 받은 다음 id부터 이어 받는다 (새 줄이 없으면 데몬이 붙잡는다)', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] }) as never;
        window.api.daemonConsole = vi
            .fn()
            .mockResolvedValueOnce({
                lines: [{ id: 7, timestamp: Date.now(), level: 'warn', target: 'x', message: 'disk slow' }],
            })
            .mockResolvedValue({ lines: [] });
        render(<ActivityBar />);
        await flush();
        const line = screen.getByRole('button');
        expect(line).toHaveAttribute('aria-expanded', 'false');

        fireEvent.click(line);
        await flush();
        expect(window.api.daemonConsole).toHaveBeenNthCalledWith(1, {});
        expect(screen.getByText('disk slow')).toBeInTheDocument();

        await act(async () => {
            vi.advanceTimersByTime(1000);
        });
        await flush();
        expect(window.api.daemonConsole).toHaveBeenNthCalledWith(2, { since: 8, wait: 8 });
    });

    // 사용자 2026-11-12: 줄 전체를 레벨 색으로 칠하지 않는다. 레벨은 배지, 본문은 데몬이 나눈 낱말 조각(spans)으로 칠한다
    it('데몬 로그 본문은 spans 조각대로 칠하고 레벨은 배지로만 보인다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] }) as never;
        window.api.daemonConsole = vi.fn().mockResolvedValue({
            lines: [
                {
                    id: 1,
                    timestamp: Date.now(),
                    level: 'WARN',
                    target: 'x',
                    message: '[Heartbeat] Client timed out',
                    spans: [
                        [0, 11, 'keyword'],
                        [19, 28, 'error'],
                    ],
                },
            ],
        }) as never;
        render(<ActivityBar />);
        await flush();
        fireEvent.click(screen.getByRole('button'));
        await flush();
        // 펼친 콘솔은 팝오버(포털)에 뜬다
        const line = document.querySelector('.daemon-log-line') as HTMLElement;
        expect(line.querySelector('.tok-keyword')?.textContent).toBe('[Heartbeat]');
        expect(line.querySelector('.tok-error')?.textContent).toBe('timed out');
        expect(line.querySelector('.daemon-log-level')?.textContent).toBe('WARN');
    });

    // L55: 데몬은 다시 켜질 때마다 로그 id를 1부터 센다. 답의 epoch가 바뀌면 since를 버리고 처음부터 받는다.
    // 새 데몬이 이미 since보다 많은 줄을 냈어도(id 60 > since 51) 옛 줄 뒤에 섞지 않는다
    it('데몬이 다시 켜져 epoch가 바뀌면 받은 줄을 비우고 처음부터 다시 받는다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] }) as never;
        const line = (id: number, message: string) => ({ id, timestamp: Date.now(), level: 'info', target: 'x', message });
        let restarted = false;
        window.api.daemonConsole = vi.fn(async (opts) => {
            if (!restarted) return { epoch: 100, lines: opts.since === undefined ? [line(50, 'old 50')] : [] };
            return opts.since === undefined
                ? { epoch: 200, lines: [line(1, 'new 1'), line(60, 'new 60')] }
                : { epoch: 200, lines: [line(60, 'new 60')] };
        }) as never;
        render(<ActivityBar />);
        await flush();
        fireEvent.click(screen.getByRole('button'));
        await flush();
        expect(screen.getByText('old 50')).toBeInTheDocument();

        restarted = true;
        await act(async () => {
            vi.advanceTimersByTime(1000);
        });
        await flush();
        expect(window.api.daemonConsole).toHaveBeenCalledWith({ since: 51, wait: 8 });
        expect(window.api.daemonConsole).toHaveBeenLastCalledWith({});
        expect(screen.queryByText('old 50')).toBeNull();
        expect(screen.getByText('new 1')).toBeInTheDocument();
        expect(screen.getAllByText('new 60')).toHaveLength(1);
    });

    it('처음 받은 뒤로는 데몬 로그 스트림으로 새 줄을 받고, 데몬이 다시 켜져 처음부터 오면 받은 줄을 바꾼다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] }) as never;
        const line = (id: number, message: string) => ({ id, timestamp: Date.now(), level: 'info', target: 'x', message });
        window.api.daemonConsole = vi.fn(async () => ({ epoch: 100, lines: [line(7, 'first')] })) as never;
        let update: (body: unknown) => void = () => {};
        const close = vi.fn();
        const stream = vi.fn(async (_opts: object, onUpdate: (body: unknown) => void) => {
            update = onUpdate;
            return close;
        });
        window.api.daemonConsoleStream = stream as never;
        const { unmount } = render(<ActivityBar />);
        await flush();
        fireEvent.click(screen.getByRole('button'));
        await flush();
        expect(stream).toHaveBeenCalledWith({ since: 8, epoch: 100 }, expect.any(Function), expect.any(Function));

        act(() => update({ epoch: 100, lines: [line(8, 'second')] }));
        expect(screen.getByText('second')).toBeInTheDocument();
        act(() => update({ epoch: 200, reset: true, lines: [line(1, 'again')] }));
        expect(screen.queryByText('first')).toBeNull();
        expect(screen.getByText('again')).toBeInTheDocument();
        // 스트림이 열려 있는 동안은 다시 묻지 않는다
        await act(async () => {
            vi.advanceTimersByTime(5000);
        });
        expect(window.api.daemonConsole).toHaveBeenCalledTimes(1);
        unmount();
        expect(close).toHaveBeenCalledTimes(1);
        delete (window.api as Partial<typeof window.api>).daemonConsoleStream;
    });

    it('ago는 초·분·시간 단위로 쓴다', () => {
        const now = 1_000_000_000;
        expect(ago(now / 1000 - 5, now, 'en')).toBe('5 seconds ago');
        expect(ago(now / 1000 - 125, now, 'en')).toBe('2 minutes ago');
        expect(ago(now / 1000 - 7200, now, 'en')).toBe('2 hours ago');
    });
});
