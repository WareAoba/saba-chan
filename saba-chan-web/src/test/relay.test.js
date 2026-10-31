import { describe, expect, it, vi } from 'vitest';
import { createRelay } from '../relay';

const sessionReturning = (reply) => ({ request: vi.fn().mockResolvedValue(reply) });

describe('릴레이 API (/relay/*)', () => {
    it('경로와 본문을 세션 요청으로 보낸다', async () => {
        const session = sessionReturning({ status: 200, data: { bigEmoji: true } });
        const relay = createRelay(session);
        const res = await relay.saveSettings('123456789012345678', { bigEmoji: true });
        expect(session.request).toHaveBeenCalledWith('PATCH', '/relay/guilds/123456789012345678/settings', {
            bigEmoji: true,
        });
        expect(res).toEqual({ ok: true, data: { bigEmoji: true } });
    });

    it('서버 관리 경로는 채널·사람 id 를 붙인다', async () => {
        const session = sessionReturning({ status: 200, data: {} });
        const relay = createRelay(session);
        await relay.modChannel('1', '2', { slowmode: 10 });
        await relay.modMute('1', '3', { duration: 'off' });
        expect(session.request.mock.calls.map((c) => [c[0], c[1]])).toEqual([
            ['PATCH', '/relay/guilds/1/moderation/channels/2'],
            ['PUT', '/relay/guilds/1/moderation/mutes/3'],
        ]);
    });

    it('실패는 릴레이 오류 코드로, 연결 실패는 status 0 으로 돌려준다', async () => {
        const conflict = await createRelay(sessionReturning({ status: 409, data: { error: 'NEEDS_PC' } })).saveChannels(
            '1',
            { notify: '2' },
        );
        expect(conflict).toEqual({ ok: false, status: 409, error: 'NEEDS_PC', data: { error: 'NEEDS_PC' } });
        const lost = await createRelay(sessionReturning({ status: 0, code: 'ECONNRESET' })).guilds();
        expect(lost).toMatchObject({ ok: false, status: 0, error: 'ECONNRESET' });
    });
});
