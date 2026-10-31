/**
 * 채널 배정 도우미 — 배정 화면은 서버 관리 탭(ModerationTab.js)에 있다.
 */

/** 릴레이 계약: 길드당 챗봇 채널 최대 20개, 음악·알림·게임 채팅 채널 각 1개 (assign/contract.md, I3) */
export const CHATBOT_MAX = 20;

const sameSet = (a, b) => a.length === b.length && a.every((x) => b.includes(x));

/** 바뀐 키만 보낸다: 릴레이 PUT은 준 키만 바꾼다 */
export function channelsPatch(saved, draft) {
    const patch = {};
    if (!sameSet(saved.chatbot, draft.chatbot)) patch.chatbot = draft.chatbot;
    if ((saved.music || null) !== (draft.music || null)) patch.music = draft.music || null;
    if ((saved.notify || null) !== (draft.notify || null)) patch.notify = draft.notify || null;
    if ((saved.gameChat || null) !== (draft.gameChat || null)) patch.gameChat = draft.gameChat || null;
    if ((saved.board || null) !== (draft.board || null)) patch.board = draft.board || null;
    return patch;
}

/** 데몬·릴레이 오류 → 한 줄 (error_code 또는 code 또는 error) */
export function channelsErrorText(res, t, nameOf) {
    const code = res?.error_code || res?.code || res?.error;
    switch (code) {
        case 'CHANNEL_CONFLICT':
            return t('discord.channels_err_conflict');
        case 'UNKNOWN_CHANNEL':
            return t('discord.channels_err_unknown', { channel: nameOf(res?.channelId) });
        case 'BOT_UNAVAILABLE':
            return t('discord.channels_err_bot');
        case 'NODE_MOVED':
            return t('discord.channels_err_moved');
        case 'INVALID_BODY':
        case 'VALIDATION_ERROR':
            return t('discord.channels_err_generic');
        default:
            return res?.message || res?.error || t('discord.channels_err_generic');
    }
}
