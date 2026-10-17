/**
 * 디스코드 권한 레벨 (데몬 P1 계약, integration/p1-permissions-contract.md).
 * 원본은 데몬 config.json의 nodeSettings[guildId]이고, GUI는 /api/permissions로만 읽고 쓴다.
 * 모든 함수는 새 객체를 돌려주고 인자를 바꾸지 않는다.
 */

export const OWNER = 3;
/** 부여할 수 있는 레벨 (높은 것부터). 소유자는 부여할 수 없다 */
export const GRANTABLE = [2, 1, 0, -1];
/** 명령 요구 레벨로 고를 수 있는 값 (3 = 소유자만) */
export const REQUIRABLE = [3, 2, 1, 0];
export const PRESETS = ['loose', 'default', 'strict'];
export const RISKS = ['safe', 'caution', 'danger'];

const LEVEL_KEYS = { 3: 'owner', 2: 'admin', 1: 'user', 0: 'chatbot', '-1': 'banned' };
export const levelKey = (level) => LEVEL_KEYS[level] || 'user';

/** 데몬이 계약표를 안 실어 줘도 같은 값으로 판정한다 */
const PRESET_TABLE = {
    loose: { safe: 0, caution: 0, danger: 1 },
    default: { safe: 0, caution: 1, danger: 2 },
    strict: { safe: 1, caution: 2, danger: 3 },
};

/** 인스턴스마다 쓰는 내장 명령과 위험도 (help·list는 인스턴스에 매이지 않는다) */
export const BUILTIN_COMMANDS = [
    { name: 'status', risk: 'safe' },
    { name: 'start', risk: 'caution' },
    { name: 'stop', risk: 'caution' },
];

export const commandKey = (instanceId, command) => `${instanceId}/${command}`;

export const normalizeRisk = (risk) => (RISKS.includes(risk) ? risk : 'caution');

export function presetLevel(perm, risk) {
    const preset = PRESETS.includes(perm?.preset) ? perm.preset : 'default';
    const table = perm?.presets?.[preset] || PRESET_TABLE[preset];
    return table[normalizeRisk(risk)];
}

/** 명령 하나의 요구 레벨과 덮어썼는지 */
export function requiredLevel(perm, instanceId, command, risk) {
    const own = perm?.commandLevels?.[commandKey(instanceId, command)];
    if (typeof own === 'number') return { level: own, overridden: true };
    return { level: presetLevel(perm, risk), overridden: false };
}

/** 레벨별 멤버 묶음 (높은 것부터). 소유자는 맨 위에 따로 둔다 */
export function membersByLevel(perm) {
    const groups = [];
    if (perm?.ownerDiscordId) groups.push({ level: OWNER, ids: [String(perm.ownerDiscordId)] });
    const members = perm?.members || {};
    for (const level of GRANTABLE) {
        const ids = Object.keys(members).filter(
            (id) => members[id] === level && id !== String(perm?.ownerDiscordId ?? ''),
        );
        if (ids.length) groups.push({ level, ids });
    }
    return groups;
}

/**
 * 릴레이에 남은 부여값 중 데몬과 다른 것 (가져오기 후보).
 * 데몬은 자동으로 가져오지 않는다: 원본이 둘이 되지 않게 사용자가 골라서 데몬에 쓴다.
 */
export function importCandidates(perm, relayMembers) {
    const members = perm?.members || {};
    const owner = String(perm?.ownerDiscordId ?? '');
    return (relayMembers || []).filter(
        (m) => GRANTABLE.includes(m?.level) && String(m.id) !== owner && members[String(m.id)] !== m.level,
    );
}

/**
 * 릴레이가 보낸 소유자가 고정된 소유자와 다르면 그 id (같거나 모르면 null).
 * 필드 이름은 로드맵 스레드 구현을 따른다: relayOwnerDiscordId.
 */
export function ownerMismatch(perm) {
    const relay = perm?.relayOwnerDiscordId;
    if (!relay) return null;
    return String(relay) === String(perm?.ownerDiscordId ?? '') ? null : String(relay);
}
