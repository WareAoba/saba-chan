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

async function loadModuleMetadata() {
    try {
        const modules = await ipc.getModules();

        for (const mod of modules) {
            if (mod.commands && mod.commands.fields) {
                moduleCommands[mod.name] = {};
                for (const cmd of mod.commands.fields) {
                    moduleCommands[mod.name][cmd.name] = cmd;
                    console.log(`[Resolver] Command '${cmd.name}' for module ${mod.name} (${cmd.http_method || 'N/A'})`);
                }
            }

            try {
                const toml = await ipc.getModuleDetail(mod.name);
                moduleMetadata[mod.name] = toml;
                console.log(`[Resolver] Metadata loaded: ${mod.name}`);
            } catch (e) {
                console.warn(`[Resolver] Could not load metadata for ${mod.name}:`, e.message);
            }
        }

        console.log(`[Resolver] Total modules with commands: ${Object.keys(moduleCommands).length}`);
    } catch (error) {
        console.error('[Resolver] Failed to load module metadata:', error.message);
    }
}

// ── 별명 맵 (항상 최신 반환) ──

function getModuleAliases() {
    return buildModuleAliasMap(botConfig, moduleMetadata);
}

function getCommandAliases() {
    return buildCommandAliasMap(botConfig, moduleMetadata);
}

function resolveModule(alias) {
    return resolveAlias(alias, getModuleAliases());
}

function resolveCommand(alias) {
    return resolveAlias(alias, getCommandAliases());
}

function checkModuleConflict(alias) {
    return checkAliasConflict(alias, getModuleAliases());
}

/**
 * 입력값이 알려진 모듈 별명인지 확인 (대소문자 무시)
 * @param {string} alias
 * @returns {boolean}
 */
function isKnownModuleAlias(alias) {
    const aliasMap = getModuleAliases();
    const lower = alias.toLowerCase();
    for (const key of Object.keys(aliasMap)) {
        if (key.startsWith('__')) continue;
        if (key.toLowerCase() === lower) return true;
    }
    return false;
}

// ── 조회 헬퍼 ──

function getConfig()               { return botConfig; }
function getModuleCommands(name)   { return moduleCommands[name] || {}; }
function getModuleMeta(name)       { return moduleMetadata[name] || {}; }
function getAllModuleMetadata()     { return moduleMetadata; }

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
