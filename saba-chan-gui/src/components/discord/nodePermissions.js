/**
 * 디스코드 노드(길드) 권한 — bot-config.json의 nodeSettings를 다루는 순수 함수 모음.
 *
 * 데몬 해석(src/chat/mod.rs `allowed_instances` / `is_denied`)과 똑같이 맞춘다:
 *   nodeSettings[guildId] = {
 *     allowedInstances?: [serverId],                 // 키가 없으면 모든 인스턴스 허용
 *     memberPermissions?: { userId: { serverId: [막힌 명령] } }, // 없는 멤버는 제한 없음
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

// ── 멤버 ──

export function isMemberRestricted(settings, guildId, userId) {
    const perms = nodeConfig(settings, guildId).memberPermissions;
    return !!perms && Object.hasOwn(perms, userId);
}

export function blockedCommands(settings, guildId, userId, serverId) {
    const list = nodeConfig(settings, guildId).memberPermissions?.[userId]?.[serverId];
    return Array.isArray(list) ? list : [];
}

/** 이 멤버가 막힌 명령 수 (모든 인스턴스 합) */
export function blockedCount(settings, guildId, userId) {
    const perUser = nodeConfig(settings, guildId).memberPermissions?.[userId];
    if (!perUser) return 0;
    return Object.values(perUser).reduce((n, v) => n + (Array.isArray(v) ? v.length : 0), 0);
}

export function setMemberRestricted(settings, guildId, userId, restricted) {
    const perms = { ...(nodeConfig(settings, guildId).memberPermissions || {}) };
    if (restricted) {
        if (!perms[userId]) perms[userId] = {};
    } else {
        delete perms[userId];
    }
    return withNode(settings, guildId, { memberPermissions: perms });
}

function withUserServer(settings, guildId, userId, serverId, blocked) {
    const perms = { ...(nodeConfig(settings, guildId).memberPermissions || {}) };
    perms[userId] = { ...(perms[userId] || {}), [serverId]: blocked };
    return withNode(settings, guildId, { memberPermissions: perms });
}

/** 명령 하나 허용/차단 뒤집기 */
export function toggleMemberCommand(settings, guildId, userId, serverId, command) {
    const blocked = blockedCommands(settings, guildId, userId, serverId);
    const next = blocked.includes(command) ? blocked.filter((c) => c !== command) : [...blocked, command];
    return withUserServer(settings, guildId, userId, serverId, next);
}

/** 이 인스턴스의 명령 전부 허용(allow=true) 또는 전부 차단 */
export function setMemberAllCommands(settings, guildId, userId, serverId, commands, allow) {
    return withUserServer(settings, guildId, userId, serverId, allow ? [] : [...commands]);
}

// ── 명령어 단어(별명) ──

/** "a, b ,c" ↔ ['a','b','c'] — bot-config는 쉼표 문자열로 저장한다 */
export const parseAliasList = (str) =>
    (str || '')
        .split(',')
        .map((a) => a.trim())
        .filter(Boolean);

/**
 * 모듈 하나의 편집용 별명: 사용자가 저장한 것이 있으면 그것, 없으면 module.toml 기본값.
 * @returns {{ moduleAliases: string[], commands: Record<string,{aliases:string[],label:string,description:string,defaults:string[]}> }}
 */
export function aliasDraftFor(moduleName, defaults, savedModuleAliases, savedCommandAliases) {
    const base = defaults || {};
    const savedCmds = savedCommandAliases?.[moduleName];
    const commands = {};
    for (const [cmd, data] of Object.entries(base.commands || {})) {
        const def = Array.isArray(data) ? data : data?.aliases || [];
        commands[cmd] = {
            aliases: savedCmds && Object.hasOwn(savedCmds, cmd) ? parseAliasList(savedCmds[cmd]) : [...def],
            defaults: [...def],
            label: data?.label || cmd,
            description: data?.description || '',
        };
    }
    const hasSaved = savedModuleAliases && Object.hasOwn(savedModuleAliases, moduleName);
    return {
        moduleAliases: hasSaved ? parseAliasList(savedModuleAliases[moduleName]) : [...(base.module_aliases || [])],
        moduleDefaults: [...(base.module_aliases || [])],
        commands,
    };
}

/** 같은 모듈 안에서 두 명령이 같은 단어를 쓰면 봇이 고를 수 없다 */
export function commandAliasClashes(commands) {
    const seen = new Map();
    const clashes = [];
    for (const [cmd, data] of Object.entries(commands || {})) {
        for (const word of [cmd, ...(data.aliases || [])]) {
            const key = word.toLowerCase();
            const other = seen.get(key);
            if (other && other !== cmd) clashes.push({ alias: word, commands: [other, cmd] });
            else seen.set(key, cmd);
        }
    }
    return clashes;
}

/**
 * 인스턴스 별명 충돌: 봇은 인스턴스 이름 → 인스턴스 별명 → 모듈 단어 순으로 찾으므로
 * 한 단어가 두 인스턴스를 가리키거나, 이 모듈의 단어와 같으면 헷갈린다.
 * @param {Record<string,string[]>} draft 이 모듈 인스턴스들의 별명 (편집 중)
 * @param {Array<{id,name}>} servers 모든 인스턴스
 * @param {Record<string,string>} savedInstanceAliases 저장된 전체 인스턴스 별명 ("a, b")
 * @param {string[]} moduleWords 이 모듈의 단어 (모듈 이름 포함)
 * @returns {Array<{alias:string, instance:string, other:string}>}
 */
export function instanceAliasClashes(draft, servers, savedInstanceAliases, moduleWords) {
    const owner = new Map(); // 단어 → 가리키는 것 이름
    const add = (word, who) => {
        const k = word.toLowerCase();
        if (!owner.has(k)) owner.set(k, who);
    };
    for (const s of servers) if (s.name) add(s.name, s.name);
    for (const s of servers) {
        if (Object.hasOwn(draft, s.id)) continue;
        for (const a of parseAliasList(savedInstanceAliases?.[s.id])) add(a, s.name);
    }
    for (const w of moduleWords) add(w, w);
    const clashes = [];
    const nameOf = (id) => servers.find((s) => s.id === id)?.name || id;
    for (const [id, words] of Object.entries(draft)) {
        for (const w of words) {
            const k = w.toLowerCase();
            const other = owner.get(k);
            if (other !== undefined && other !== nameOf(id)) clashes.push({ alias: w, instance: nameOf(id), other });
            else owner.set(k, nameOf(id));
        }
    }
    return clashes;
}
