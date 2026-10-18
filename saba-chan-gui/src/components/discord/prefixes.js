/**
 * 디스코드 명령 접두사 (bot-config `prefixes`). 비어 있으면 릴레이가 길드 언어에 맞는 기본값을 쓴다
 * (/mnt/project-files/integration/chatbot-settings-v2.md §2·§3). 접두사는 챗봇의 호출어이기도 하다.
 */
export const PREFIX_MAX = 10;
export const PREFIX_MAX_LEN = 32;
/** 0.1.5까지의 기본값. 옛 설정에 남아 있으면 기본값을 쓰는 것으로 본다 */
const LEGACY_DEFAULT = '!saba';

/** bot-config → 저장된 접두사 목록 (옛 `prefix` 문자열도 읽는다) */
export function readPrefixes(botCfg) {
    if (Array.isArray(botCfg?.prefixes)) return botCfg.prefixes.filter((p) => typeof p === 'string' && p);
    const old = typeof botCfg?.prefix === 'string' ? botCfg.prefix.trim() : '';
    return old && old !== LEGACY_DEFAULT ? [old] : [];
}

/** 저장할 수 없는 접두사를 이유와 함께 돌려준다 */
export function prefixProblems(prefixes, t) {
    const out = [];
    if (prefixes.length > PREFIX_MAX) out.push(t('discord.prefix_err_count', { max: PREFIX_MAX }));
    const long = prefixes.filter((p) => [...p].length > PREFIX_MAX_LEN);
    if (long.length) out.push(t('discord.prefix_err_length', { max: PREFIX_MAX_LEN, words: long.join(', ') }));
    return out;
}

/**
 * 기본 접두사 글자. 릴레이가 알려 준 값(노드의 prefixDefault·prefixes)을 먼저 쓰고,
 * 아직 모르면 앱 언어의 기본값을 보인다 (릴레이도 호스트 앱 언어를 따른다).
 */
export function defaultPrefix(t, nodes = []) {
    const node = nodes.find((n) => n?.prefixDefault && Array.isArray(n.prefixes) && n.prefixes[0]);
    return node ? node.prefixes[0] : t('discord.default_prefix');
}

/** 예시·안내에 쓸 대표 접두사 */
export const primaryPrefix = (prefixes, t, nodes) => prefixes?.[0] || defaultPrefix(t, nodes);
