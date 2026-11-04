/**
 * 채널 배정 도우미 — 배정 화면은 서버 관리 탭(ModerationTab.js)에 있다.
 */

/** 릴레이 계약: 챗봇·게임 채팅·상황판은 길드당 종류마다 최대 20채널, 음악·알림은 1채널 (2026-11-04) */
export const CHATBOT_MAX = 20;

/** 여러 채널에 둘 수 있는 일 — 나머지(음악·알림)는 서버마다 한 채널 */
export const LIST_KINDS = ['chatbot', 'gameChat', 'board'];

const sameSet = (a, b) => a.length === b.length && a.every((x) => b.includes(x));

/** 바뀐 키만 보낸다: 릴레이 PUT은 준 키만 바꾼다 */
export function channelsPatch(saved, draft) {
    const patch = {};
    for (const k of LIST_KINDS) {
        if (!draft[k]) continue;
        if (!sameSet(saved[k] || [], draft[k])) patch[k] = draft[k];
    }
    for (const k of ['music', 'notify']) {
        if ((saved[k] || null) !== (draft[k] || null)) patch[k] = draft[k] || null;
    }
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
