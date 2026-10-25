/**
 * 디스코드 권한 레벨 (데몬 P1 계약, integration/p1-permissions-contract.md).
 * 원본은 데몬 config.json의 nodeSettings[guildId]이고, GUI는 /api/permissions로만 읽고 쓴다.
 * 길드마다 단계 사다리(`ladder`, 높은 것부터)가 있다: 단계를 더하고(id ≥ 10, 이름) 순서를 바꾸고 지울 수 있다.
 * 소유자는 늘 맨 위, 금지는 늘 맨 아래다. 높고 낮음은 숫자가 아니라 사다리 순서다.
 * 모든 함수는 새 객체를 돌려주고 인자를 바꾸지 않는다.
 */

export const OWNER = 3;
export const BANNED = -1;
/** 기본 단계 (높은 것부터). 지운 기본 단계는 이 순서로 가까운 단계를 찾는다 */
const BUILTIN = [3, 2, 1, 0, -1];
export const PRESETS = ['loose', 'default', 'strict'];
export const RISKS = ['safe', 'caution', 'danger'];

const LEVEL_KEYS = { 3: 'owner', 2: 'admin', 1: 'user', 0: 'chatbot', '-1': 'banned' };
/** 기본 단계의 로케일 키 (더한 단계는 'custom') */
export const levelKey = (level) => LEVEL_KEYS[level] || 'custom';

/** 이 길드의 사다리 (데몬이 안 실어 주면 기본 다섯 단계) */
export function ladderOf(perm) {
    const ladder = perm?.ladder;
    if (Array.isArray(ladder) && ladder.length >= 3) return ladder;
    return BUILTIN.map((id) => ({ id }));
}

/** 멤버에게 줄 수 있는 레벨 (높은 것부터, 소유자 제외) */
export const grantable = (perm) =>
    ladderOf(perm)
        .map((l) => l.id)
        .filter((id) => id !== OWNER);
/** 명령 요구 레벨로 고를 수 있는 값 (높은 것부터, 금지 제외) */
export const requirable = (perm) =>
    ladderOf(perm)
        .map((l) => l.id)
        .filter((id) => id !== BANNED);

/** 레벨 이름: 더한 단계는 그 이름, 기본 단계는 로케일 */
export function levelLabel(t, perm, id) {
    const custom = ladderOf(perm).find((l) => l.id === id)?.name;
    return custom || t(`discord.level_${levelKey(id)}`);
}

// ── 사다리 편집 (화면 초안). 새 단계는 id 없이 key만 있고 데몬이 번호를 준다 ──
let newKey = 0;
const movable = (ladder, i) => i > 0 && i < ladder.length - 1;

/** from 칸을 to 칸으로 옮긴다. 소유자·금지 칸이나 그 자리로는 옮기지 않는다 (그대로 돌려준다) */
export function moveLevel(ladder, from, to) {
    if (from === to || !movable(ladder, from) || !movable(ladder, to)) return ladder;
    const next = [...ladder];
    const [item] = next.splice(from, 1);
    next.splice(to, 0, item);
    return next;
}

/** 소유자 바로 아래에 새 단계를 더한다 */
export function addLevel(ladder, name) {
    newKey += 1;
    return [ladder[0], { key: `new-${newKey}`, name: name.trim() }, ...ladder.slice(1)];
}

export const removeLevel = (ladder, index) => (movable(ladder, index) ? ladder.filter((_, i) => i !== index) : ladder);

export const renameLevel = (ladder, index, name) => ladder.map((l, i) => (i === index ? { ...l, name } : l));

export const MAX_NAME = 32;
export const MAX_CUSTOM = 20;
export const isCustomLevel = (l) => l.id === undefined || !BUILTIN.includes(l.id);
export const canAddLevel = (ladder) => ladder.filter(isCustomLevel).length < MAX_CUSTOM;

/** 데몬 check_ladder 와 같은 규칙: 더한 단계 이름 1~32자, 20개까지 */
export function ladderValid(ladder) {
    const customs = ladder.filter(isCustomLevel);
    return (
        customs.length <= MAX_CUSTOM &&
        customs.every((l) => {
            const name = String(l.name ?? '').trim();
            return name.length > 0 && name.length <= MAX_NAME;
        })
    );
}

/** 데몬에 보낼 모양: 기본 단계는 {id}, 더한 단계는 {id?, name} */
export function ladderPatch(ladder) {
    return ladder.map((l) => {
        if (l.id !== undefined && BUILTIN.includes(l.id)) return { id: l.id };
        const name = String(l.name ?? '').trim();
        return l.id === undefined ? { name } : { id: l.id, name };
    });
}

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

/** 프리셋이 이 risk에 요구하는 레벨. 그 기본 단계를 지웠으면 더 엄한 쪽 (데몬 `preset_need`) */
export function presetLevel(perm, risk) {
    const preset = PRESETS.includes(perm?.preset) ? perm.preset : 'default';
    const table = perm?.presets?.[preset] || PRESET_TABLE[preset];
    const level = table[normalizeRisk(risk)];
    const ids = ladderOf(perm).map((l) => l.id);
    const at = BUILTIN.indexOf(level);
    return (
        BUILTIN.slice(0, at + 1)
            .reverse()
            .find((b) => ids.includes(b)) ?? OWNER
    );
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
    for (const level of grantable(perm)) {
        const ids = Object.keys(members).filter(
            (id) => members[id] === level && id !== String(perm?.ownerDiscordId ?? ''),
        );
        if (ids.length) groups.push({ level, ids });
    }
    return groups;
}

/** 디스코드 멤버 중 따로 레벨이 없는 사람 (소유자 제외): 길드 기본 레벨을 쓴다 */
export function defaultMembers(perm, discordMembers) {
    const members = perm?.members || {};
    const owner = String(perm?.ownerDiscordId ?? '');
    return (discordMembers || []).filter((m) => {
        const id = String(m?.id ?? '');
        return id && id !== owner && !(id in members);
    });
}

/**
 * 릴레이에 남은 부여값 중 데몬과 다른 것 (가져오기 후보).
 * 데몬은 자동으로 가져오지 않는다: 원본이 둘이 되지 않게 사용자가 골라서 데몬에 쓴다.
 */
export function importCandidates(perm, relayMembers) {
    const members = perm?.members || {};
    const owner = String(perm?.ownerDiscordId ?? '');
    return (relayMembers || []).filter(
        (m) => grantable(perm).includes(m?.level) && String(m.id) !== owner && members[String(m.id)] !== m.level,
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
