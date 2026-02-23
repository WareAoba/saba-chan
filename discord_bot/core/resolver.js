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

const configPath = process.env.BOT_CONFIG_PATH
    || path.join(__dirname, '..', 'bot-config.json');

function loadConfig() {
    if (fs.existsSync(configPath)) {
        try {
            const loaded = JSON.parse(fs.readFileSync(configPath, 'utf8'));
            botConfig = { ...botConfig, ...loaded };
            console.log('[Resolver] Bot config loaded:', JSON.stringify(botConfig));
        } catch (e) {
            console.error('[Resolver] Failed to load bot-config.json:', e.message);
        }
    } else {
        console.log('[Resolver] bot-config.json not found at:', configPath, '— using defaults');
    }
}

// ── 모듈 메타데이터 / 명령어 ──
let moduleMetadata = {};   // { moduleName: toml }
let moduleCommands = {};   // { moduleName: { cmdName: CommandField } }

async function loadModuleMetadata(guildId) {
    try {
        const modules = await ipc.getModules(guildId);
        const cmds = {};
        const meta = {};

        for (const mod of modules) {
            if (mod.commands && mod.commands.fields) {
                cmds[mod.name] = {};
                for (const cmd of mod.commands.fields) {
                    cmds[mod.name][cmd.name] = cmd;
                    console.log(`[Resolver] Command '${cmd.name}' for module ${mod.name} (${cmd.http_method || 'N/A'})`);
                }
            }

            try {
                const toml = await ipc.getModuleDetail(mod.name);
                meta[mod.name] = toml;
                console.log(`[Resolver] Metadata loaded: ${mod.name}`);
            } catch (e) {
                console.warn(`[Resolver] Could not load metadata for ${mod.name}:`, e.message);
            }
        }

        moduleMetadata = meta;
        moduleCommands = cmds;

        console.log(`[Resolver] Total modules with commands: ${Object.keys(cmds).length}`);
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

// ── 초기화 ──

async function init() {
    console.log('[Resolver] Config path:', configPath);
    loadConfig();

    console.log('[Resolver] Loading module metadata from IPC…');
    await loadModuleMetadata();

    const ma = getModuleAliases();
    const ca = getCommandAliases();
    console.log(`[Resolver] Module aliases: ${JSON.stringify(ma)}`);
    console.log(`[Resolver] Command aliases: ${JSON.stringify(ca)}`);
}

module.exports = {
    init,
    loadConfig,
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
};
