import { describe, expect, it } from 'vitest';
import { channelsErrorText, channelsPatch } from '../components/discord/ChannelsTab';

describe('채널 배정 변환', () => {
    it('바뀐 키만 보낸다 (음악 해제는 null, 챗봇 전부 해제는 [])', () => {
        const saved = { chatbot: ['1', '2'], music: '3' };
        expect(channelsPatch(saved, { chatbot: ['2', '1'], music: '3' })).toEqual({});
        expect(channelsPatch(saved, { chatbot: [], music: null })).toEqual({ chatbot: [], music: null });
    });

    it('게임 채팅·상황판은 채널 목록 — 바꾼 것만, 순서만 다르면 보내지 않는다 (사용자 11-04)', () => {
        const saved = { chatbot: [], music: null, gameChat: ['1'], board: [] };
        expect(channelsPatch(saved, { ...saved, gameChat: ['1', '2'] })).toEqual({ gameChat: ['1', '2'] });
        expect(channelsPatch({ ...saved, gameChat: ['1', '2'] }, { ...saved, gameChat: ['2', '1'] })).toEqual({});
        expect(channelsPatch(saved, { ...saved, board: ['3'] })).toEqual({ board: ['3'] });
        expect(channelsPatch(saved, { ...saved, gameChat: [] })).toEqual({ gameChat: [] });
    });

    it('데몬·릴레이 검증 오류는 코드 대신 일반 문구로 보인다', () => {
        const t = (k) => k;
        expect(channelsErrorText({ error: 'VALIDATION_ERROR', details: [] }, t, String)).toBe(
            'discord.channels_err_generic',
        );
        expect(channelsErrorText({ error: 'INVALID_BODY' }, t, String)).toBe('discord.channels_err_generic');
    });

    it('저장하는 사이 이 서버가 다른 PC로 옮겨 갔으면(409 NODE_MOVED) 코드 대신 알린다', () => {
        const t = (k) => k;
        expect(channelsErrorText({ error: 'NODE_MOVED', error_code: 'NODE_MOVED' }, t, String)).toBe(
            'discord.channels_err_moved',
        );
    });
});
