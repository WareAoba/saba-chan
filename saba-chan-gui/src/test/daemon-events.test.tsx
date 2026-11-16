import { act, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ActivityBar } from '../components/shell/ActivityBar';
import { DaemonEvents } from '../components/shell/DaemonEvents';
import i18n from '../i18n';
import { eventToast, normalizeEvent, subscribeDaemonEvents } from '../utils/daemonEvents';
import { SERVER_UPDATE_ACTION } from '../utils/noticeView';
import { applyServerUpdateFromNotice } from '../utils/serverUpdateNotice';

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
    const subs = new Set<(ev: unknown) => void>();
    window.api.onDaemonEvent = vi.fn((cb) => {
        subs.add(cb);
        return () => subs.delete(cb);
    }) as never;
    return {
        emit: (ev: unknown) =>
            act(async () => {
                for (const cb of subs) cb(ev);
            }),
        subs,
    };
}

describe('데몬 이벤트 스트림', () => {
    afterEach(() => {
        delete (window.api as Partial<typeof window.api>).onDaemonEvent;
        delete (window.api as Partial<typeof window.api>).auditList;
        window.showToast = undefined;
        vi.useRealTimers();
    });

    it('문자열 data도 읽는다', () => {
        expect(normalizeEvent({ id: 3, event: 'backup.done', data: '{"ok":true}' })).toEqual({
            id: 3,
            type: 'backup.done',
            data: { ok: true },
        });
        expect(normalizeEvent({ data: {} })).toBeNull();
    });

    it('토스트: 디스코드에서 한 명령·크래시·자동 백업·예약 예고 (알림함은 데몬이 정한다)', () => {
        // 디스코드에서 켜고 끈 것은 instance.started·stopped가 알린다
        expect(eventToast({ type: 'action.run', data: discordStop }, t)).toBeNull();
        // 켜고 끄기에 실패했으면 이벤트가 없으니 실패 이유와 함께 토스트로
        const failedStop = eventToast({ type: 'action.run', data: { ...discordStop, ok: false, status: 409, code: 'server_not_running' } }, t);
        expect(failedStop).toMatchObject({ message: 'Run from Discord: Kimchi · Survival · Stop · Failed · Server stopped', type: 'warning' });
        // 게임 명령은 토스트로만 알리고, 빈 인자 {}는 보이지 않는다
        const say = { ...discordStop, action: 'command', command: 'say', args: '{"message":"hi"}' };
        expect(eventToast({ type: 'action.run', data: say }, t)).toMatchObject({ message: 'Run from Discord: Kimchi · Survival · Command · say “hi”' });
        expect(eventToast({ type: 'action.run', data: { ...say, command: 'save-on', args: '{}' } }, t)?.message).toBe(
            'Run from Discord: Kimchi · Survival · Command · save-on',
        );
        expect(
            eventToast({ type: 'action.run', data: { ...say, ok: false, status: 400, code: 'invalid_argument' } }, t)?.message,
        ).toBe('Run from Discord: Kimchi · Survival · Command · say “hi” · Failed · Invalid input value');
        expect(eventToast({ type: 'action.run', data: { ...say, source: 'gui' } }, t)).toBeNull();
        // 게임 서버를 대상으로 하지 않은 일(디스코드 설정 바꾸기 등)은 작업 기록에만 남는다
        const guildSetting = { source: 'discord', ok: true, userName: 'Kimchi', action: 'chatbot.settings.set' };
        expect(eventToast({ type: 'action.run', data: guildSetting }, t)).toBeNull();
        expect(eventToast({ type: 'action.run', data: { ...guildSetting, ok: false, status: 403 } }, t)).toBeNull();

        // 크래시: 토스트에는 서버 이름과 번역한 이유
        const crash = eventToast(
            {
                type: 'instance.exit',
                data: { instanceId: 'srv-1', instance: 'Survival', kind: 'crash', restartInSecs: 5, evidenceCode: 'memory_limit', evidenceArgs: { mb: 4096 } },
            },
            t,
        );
        expect(crash?.message).toBe("'Survival' stopped (Crash) Memory limit reached (4096 MiB). Restarting in 5s");
        expect(eventToast({ type: 'instance.exit', data: { instance: 'Survival', kind: 'user_stop' } }, t)).toBeNull();

        // 자동 백업·예약 예고는 토스트로만
        const nameOf = () => 'Survival';
        expect(eventToast({ type: 'backup.done', data: { kind: 'manual', ok: true } }, t, nameOf)).toBeNull();
        const scheduled = eventToast({ type: 'backup.done', data: { kind: 'scheduled', ok: true } }, t, nameOf);
        expect(scheduled?.message).toBe('Survival backup complete (Scheduled)');
        expect(
            eventToast({ type: 'backup.done', data: { kind: 'scheduled', ok: false, error: 'disk full' } }, t, nameOf)
                ?.type,
        ).toBe('error');
        const warn = eventToast({ type: 'schedule.warn', data: { instance: 'Survival', action: 'restart', minutesLeft: 5 } }, t);
        expect(warn?.message).toBe("'Survival' restarts as scheduled in 5 min.");
        expect(eventToast({ type: 'mystery', data: {} }, t)).toBeNull();
    });

    it('복원 토스트: 앱에서 한 것은 다시 띄우지 않는다', () => {
        const restore = { at: 1700000000, source: 'web', action: 'restore', instanceId: 'srv-1', instance: 'Survival', ok: true, status: 200 };
        expect(eventToast({ type: 'action.run', data: restore }, t)).toMatchObject({
            message: "'Survival' restored (Saba-chan web)",
            type: 'success',
        });
        expect(eventToast({ type: 'action.run', data: { ...restore, source: 'gui' } }, t)).toMatchObject({ silent: true });
        const broken = eventToast({ type: 'action.run', data: { ...restore, source: 'discord', ok: false, status: 409, code: 'server_running' } }, t);
        expect(broken).toMatchObject({ type: 'error' });
    });

    it('켜고 끈 출처를 토스트에 싣는다 (사바쨩 밖에서 켠 것만 경고로)', () => {
        const started = (data: Record<string, unknown>) => eventToast({ type: 'instance.started', data: { instanceId: 'srv-1', instance: 'Survival', ...data } }, t);
        const stopped = (data: Record<string, unknown>) => eventToast({ type: 'instance.stopped', data: { instanceId: 'srv-1', instance: 'Survival', ...data } }, t);
        // 앱에서 한 일은 그 자리에서 토스트를 띄웠다
        expect(started({ source: 'gui', unexpected: false })).toMatchObject({ silent: true });
        expect(stopped({ source: 'gui', kind: 'user_stop' })).toMatchObject({ silent: true });
        const table = [
            ['gui', 'App'],
            ['discord', 'Discord'],
            ['chatbot', 'Talking Saba-chan'],
            ['cli', 'CLI'],
            ['web', 'Saba-chan web'],
            ['remote', 'Remote PC'],
            ['schedule', 'Schedule'],
            ['restart', 'Auto restart'],
            ['daemon', 'Daemon'],
        ];
        for (const [source, label] of table) {
            const on = started({ source, unexpected: false });
            expect(on).toMatchObject({ message: `'Survival' started (${label})`, type: 'info' });
            const off = stopped({ source, kind: 'user_stop' });
            expect(off).toMatchObject({ message: `'Survival' stopped (${label})`, type: 'info' });
        }
        for (const [source] of table.slice(1)) expect(started({ source, unexpected: false })?.silent).toBeFalsy();
        const inGame = stopped({ source: 'game', kind: 'in_game_stop' });
        expect(inGame).toMatchObject({ message: "'Survival' stopped from inside the game", type: 'info' });
        expect(started({ source: 'external', unexpected: true })).toMatchObject({
            message: "'Survival' started outside Saba-chan. Saba-chan did not start this process.",
            type: 'warning',
        });
    });

    it('토스트를 띄우고, 서버가 바뀌는 이벤트는 한 번만 목록을 다시 읽으며, 알림함은 데몬이 바뀌었다고 할 때 다시 읽는다', async () => {
        vi.useFakeTimers();
        window.showToast = vi.fn();
        window.api.noticesList = vi.fn().mockResolvedValue({ success: true, notices: [], unread: 0 }) as never;
        const stream = fakeStream();
        const refetch = vi.fn();
        const { unmount } = render(<DaemonEvents onServersChanged={refetch} />);
        await stream.emit({ id: 1, type: 'action.run', data: discordStop });
        await stream.emit({ id: 2, type: 'instance.exit', data: { instance: 'Survival', kind: 'user_stop' } });
        await stream.emit({ id: 3, type: 'instance.stopped', data: { instanceId: 'srv-1', instance: 'Survival', source: 'discord', userName: 'Kimchi', kind: 'user_stop' } });
        await act(async () => vi.advanceTimersByTime(400));
        expect(refetch).toHaveBeenCalledTimes(1);
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(window.showToast).toHaveBeenCalledWith("'Survival' stopped (Discord · Kimchi)", 'info', 4000, undefined);
        // 처음 붙을 때 한 번 읽고, 데몬이 알림함이 바뀌었다고 하면 다시 읽는다
        expect(window.api.noticesList).toHaveBeenCalledTimes(1);
        await stream.emit({ type: 'notices.changed', data: { unread: 1 } });
        expect(window.api.noticesList).toHaveBeenCalledTimes(2);
        delete (window.api as Partial<typeof window.api>).noticesList;
        unmount();
        expect(stream.subs.size).toBe(0);
    });

    it('업데이트 알림을 누르면 데몬에 업데이트를 시키고 결과를 토스트로 알린다', async () => {
        window.api.noticeRemove = vi.fn().mockResolvedValue({ success: true }) as never;
        window.showToast = vi.fn() as never;
        window.api.instanceApplyUpdate = vi.fn().mockResolvedValue({ success: true, scheduled: true, warn_minutes: 5, players: 2 }) as never;
        const notice = { id: 7, action: SERVER_UPDATE_ACTION, data: { instanceId: 'i', name: 'Pal' } } as never;
        await applyServerUpdateFromNotice(notice, t);
        expect(window.api.instanceApplyUpdate).toHaveBeenCalledWith('i');
        expect(window.api.noticeRemove).toHaveBeenCalledWith(7);
        expect(window.showToast).toHaveBeenLastCalledWith(
            'Pal: warned in game. Update in 5 min.',
            'info',
            5000,
            undefined,
        );
        window.api.instanceApplyUpdate = vi.fn().mockResolvedValue({ success: false, error: 'already updating' }) as never;
        await applyServerUpdateFromNotice(notice, t);
        expect(window.showToast).toHaveBeenLastCalledWith('Pal: update start failed. already updating', 'error', 5000, undefined);
        delete (window.api as Partial<typeof window.api>).noticeRemove;
    });

    it('활동 줄은 action.run을 받으면 폴링을 기다리지 않고 바뀐다', async () => {
        const stream = fakeStream();
        window.api.auditList = vi.fn().mockResolvedValue({ entries: [] }) as never;
        render(<ActivityBar />);
        await flush();
        // 콘솔을 펼칠 수 있으면 평소엔 안내 문구를 보인다
        expect(screen.getByText('Click to view the daemon console log')).toBeInTheDocument();
        await stream.emit({ id: 1, type: 'action.run', data: { ...discordStop, at: Math.floor(Date.now() / 1000) } });
        expect(screen.getByText('Discord · Kimchi · Survival · Stop')).toBeInTheDocument();
        expect(window.api.auditList).toHaveBeenCalledTimes(1);
    });
});
