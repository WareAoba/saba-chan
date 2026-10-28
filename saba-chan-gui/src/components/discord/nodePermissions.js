/**
 * 디스코드 노드(길드) 권한 — bot-config.json의 nodeSettings를 다루는 순수 함수 모음.
 *
 * 데몬 해석(src/chat/mod.rs `allowed_instances`)과 똑같이 맞춘다:
 *   nodeSettings[guildId] = {
 *     allowedInstances?: [serverId],                 // 키가 없으면 모든 인스턴스 허용
 *   }
 * 인스턴스 하나는 노드 하나에만 고를 수 있다 (형제 노드끼리 단일 할당).
 * 모든 함수는 새 객체를 돌려주고 인자를 바꾸지 않는다.
 */

const EMPTY = Object.freeze({});

export function nodeConfig(settings, guildId) {
    return settings?.[guildId] || EMPTY;
}

/** 'all' = 제한 없음 (allowedInstances 키 없음), 'pick' = 고른 것만 */
export function instanceMode(settings, guildId) {
    return Array.isArray(nodeConfig(settings, guildId).allowedInstances) ? 'pick' : 'all';
}

export function isInstanceAllowed(settings, guildId, serverId) {
    const list = nodeConfig(settings, guildId).allowedInstances;
    return !Array.isArray(list) || list.includes(serverId);
}

/** 이 인스턴스를 고른 다른 노드 (없으면 null) */
export function instanceOwner(settings, nodeIds, serverId, exceptGuildId) {
    for (const id of nodeIds) {
        if (id === exceptGuildId) continue;
        const list = settings?.[id]?.allowedInstances;
        if (Array.isArray(list) && list.includes(serverId)) return id;
    }
    return null;
}

function withNode(settings, guildId, patch) {
    const cfg = { ...nodeConfig(settings, guildId), ...patch };
    for (const k of Object.keys(cfg)) if (cfg[k] === undefined) delete cfg[k];
    return { ...settings, [guildId]: cfg };
}

/**
 * 모드 전환. 'pick'으로 바꿀 때는 지금 보이는 것(다른 노드가 고른 것 제외)을 그대로 골라 둬서
 * 바꾸는 순간 접근이 끊기지 않게 한다.
 */
export function setInstanceMode(settings, guildId, mode, serverIds, nodeIds) {
    if (mode === 'all') return withNode(settings, guildId, { allowedInstances: undefined });
    if (instanceMode(settings, guildId) === 'pick') return settings;
    const picked = serverIds.filter((id) => !instanceOwner(settings, nodeIds, id, guildId));
    return withNode(settings, guildId, { allowedInstances: picked });
}

export function toggleInstance(settings, guildId, serverId, nodeIds) {
    const list = nodeConfig(settings, guildId).allowedInstances;
    if (!Array.isArray(list)) return settings;
    if (list.includes(serverId)) {
        return withNode(settings, guildId, { allowedInstances: list.filter((id) => id !== serverId) });
    }
    if (instanceOwner(settings, nodeIds, serverId, guildId)) return settings;
    return withNode(settings, guildId, { allowedInstances: [...list, serverId] });
}

// ── 명령어 단어(별명) ──

/** "a, b ,c" ↔ ['a','b','c'] — bot-config는 쉼표 문자열로 저장한다 */
export const parseAliasList = (str) =>
    (str || '')
        .split(',')
        .map((a) => a.trim())
        .filter(Boolean);
