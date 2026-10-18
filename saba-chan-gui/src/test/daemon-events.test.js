import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ActivityBar } from '../components/shell/ActivityBar';
import { DaemonEvents } from '../components/shell/DaemonEvents';
import i18n from '../i18n';
import { eventToast, normalizeEvent, subscribeDaemonEvents } from '../utils/daemonEvents';

const t = i18n.getFixedT('en', 'gui');
const flush = () => act(async () => {});
const discordStop = {
    at: 1700000000,
    source: 'discord',
    userName: 'Kimchi',
    action: 'stop',
    instanceId: 'srv-1',
    instance: 'Survival',
    ok: true,
    status: 200,
};

/** onDaemonEvent 흉내: 넘긴 콜백을 잡아 두고 emit으로 부른다 */
function fakeStream() {
    const subs = new Set();
    window.api.onDaemonEvent = vi.fn((cb) => {
        subs.add(cb);
        return () => subs.delete(cb);
    });
    return {
        emit: (ev) =>
            act(async () => {
                for (const cb of subs) cb(ev);
            }),
        subs,
    };
}

describe('데몬 이벤트 스트림', () => {
    afterEach(() => {
        delete window.api.onDaemonEvent;
        delete window.api.auditList;
        window.showToast = undefined;
        vi.useRealTimers();
    });

    it('채널이 없으면 구독하지 않고, 문자열 data도 읽는다', () => {
        window.api.onDaemonEvent = undefined;
        const off = subscribeDaemonEvents(vi.fn());
        expect(typeof off).toBe('function');
        expect(normalizeEvent({ id: 3, event: 'backup.done', data: '{"ok":true}' })).toEqual({
            id: 3,
            type: 'backup.done',
            data: { ok: true },
        });
        expect(normalizeEvent({ data: {} })).toBeNull();
    });

    it('디스코드에서 한 일·크래시·자동 백업만 토스트로 알린다', () => {
        expect(eventToast({ type: 'action.run', data: discordStop }, t).message).toBe(
            'Run from Discord: Kimchi · Survival · Stop',
        );
        expect(eventToast({ type: 'action.run', data: { ...discordStop, source: 'gui' } }, t)).toBeNull();
        expect(
            eventToast({ type: 'instance.exit', data: { instance: 'Survival', kind: 'crash', restartInSecs: 5 } }, t)
                .message,
        ).toBe('Survival stopped (Crash). Restarting in 5s');
        expect(eventToast({ type: 'instance.exit', data: { instance: 'Survival', kind: 'user_stop' } }, t)).toBeNull();
        const nameOf = () => 'Survival';
        expect(eventToast({ type: 'backup.done', data: { kind: 'manual', ok: true } }, t, nameOf)).toBeNull();
        expect(eventToast({ type: 'backup.done', data: { kind: 'scheduled', ok: true } }, t, nameOf).message).toBe(
            'Backed up Survival (Scheduled).',
        );
        expect(
            eventToast({ type: 'backup.done', data: { kind: 'scheduled', ok: false, error: 'disk full' } }, t, nameOf)
                .type,
        ).toBe('error');
        expect(
            eventToast({ type: 'schedule.warn', data: { instance: 'Survival', action: 'restart', minutesLeft: 5 } }, t)
                .message,
        ).toBe('Survival will restart as scheduled in 5 min.');
        expect(eventToast({ type: 'mystery', data: {} }, t)).toBeNull();
    });

    it('토스트를 띄우고, 서버가 바뀌는 이벤트는 한 번만 목록을 다시 읽는다', async () => {
        vi.useFakeTimers();
        window.showToast = vi.fn();
        const stream = fakeStream();
        const refetch = vi.fn();
        const { unmount } = render(<DaemonEvents onServersChanged={refetch} />);
        await stream.emit({ id: 1, type: 'action.run', data: discordStop });
        await stream.emit({ id: 2, type: 'instance.exit', data: { instance: 'Survival', kind: 'user_stop' } });
        await act(async () => vi.advanceTimersByTime(400));
        expect(refetch).toHaveBeenCalledTimes(1);
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(window.showToast).toHaveBeenCalledWith('Run from Discord: Kimchi · Survival · Stop', 'info', 5000, {
            isNotice: true,
            source: 'saba-chan',
        });
        window.__sabaNotice = { addNotice: vi.fn() };
        await stream.emit({ id: 3, type: 'update.available', data: { components: [{ name: 'saba-core' }, 'gui'] } });
        expect(window.__sabaNotice.addNotice).toHaveBeenCalledWith(
            expect.objectContaining({ message: '📦 2 update(s) found: saba-core, gui' }),
        );
        delete window.__sabaNotice;
        unmount();
        expect(stream.subs.size).toBe(0);
    });

    it('활동 줄은 action.run을 받으면 폴링을 기다리지 않고 바뀐다', async () => {
        const stream = fakeStream();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] });
        render(<ActivityBar />);
        await flush();
        expect(screen.getByText('No server commands have run yet.')).toBeInTheDocument();
        await stream.emit({ id: 1, type: 'action.run', data: { ...discordStop, at: Math.floor(Date.now() / 1000) } });
        expect(screen.getByText('Discord · Kimchi · Survival · Stop')).toBeInTheDocument();
        expect(window.api.auditList).toHaveBeenCalledTimes(1);
    });
});
