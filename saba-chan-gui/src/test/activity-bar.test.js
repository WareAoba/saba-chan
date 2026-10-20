import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ACTIVITY_POLL_MS, ActivityBar, ago } from '../components/shell/ActivityBar';

const entry = (over) => ({
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
        window.api.onDaemonEvent = undefined; // 이벤트 스트림이 없을 때의 폴링
    });
    afterEach(() => {
        vi.useRealTimers();
        delete window.api.onDaemonEvent;
        delete window.api.auditList;
        delete window.api.daemonConsole;
    });

    it('최신 감사 1건을 한 줄로 보이고, 다음 폴링에서 바뀐다', async () => {
        window.api.daemonConsole = undefined; // preload에 아직 없을 때
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
        expect(screen.getByText('Failed (500)')).toBeInTheDocument();
    });

    it('누르면 데몬 콘솔을 펼치고, 받은 다음 id부터 이어 받는다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] });
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
        expect(window.api.daemonConsole).toHaveBeenNthCalledWith(2, { since: 8 });
    });

    // L55: 데몬은 다시 켜질 때마다 로그 id를 1부터 센다. 답의 epoch가 바뀌면 since를 버리고 처음부터 받는다.
    // 새 데몬이 이미 since보다 많은 줄을 냈어도(id 60 > since 51) 옛 줄 뒤에 섞지 않는다
    it('데몬이 다시 켜져 epoch가 바뀌면 받은 줄을 비우고 처음부터 다시 받는다', async () => {
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] });
        const line = (id, message) => ({ id, timestamp: Date.now(), level: 'info', target: 'x', message });
        let restarted = false;
        window.api.daemonConsole = vi.fn(async (opts) => {
            if (!restarted) return { epoch: 100, lines: opts.since === undefined ? [line(50, 'old 50')] : [] };
            return opts.since === undefined
                ? { epoch: 200, lines: [line(1, 'new 1'), line(60, 'new 60')] }
                : { epoch: 200, lines: [line(60, 'new 60')] };
        });
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
        expect(window.api.daemonConsole).toHaveBeenCalledWith({ since: 51 });
        expect(window.api.daemonConsole).toHaveBeenLastCalledWith({});
        expect(screen.queryByText('old 50')).toBeNull();
        expect(screen.getByText('new 1')).toBeInTheDocument();
        expect(screen.getAllByText('new 60')).toHaveLength(1);
    });

    it('ago는 초·분·시간 단위로 쓴다', () => {
        const now = 1_000_000_000;
        expect(ago(now / 1000 - 5, now, 'en')).toBe('5 seconds ago');
        expect(ago(now / 1000 - 125, now, 'en')).toBe('2 minutes ago');
        expect(ago(now / 1000 - 7200, now, 'en')).toBe('2 hours ago');
    });
});
