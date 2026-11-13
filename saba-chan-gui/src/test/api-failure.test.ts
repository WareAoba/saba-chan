/**
 * 브리지의 실패 모양은 하나다: 데몬이 준 오류 본문 + error(문구)·error_code.
 * 화면은 utils/apiResult의 failed() 하나로 판정한다 (데몬 본문은 ok·success 중 하나만 싣거나 둘 다 없다).
 */
import { describe, expect, it, vi } from 'vitest';
import { createApi } from '../api/createApi';
import { createDaemonClient } from '../api/daemon';
import type { ApiBody } from '../types/daemon';
import { failed } from '../utils/apiResult';

const refused = { status: 0, error: 'connect refused', code: 'ECONNREFUSED' };

describe('데몬 호출 실패 모양', () => {
    it('HTTP 오류는 데몬 본문을 그대로 두고 error가 없으면 message나 상태로 채운다', async () => {
        const body = { ok: false, error: 'busy', error_code: 'instance_busy' };
        expect(await createDaemonClient(vi.fn().mockResolvedValue({ status: 409, data: body })).call('get', '/x')).toEqual(body);

        const messageOnly = { success: false, message: 'Wait' };
        expect(
            await createDaemonClient(vi.fn().mockResolvedValue({ status: 400, data: messageOnly })).call('get', '/x'),
        ).toEqual({ ...messageOnly, error: 'Wait' });

        const empty = await createDaemonClient(vi.fn().mockResolvedValue({ status: 500, data: '' })).call('get', '/x');
        expect(empty).toEqual({ error: 'Request failed with status code 500' });
    });

    it('연결 실패는 {error, error_code}다', async () => {
        expect(await createDaemonClient(vi.fn().mockResolvedValue(refused)).call('get', '/x')).toEqual({
            error: 'connect refused',
            error_code: 'ECONNREFUSED',
        });
    });

    it('2xx 본문은 ok·success가 무엇이든 그대로 돌려준다', async () => {
        const own = { success: false, ok: true, error_code: 'game_not_ready' };
        expect(await createDaemonClient(vi.fn().mockResolvedValue({ status: 200, data: own })).call('get', '/x')).toEqual(own);
    });

    it('옛 daemonCall·daemonJson·errorBody 경로가 모두 같은 모양으로 실패한다', async () => {
        const { api } = createApi({ invoke: vi.fn().mockResolvedValue(refused), hub: { dispatch: vi.fn(), on: vi.fn(), live: () => true } });
        const expected = { error: 'connect refused', error_code: 'ECONNREFUSED' };
        expect(await api.instanceStop('a')).toEqual(expected);
        expect(await api.chatbotGet()).toEqual(expected);
        expect(await api.makerInfo()).toEqual(expected);
        expect(await api.updaterStatus()).toEqual(expected);
        expect(await api.instanceEvents('a')).toEqual({ ...expected, events: [] });
    });
});

describe('failed()', () => {
    it('브리지 실패와 데몬의 ok·success:false 본문을 실패로 본다', () => {
        expect(failed(null)).toBe(true);
        expect(failed(undefined)).toBe(true);
        expect(failed({ error: 'x' })).toBe(true);
        expect(failed({ error_code: 'canceled' })).toBe(true);
        expect(failed({ ok: false })).toBe(true);
        expect(failed({ success: false })).toBe(true);
    });

    it('성공 본문은 실패로 보지 않는다', () => {
        expect(failed({})).toBe(false);
        expect(failed({ ok: true, items: [] })).toBe(false);
        expect(failed({ success: true, error: null, error_code: null } as unknown as ApiBody)).toBe(false);
    });
});
