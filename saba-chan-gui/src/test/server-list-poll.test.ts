/**
 * 얇은 클라이언트 점검 3번 (2026-11-12): 서버 목록을 2초마다 받던 것을, 데몬 이벤트를 받으면 30초로 늦춘다.
 * 상태가 바뀌면 이벤트가 바로 다시 읽게 한다. 이벤트가 없는 경우(옛 셸, 익스텐션이 상태를 알려 주는 서버)는 사용자 주기 그대로.
 */
import { describe, expect, it } from 'vitest';
import { SERVER_CHANGING, serverListPollMs } from '../utils/daemonEvents';

describe('서버 목록 확인 주기', () => {
    it('이벤트를 받으면 느리게, 못 받으면 사용자 주기', () => {
        expect(serverListPollMs(2000, { events: true, extensionStatus: false })).toBe(30_000);
        expect(serverListPollMs(2000, { events: false, extensionStatus: false })).toBe(2000);
        expect(serverListPollMs(60_000, { events: true, extensionStatus: false })).toBe(60_000);
    });

    it('익스텐션이 상태를 알려 주는 서버가 있으면 이벤트가 없으니 사용자 주기', () => {
        expect(serverListPollMs(2000, { events: true, extensionStatus: true })).toBe(2000);
    });

    it('목록에 보이는 값이 바뀌는 이벤트에 다시 읽는다', () => {
        for (const kind of ['servers.changed', 'instance.player', 'instance.updated', 'instance.ready', 'instance.update_checked'])
            expect(SERVER_CHANGING).toContain(kind);
    });
});
