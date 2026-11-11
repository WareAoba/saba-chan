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

    it('채널이 없으면 구독하지 않고, 문자열 data도 읽는다', () => {
        (window.api as Partial<typeof window.api>).onDaemonEvent = undefined;
        const off = subscribeDaemonEvents(vi.fn());
        expect(typeof off).toBe('function');
        expect(normalizeEvent({ id: 3, event: 'backup.done', data: '{"ok":true}' })).toEqual({
            id: 3,
            type: 'backup.done',
            data: { ok: true },
        });
        expect(normalizeEvent({ data: {} })).toBeNull();
    });

    it('토스트: 디스코드에서 한 명령·크래시·자동 백업·예약 예고. 알림 모음에는 시작·정지·복원·크래시만', () => {
        // 디스코드에서 켜고 끈 것은 instance.started·stopped가 알린다
        expect(eventToast({ type: 'action.run', data: discordStop }, t)).toBeNull();
        // 켜고 끄기에 실패했으면 이벤트가 없으니 실패 이유와 함께 토스트로
        const failedStop = eventToast({ type: 'action.run', data: { ...discordStop, ok: false, status: 409, code: 'server_not_running' } }, t);
        expect(failedStop).toMatchObject({ message: 'Run from Discord: Kimchi · Survival · Stop · Failed · The server was off', type: 'warning' });
        expect(failedStop?.notice).toBeUndefined();
        // 게임 명령은 토스트로만 알리고, 빈 인자 {}는 보이지 않는다
        const say = { ...discordStop, action: 'command', command: 'say', args: '{"message":"hi"}' };
        expect(eventToast({ type: 'action.run', data: say }, t)).toMatchObject({ message: 'Run from Discord: Kimchi · Survival · Command · say “hi”' });
        expect(eventToast({ type: 'action.run', data: say }, t)?.notice).toBeUndefined();
        expect(eventToast({ type: 'action.run', data: { ...say, command: 'save-on', args: '{}' } }, t)?.message).toBe(
            'Run from Discord: Kimchi · Survival · Command · save-on',
        );
        expect(
            eventToast({ type: 'action.run', data: { ...say, ok: false, status: 400, code: 'invalid_argument' } }, t)?.message,
        ).toBe('Run from Discord: Kimchi · Survival · Command · say “hi” · Failed · An input value wasn\'t valid');
        expect(eventToast({ type: 'action.run', data: { ...say, source: 'gui' } }, t)).toBeNull();
        // 게임 서버를 대상으로 하지 않은 일(디스코드 설정 바꾸기 등)은 작업 기록에만 남는다
        const guildSetting = { source: 'discord', ok: true, userName: 'Kimchi', action: 'chatbot.settings.set' };
        expect(eventToast({ type: 'action.run', data: guildSetting }, t)).toBeNull();
        expect(eventToast({ type: 'action.run', data: { ...guildSetting, ok: false, status: 403 } }, t)).toBeNull();

        // 크래시: 토스트에는 서버 이름, 알림에는 서버별로 묶을 짧은 줄과 번역한 이유
        const crash = eventToast(
            {
                type: 'instance.exit',
                data: { instanceId: 'srv-1', instance: 'Survival', kind: 'crash', restartInSecs: 5, evidenceCode: 'memory_limit', evidenceArgs: { mb: 4096 } },
            },
            t,
        );
        expect(crash?.message).toBe('Survival stopped (Crash). Hit the memory limit (4096 MiB). Restarting in 5s');
        expect(crash?.notice).toEqual({
            message: 'Crashed',
            detail: 'Hit the memory limit (4096 MiB) · Restarting in 5s',
            type: 'error',
            source: 'Survival',
            instanceId: 'srv-1',
        });
        const failed = eventToast(
            { type: 'instance.exit', data: { instanceId: 'srv-1', instance: 'Survival', kind: 'start_failed', evidenceCode: 'ended_early', evidenceArgs: { secs: 4 } } },
            t,
        );
        expect(failed?.notice).toMatchObject({ message: 'Failed to start', detail: 'Ended 4s after starting' });
        // 콘솔 줄이 근거인 것은 이유를 따로 적지 않는다
        const quiet = eventToast({ type: 'instance.exit', data: { instance: 'Survival', kind: 'unknown_exit', evidenceCode: 'console' } }, t)?.notice;
        expect(quiet?.message).toBe('Stopped unexpectedly');
        expect(quiet?.detail).toBeUndefined();
        expect(eventToast({ type: 'instance.exit', data: { instance: 'Survival', kind: 'user_stop' } }, t)).toBeNull();

        // 자동 백업·예약 예고는 토스트로만
        const nameOf = () => 'Survival';
        expect(eventToast({ type: 'backup.done', data: { kind: 'manual', ok: true } }, t, nameOf)).toBeNull();
        const scheduled = eventToast({ type: 'backup.done', data: { kind: 'scheduled', ok: true } }, t, nameOf);
        expect(scheduled?.message).toBe('Backed up Survival (Scheduled).');
        expect(scheduled?.notice).toBeUndefined();
        expect(
            eventToast({ type: 'backup.done', data: { kind: 'scheduled', ok: false, error: 'disk full' } }, t, nameOf)
                ?.type,
        ).toBe('error');
        const warn = eventToast({ type: 'schedule.warn', data: { instance: 'Survival', action: 'restart', minutesLeft: 5 } }, t);
        expect(warn?.message).toBe('Survival will restart as scheduled in 5 min.');
        expect(warn?.notice).toBeUndefined();
        expect(eventToast({ type: 'mystery', data: {} }, t)).toBeNull();
    });

    it('복원은 어디서 했든 알림에 남기고, 앱에서 한 것은 토스트를 다시 띄우지 않는다', () => {
        const restore = { at: 1700000000, source: 'web', action: 'restore', instanceId: 'srv-1', instance: 'Survival', ok: true, status: 200 };
        expect(eventToast({ type: 'action.run', data: restore }, t)).toMatchObject({
            message: 'Survival was restored (Saba-chan web).',
            type: 'success',
            notice: { message: 'Restored · Saba-chan web', type: 'success', source: 'Survival', instanceId: 'srv-1' },
        });
        const mine = eventToast({ type: 'action.run', data: { ...restore, source: 'gui' } }, t);
        expect(mine).toMatchObject({ silent: true, notice: { message: 'Restored · App', read: true } });
        const broken = eventToast({ type: 'action.run', data: { ...restore, source: 'discord', ok: false, status: 409, code: 'server_running' } }, t);
        expect(broken?.notice).toMatchObject({ message: 'Restore failed', detail: 'The server was running', type: 'error' });
    });

    it('켜고 끈 출처를 문구에 싣고, 사바쨩 밖에서 켠 것만 경고로 알린다', () => {
        const started = (data: Record<string, unknown>) => eventToast({ type: 'instance.started', data: { instanceId: 'srv-1', instance: 'Survival', ...data } }, t);
        const stopped = (data: Record<string, unknown>) => eventToast({ type: 'instance.stopped', data: { instanceId: 'srv-1', instance: 'Survival', ...data } }, t);
        // 앱에서 한 일은 그 자리에서 토스트를 띄웠으니 알림에만 읽은 것으로 남긴다
        expect(started({ source: 'gui', unexpected: false })).toMatchObject({ silent: true, notice: { message: 'Started · App', read: true } });
        expect(stopped({ source: 'gui', kind: 'user_stop' })).toMatchObject({ silent: true, notice: { message: 'Stopped · App', read: true } });
        const table = [
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
            expect(on).toMatchObject({ message: `Survival started (${label}).`, type: 'info' });
            expect(on?.silent).toBeFalsy();
            expect(on?.notice).toEqual({ message: `Started · ${label}`, type: 'info', source: 'Survival', instanceId: 'srv-1' });
            expect(stopped({ source, kind: 'user_stop' })).toMatchObject({
                message: `Survival stopped (${label}).`,
                type: 'info',
                notice: { message: `Stopped · ${label}` },
            });
        }
        expect(started({ source: 'discord', userName: 'Kimchi', unexpected: false })?.notice?.message).toBe('Started · Discord · Kimchi');
        expect(stopped({ source: 'game', kind: 'in_game_stop' })).toMatchObject({
            message: 'Survival was stopped from inside the game.',
            type: 'info',
            notice: { message: 'Stopped from inside the game' },
        });
        expect(started({ source: 'external', unexpected: true })).toMatchObject({
            message: 'Survival was started outside Saba-chan. Saba-chan did not start this process.',
            type: 'warning',
            notice: { message: 'Started outside Saba-chan' },
        });
    });

    it('토스트를 띄우고 알림은 서버별 한 줄로 남기며, 서버가 바뀌는 이벤트는 한 번만 목록을 다시 읽는다', async () => {
        vi.useFakeTimers();
        window.showToast = vi.fn();
        window.__sabaNotice = { addNotice: vi.fn() } as unknown as typeof window.__sabaNotice;
        const stream = fakeStream();
        const refetch = vi.fn();
        const { unmount } = render(<DaemonEvents onServersChanged={refetch} />);
        await stream.emit({ id: 1, type: 'action.run', data: discordStop });
        await stream.emit({ id: 2, type: 'instance.exit', data: { instance: 'Survival', kind: 'user_stop' } });
        await stream.emit({ id: 3, type: 'instance.stopped', data: { instanceId: 'srv-1', instance: 'Survival', source: 'discord', userName: 'Kimchi', kind: 'user_stop' } });
        await act(async () => vi.advanceTimersByTime(400));
        expect(refetch).toHaveBeenCalledTimes(1);
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(window.showToast).toHaveBeenCalledWith('Survival stopped (Discord · Kimchi).', 'info', 4000, undefined);
        expect(window.__sabaNotice?.addNotice).toHaveBeenCalledTimes(1);
        expect(window.__sabaNotice?.addNotice).toHaveBeenCalledWith({
            message: 'Stopped · Discord · Kimchi',
            type: 'info',
            source: 'Survival',
            instanceId: 'srv-1',
        });
        // 새 판은 상단 바 업데이트 알약이 보인다: "N개 발견" 알림은 만들지 않는다
        await stream.emit({ id: 4, type: 'update.available', data: { components: [{ name: 'saba-core' }, 'gui'] } });
        expect(window.__sabaNotice?.addNotice).toHaveBeenCalledTimes(1);
        delete window.__sabaNotice;
        unmount();
        expect(stream.subs.size).toBe(0);
    });

    it('켠 서버의 모듈·게임 서버에 새 버전이 있으면 알림을 남긴다 (모듈은 업데이트 화면으로)', async () => {
        const stream = fakeStream();
        window.__sabaNotice = { addNotice: vi.fn() } as unknown as typeof window.__sabaNotice;
        const { unmount } = render(<DaemonEvents onServersChanged={vi.fn()} />);
        await stream.emit({
            id: 1,
            type: 'instance.update_notice',
            data: { instanceId: 'i', instance: 'Pal', kind: 'module', module: 'palworld', latest: '1.1.0' },
        });
        expect(window.__sabaNotice?.addNotice).toHaveBeenLastCalledWith(
            expect.objectContaining({
                message: 'Pal: a new version 1.1.0 of the palworld module is out.',
                action: 'openUpdateModal',
            }),
        );
        await stream.emit({
            id: 2,
            type: 'instance.update_notice',
            data: { instanceId: 'i', instance: 'Pal', kind: 'server' },
        });
        expect(window.__sabaNotice?.addNotice).toHaveBeenLastCalledWith(
            expect.objectContaining({
                message: "Pal: a new game server version is out. If players can't join, update the server.",
            }),
        );
        delete window.__sabaNotice;
        unmount();
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
