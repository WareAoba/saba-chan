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

/** 아래(금지)부터 위(소유자)로 센 레벨 번호. 금지가 Lv 0이다 (화면 표시용, 저장값과 무관) */
export function levelNumber(perm, id) {
    const ladder = ladderOf(perm);
    const at = ladder.findIndex((l) => l.id === id);
    return at < 0 ? null : ladder.length - 1 - at;
}

/** "Lv 2 · 유저" */
export function levelTitle(t, perm, id) {
    return t('discord.perm_level_numbered', { n: levelNumber(perm, id), name: levelLabel(t, perm, id) });
}

/** 사다리 id (낮은 것부터) */
export const ascending = (perm) =>
    ladderOf(perm)
        .map((l) => l.id)
        .reverse();

/** 모든 인스턴스 보기 */
export const ALL_INSTANCES = '*';

/**
 * 계단 칩 (데몬 `GET /api/permissions/:guildId/commands` 항목을 묶는다).
 * 같은 이름의 명령은 한 칩이다. 모든 인스턴스 보기에서는 가장 많은 인스턴스가 있는 레벨에 칩 하나를 두고,
 * 다른 레벨인 인스턴스는 꼬리표(`instance`) 칩으로 따로 둔다. 큰 칩을 옮기면 그 명령의 모든 인스턴스가 옮겨진다
 */
export function stairChips(board, view = ALL_INSTANCES) {
    const items = (board?.items || []).filter((i) => view === ALL_INSTANCES || !i.instance || i.instance === view);
    const groups = new Map();
    for (const item of items) {
        const id = `${item.instance ? 'i' : 'g'}:${item.name}`;
        if (!groups.has(id)) groups.set(id, []);
        groups.get(id).push(item);
    }
    const chips = [];
    for (const [id, list] of groups) {
        const byLevel = new Map();
        for (const item of list) byLevel.set(item.level, [...(byLevel.get(item.level) || []), item]);
        const [[level, main]] = [...byLevel].sort((a, b) => b[1].length - a[1].length);
        const head = list[0];
        const chip = { title: head.title, risk: head.risk, kind: head.kind, fixed: head.fixed, name: head.name };
        chips.push({ ...chip, id, level, items: list, overridden: main.some((i) => i.overridden) });
        for (const item of list.filter((i) => i.level !== level)) {
            chips.push({
                ...chip,
                id: `${id}@${item.instance}`,
                level: item.level,
                items: [item],
                instance: item.instance,
                overridden: item.overridden,
            });
        }
    }
    return chips;
}

/**
 * 칩을 `level`로 옮기는 `commandLevels` 바꾸기. 프리셋 값과 같으면 덮어쓰기를 지운다(`null`),
 * 다르면 첫 키에 쓰고 나머지 키(예전 키)는 지운다. 고정 레벨 항목은 건드리지 않는다
 */
export function levelPatch(items, level) {
    const out = {};
    for (const item of items) {
        if (item.fixed) continue;
        const keys = item.keys?.length ? item.keys : [item.key];
        keys.forEach((k, i) => {
            out[k] = i === 0 && level !== item.preset ? level : null;
        });
    }
    return out;
}

/** 프리셋과 다르게 옮긴 항목 수 */
export const overriddenCount = (board) => (board?.items || []).filter((i) => i.overridden).length;

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

const nameCollator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** 이름순 새 배열 (대소문자 무시, 숫자는 크기 순). 멤버 목록이 디스코드 캐시 순서로 섞여 보이지 않게 */
export function sortByName(items, nameOf) {
    return [...items].sort((a, b) => nameCollator.compare(String(nameOf(a) ?? ''), String(nameOf(b) ?? '')));
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
