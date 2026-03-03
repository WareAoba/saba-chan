/**
 * 🔍 리졸버 — 별명/매핑 통합 관리
 * 
 * bot-config, 모듈 메타데이터, 별명 맵을 소유하고
 * 다른 모듈에 resolve 인터페이스를 제공합니다.
 */

const fs = require('fs');
const path = require('path');
const ipc = require('./ipc');
const {
    buildModuleAliasMap,
    buildCommandAliasMap,
    resolveAlias,
    checkAliasConflict,
} = require('../utils/aliasResolver');

// ── 봇 설정 ──
let botConfig = {
    prefix: '!saba',
    moduleAliases: {},
    commandAliases: {},
};

// ── 봇 설정 경로: AppData 기반 (환경변수 > %APPDATA%/saba-chan/bot-config.json > 로컬 fallback) ──
function resolveConfigPath() {
    if (process.env.BOT_CONFIG_PATH) {
        return process.env.BOT_CONFIG_PATH;
    }
    // %APPDATA%/saba-chan/bot-config.json (Windows) 또는 ~/.config/saba-chan/bot-config.json (Linux/macOS)
    const appData = process.platform === 'win32'
        ? process.env.APPDATA
        : (process.env.XDG_CONFIG_HOME || path.join(process.env.HOME || '', '.config'));
    if (appData) {
        return path.join(appData, 'saba-chan', 'bot-config.json');
    }
    // 최종 fallback: 실행 파일 옆
    return path.join(__dirname, '..', 'bot-config.json');
}

const configPath = resolveConfigPath();

let _configMtime = 0; // 마지막으로 읽은 파일 수정 시각

function loadConfig() {
    if (fs.existsSync(configPath)) {
        try {
            const loaded = JSON.parse(fs.readFileSync(configPath, 'utf8'));
            botConfig = { ...botConfig, ...loaded };
            _configMtime = fs.statSync(configPath).mtimeMs;
            const moduleAliasCount = Object.keys(botConfig.moduleAliases || {}).length;
            const commandAliasCount = Object.keys(botConfig.commandAliases || {}).length;
            console.log(`[Resolver] Bot config loaded (prefix=${botConfig.prefix}, moduleAliases=${moduleAliasCount}, commandAliases=${commandAliasCount})`);
        } catch (e) {
            console.error('[Resolver] Failed to load bot-config.json:', e.message);
        }
    } else {
        console.log('[Resolver] bot-config.json not found at:', configPath, '— using defaults');
    }
}

/**
 * 설정 파일 변경 감지 후 핫-리로드
 * (매 명령어 실행 전 호출 — 파일 mtime만 비교하므로 비용 최소)
 */
function reloadConfigIfChanged() {
    try {
        if (!fs.existsSync(configPath)) return;
        const mtime = fs.statSync(configPath).mtimeMs;
        if (mtime !== _configMtime) {
            console.log('[Resolver] Config file changed — reloading…');
            loadConfig();
        }
    } catch (_) {}
}

// ── 모듈 메타데이터 / 명령어 ──
let moduleMetadata = {};   // { moduleName: toml }
let moduleCommands = {};   // { moduleName: { cmdName: CommandField } }

async function loadModuleMetadata(guildId) {
    try {
        const modules = await ipc.getModules(guildId);
        const cmds = {};
        const meta = {};
        const moduleCommandSummary = [];
        let totalCommands = 0;
        let metadataLoaded = 0;
        let metadataFailed = 0;

        for (const mod of modules) {
            if (mod.commands && mod.commands.fields) {
                cmds[mod.name] = {};
                for (const cmd of mod.commands.fields) {
                    cmds[mod.name][cmd.name] = cmd;
                }

                const commandCount = Object.keys(cmds[mod.name]).length;
                if (commandCount > 0) {
                    moduleCommandSummary.push(`${mod.name}(${commandCount})`);
                    totalCommands += commandCount;
                }
            }

            try {
                const toml = await ipc.getModuleDetail(mod.name);
                meta[mod.name] = toml;
                metadataLoaded += 1;
            } catch (e) {
                metadataFailed += 1;
                console.warn(`[Resolver] Could not load metadata for ${mod.name}:`, e.message);
            }
        }

        moduleMetadata = meta;
        moduleCommands = cmds;

        const previewLimit = 8;
        const modulePreview = moduleCommandSummary.slice(0, previewLimit).join(', ');
        const moduleSuffix = moduleCommandSummary.length > previewLimit
            ? ` ... +${moduleCommandSummary.length - previewLimit}`
            : '';

        console.log(
            `[Resolver] Module metadata loaded: modules=${modules.length}, metadataOk=${metadataLoaded}, metadataFailed=${metadataFailed}, commands=${totalCommands}`
        );
        if (moduleCommandSummary.length > 0) {
            console.log(`[Resolver] Command map: ${modulePreview}${moduleSuffix}`);
        }
    } catch (error) {
        console.error('[Resolver] Failed to load module metadata:', error.message);
    }
}

// ── 별명 맵 (항상 최신 반환) ──

/**
 * 길드별 메타데이터 로드 (필요 시 레이지 로드)
 * @param {string} [guildId]
 */
async function ensureGuildMetadata(guildId) {
    // 로컬 모드에서는 초기화 시 이미 로드됨
}

function _getMetadata(guildId) {
    return moduleMetadata;
}
function _getCommands(guildId) {
    return moduleCommands;
}

function getModuleAliases(guildId) {
    return buildModuleAliasMap(botConfig, _getMetadata(guildId));
}

function getCommandAliases(guildId) {
    return buildCommandAliasMap(botConfig, _getMetadata(guildId));
}

function resolveModule(alias, guildId) {
    return resolveAlias(alias, getModuleAliases(guildId));
}

function resolveCommand(alias, guildId) {
    return resolveAlias(alias, getCommandAliases(guildId));
}

function checkModuleConflict(alias, guildId) {
    return checkAliasConflict(alias, getModuleAliases(guildId));
}

/**
 * 입력값이 알려진 모듈 별명인지 확인 (대소문자 무시)
 * @param {string} alias
 * @returns {boolean}
 */
function isKnownModuleAlias(alias, guildId) {
    const aliasMap = getModuleAliases(guildId);
    const lower = alias.toLowerCase();
    for (const key of Object.keys(aliasMap)) {
        if (key.startsWith('__')) continue;
        if (key.toLowerCase() === lower) return true;
    }
    return false;
}

// ── 조회 헬퍼 ──

function getConfig()               { return botConfig; }
function getModuleCommands(name, guildId) { return _getCommands(guildId)[name] || {}; }
function getModuleMeta(name, guildId)     { return _getMetadata(guildId)[name] || {}; }
function getAllModuleMetadata(guildId)     { return _getMetadata(guildId); }

// ── nodeSettings 접근 헬퍼 ──

/**
 * 모드별 nodeSettings 조회 (로컬 ↔ 클라우드 완전 분리)
 *   - 로컬 모드: guildId 키 우선, 'local' 폴백 (레거시 호환)
 *   - 클라우드 모드: guildId 키로만 조회 ('local' 폴백 없음)
 */
function _resolveNodeSettings(guildId) {
    const ns = botConfig.nodeSettings;
    if (!ns) return null;

    if (botConfig.mode === 'cloud') {
        // 클라우드: guildId 전용, 'local' 폴백 없음
        return (guildId && ns[guildId]) ? ns[guildId] : null;
    }

    // 로컬: guildId 우선 → 'local' 폴백 (레거시 단일 키 호환)
    if (guildId && ns[guildId]) return ns[guildId];
    return ns['local'] || null;
}

/**
 * 특정 노드(guildId)에서 허용된 인스턴스 목록
 * @param {string} guildId — 길드 ID 또는 'local'
 * @returns {string[]|null} — null이면 제한 없음(설정 미존재)
 */
function getAllowedInstances(guildId) {
    const cfg = _resolveNodeSettings(guildId);
    if (!cfg) return null; // 설정 없음 → 제한 없음
    return Array.isArray(cfg.allowedInstances) ? cfg.allowedInstances : null;
}

/**
 * 특정 노드에서 멤버가 특정 인스턴스에 대해 차단된 명령어 목록
 * @param {string} guildId
 * @param {string} userId
 * @param {string} serverId — 인스턴스 ID
 * @returns {string[]|null} — null이면 제한 없음(비관리 대상), 배열이면 해당 명령어만 차단
 */
function getMemberDeniedCommands(guildId, userId, serverId) {
    const cfg = _resolveNodeSettings(guildId);
    if (!cfg?.memberPermissions) return null; // 멤버 권한 설정 자체가 없음 → 제한 없음

    const memberPerms = cfg.memberPermissions[userId];
    if (memberPerms === undefined) return null; // 이 멤버에 대한 설정 없음 → 제한 없음

    // memberPerms: { [serverId]: string[] } — 차단된 명령어 목록
    const cmds = memberPerms[serverId];
    if (!Array.isArray(cmds)) return []; // 설정 없음 → 빈 배열(차단 없음 = 모두 허용)
    return cmds;
}

/**
 * 특정 노드에서 멤버가 권한 관리 대상인지 확인
 * (memberPermissions에 등록되어 있으면 관리 대상)
 * @param {string} guildId
 * @param {string} userId
 * @returns {boolean}
 */
function isMemberManaged(guildId, userId) {
    const cfg = _resolveNodeSettings(guildId);
    if (!cfg?.memberPermissions) return false;
    return userId in cfg.memberPermissions;
}

// ── 초기화 ──

async function init() {
    console.log('[Resolver] Config path:', configPath);
    loadConfig();

    console.log('[Resolver] Loading module metadata from IPC…');
    await loadModuleMetadata();

    const ma = getModuleAliases();
    const ca = getCommandAliases();
    const moduleAliasCount = Object.keys(ma).filter(k => !k.startsWith('__')).length;
    const commandAliasCount = Object.keys(ca).filter(k => !k.startsWith('__')).length;
    console.log(`[Resolver] Alias map ready: moduleAliases=${moduleAliasCount}, commandAliases=${commandAliasCount}`);
}

module.exports = {
    init,
    loadConfig,
    reloadConfigIfChanged,
    loadModuleMetadata,
    ensureGuildMetadata,
    getConfig,
    getModuleAliases,
    getCommandAliases,
    resolveModule,
    resolveCommand,
    checkModuleConflict,
    isKnownModuleAlias,
    getModuleCommands,
    getModuleMeta,
    getAllModuleMetadata,
    getAllowedInstances,
    getMemberDeniedCommands,
    isMemberManaged,
};
