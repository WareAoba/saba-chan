// require('dotenv').config();  // GUI에서 환경 변수로 전달하므로 불필요
const { Client, GatewayIntentBits, Collection } = require('discord.js');
const axios = require('axios');
const fs = require('fs');
const path = require('path');
const { buildModuleAliasMap, buildCommandAliasMap, resolveAlias, checkAliasConflict } = require('./utils/aliasResolver');
const i18n = require('./i18n'); // Initialize i18n

const client = new Client({ 
    intents: [
        GatewayIntentBits.Guilds, 
        GatewayIntentBits.GuildMessages,
        GatewayIntentBits.MessageContent
    ] 
});
const IPC_BASE = process.env.IPC_BASE || 'http://127.0.0.1:57474';

// ── Global axios defaults (timeout) ──
axios.defaults.timeout = 15000; // 15초 타임아웃

// ── IPC 토큰을 전용 변수로 관리 (axios.defaults.headers.common에 의존하지 않음) ──
let _botCachedIpcToken = '';

const _botTokenPath = process.env.SABA_TOKEN_PATH
    || path.join(process.env.APPDATA || process.env.HOME || '.', 'saba-chan', '.ipc_token');

function loadBotIpcToken() {
    // 환경 변수로 전달된 토큰이 있으면 우선 사용
    if (!_botCachedIpcToken && process.env.SABA_TOKEN) {
        _botCachedIpcToken = process.env.SABA_TOKEN;
    }
    try {
        if (fs.existsSync(_botTokenPath)) {
            const token = fs.readFileSync(_botTokenPath, 'utf8').trim();
            if (token) {
                const prev = _botCachedIpcToken;
                _botCachedIpcToken = token;
                if (prev !== token) {
                    console.log(`[Bot] IPC auth token loaded: ${token.substring(0, 8)}… from ${_botTokenPath}` +
                        (prev ? ` (was: ${prev.substring(0, 8)}…)` : ' (first load)'));
                }
                return true;
            }
        }
    } catch (e) {
        console.warn('[Bot] Could not read IPC token file:', e.message);
    }
    return false;
}

// 최초 토큰 로드
loadBotIpcToken();

// ── 요청 전 토큰 주입 인터셉터 ──
// 매 요청마다 _botCachedIpcToken 에서 헤더를 직접 설정
axios.interceptors.request.use((config) => {
    let token = _botCachedIpcToken;
    if (!token) {
        loadBotIpcToken();
        token = _botCachedIpcToken;
    }
    if (token) {
        if (typeof config.headers?.set === 'function') {
            config.headers.set('X-Saba-Token', token);
        } else if (config.headers) {
            config.headers['X-Saba-Token'] = token;
        }
    }
    return config;
});

// ── 401 응답 시 토큰 자동 재로드 + 재시도 인터셉터 ──
let _botTokenRefreshPromise = null;

axios.interceptors.response.use(
    (response) => response,
    async (error) => {
        const originalRequest = error.config;
        if (error.response && error.response.status === 401 && !originalRequest._retried) {
            originalRequest._retried = true;

            if (!_botTokenRefreshPromise) {
                _botTokenRefreshPromise = (async () => {
                    try {
                        const newToken = fs.readFileSync(_botTokenPath, 'utf8').trim();
                        if (newToken) {
                            _botCachedIpcToken = newToken;
                            console.log(`[Bot] Token refreshed after 401: ${newToken.substring(0, 8)}…`);
                            return newToken;
                        }
                    } catch (_) { /* 토큰 파일 읽기 실패 */ }
                    return null;
                })();
                _botTokenRefreshPromise.finally(() => {
                    setTimeout(() => { _botTokenRefreshPromise = null; }, 300);
                });
            }

            const refreshedToken = await _botTokenRefreshPromise;
            if (refreshedToken) {
                if (typeof originalRequest.headers?.set === 'function') {
                    originalRequest.headers.set('X-Saba-Token', refreshedToken);
                } else {
                    originalRequest.headers['X-Saba-Token'] = refreshedToken;
                }
                return axios(originalRequest);
            }
        }
        return Promise.reject(error);
    }
);

// ── Global error handlers ──
process.on('unhandledRejection', (reason, promise) => {
    console.error('[Bot] Unhandled rejection at:', promise, 'reason:', reason);
});

process.on('uncaughtException', (error) => {
    console.error('[Bot] Uncaught exception:', error);
});

// Load bot config (written by Electron main process)
let botConfig = {
    prefix: '!saba',  // 기본값: !saba (사바쨩)
    moduleAliases: {},  // 사용자가 GUI에서 추가
    commandAliases: {}  // 사용자가 GUI에서 추가
};

// 설정 파일 경로: 환경 변수 > 로컬 파일
const configPath = process.env.BOT_CONFIG_PATH || path.join(__dirname, 'bot-config.json');
console.log('Bot config path:', configPath);

// 설정 파일 로드 함수
function loadBotConfig() {
    if (fs.existsSync(configPath)) {
        try {
            const loaded = JSON.parse(fs.readFileSync(configPath, 'utf8'));
            botConfig = { ...botConfig, ...loaded };
            console.log('Bot config loaded:', botConfig);
        } catch (e) {
            console.error('Failed to load bot-config.json:', e.message);
        }
    } else {
        console.log('bot-config.json not found at:', configPath, '- using default config');
    }
}

// 초기 로드
loadBotConfig();

// Module metadata (loaded from IPC) - includes commands from module.toml
let moduleMetadata = {};
// Module commands (parsed from module list API)
let moduleCommands = {};  // { moduleName: { cmdName: CommandField } }

// Load all module aliases and commands from IPC
async function loadModuleMetadata() {
    try {
        const response = await axios.get(`${IPC_BASE}/api/modules`);
        const modules = response.data.modules || [];
        
        for (const module of modules) {
            // Store commands from module.toml (via /api/modules)
            if (module.commands && module.commands.fields) {
                moduleCommands[module.name] = {};
                for (const cmd of module.commands.fields) {
                    moduleCommands[module.name][cmd.name] = cmd;
                    console.log(`[Discord] Loaded command '${cmd.name}' for module ${module.name} (${cmd.http_method || 'N/A'})`);
                }
            }
            
            // Load additional metadata (aliases)
            try {
                const metaRes = await axios.get(`${IPC_BASE}/api/module/${module.name}`);
                const toml = metaRes.data.toml || {};
                moduleMetadata[module.name] = toml;
                console.log(`[Discord] Loaded aliases for module: ${module.name}`);
            } catch (e) {
                console.warn(`[Discord] Could not load metadata for module ${module.name}:`, e.message);
            }
        }
        
        console.log(`[Discord] Total modules with commands: ${Object.keys(moduleCommands).length}`);
    } catch (error) {
        console.error('[Discord] Failed to load module metadata:', error.message);
    }
}

// Get available commands for a module (from module.toml commands)
function getModuleCommands(moduleName) {
    return moduleCommands[moduleName] || {};
}

// ── 범용 응답 포맷터 ──────────────────────────────────────────
// 모듈이나 명령어 이름을 참조하지 않고, 데이터 구조만 보고 포맷합니다.
// 어떤 게임 모듈이든 동일한 로직으로 Discord 응답을 생성합니다.
function formatGenericResponse(data) {
    // 1) null / undefined → 성공 메시지
    if (data === null || data === undefined) {
        return i18n.t('bot:responses.command_complete');
    }
    // 2) 문자열 → 그대로 표시 (RCON 응답은 대부분 문자열)
    if (typeof data === 'string') {
        return data || i18n.t('bot:responses.command_complete');
    }
    // 3) 배열 → 리스트 포맷
    if (Array.isArray(data)) {
        if (data.length === 0) return i18n.t('bot:responses.empty_list');
        return formatArrayResponse(data);
    }
    // 4) 빈 객체 → 성공 메시지
    if (typeof data === 'object' && Object.keys(data).length === 0) {
        return i18n.t('bot:responses.command_complete');
    }
    // 5) 객체에 배열 필드가 있으면 그 배열을 리스트로 표시
    if (typeof data === 'object') {
        for (const [key, value] of Object.entries(data)) {
            if (Array.isArray(value)) {
                if (value.length === 0) {
                    return `📋 **${key}**: (empty)`;
                }
                return `📋 **${key}** (${value.length}):\n${formatArrayResponse(value)}`;
            }
        }
        // 6) 단순 key-value 객체 → 속성 나열
        const entries = Object.entries(data)
            .filter(([_, v]) => v !== null && v !== undefined)
            .map(([k, v]) => `• **${k}**: ${v}`)
            .join('\n');
        return entries || i18n.t('bot:responses.command_complete');
    }
    // 7) 기타 → 문자열 변환
    return String(data);
}

// 배열 요소를 Discord-friendly 형식으로 포맷
function formatArrayResponse(arr) {
    return arr.map((item, idx) => {
        if (typeof item === 'string') return `${idx + 1}. ${item}`;
        if (typeof item === 'object' && item !== null) {
            // name 또는 id 필드를 이름으로, 나머지는 부가 정보로 표시
            const name = item.name || item.id || item.userid || `#${idx + 1}`;
            const extras = Object.entries(item)
                .filter(([k]) => !['name', 'id'].includes(k))
                .map(([k, v]) => `${k}: ${v}`)
                .join(', ');
            return extras ? `• **${name}** (${extras})` : `• **${name}**`;
        }
        return `• ${item}`;
    }).join('\n');
}

client.commands = new Collection();

// 중복 메시지 처리 방지를 위한 캐시
const processedMessages = new Set();
const MESSAGE_CACHE_TTL = 5000; // 5초

// 메시지 리스닝
client.on('messageCreate', async (message) => {
    if (message.author.bot) return;

    // 중복 메시지 처리 방지
    if (processedMessages.has(message.id)) {
        console.log(`[Discord] Duplicate message detected: ${message.id}`);
        return;
    }
    processedMessages.add(message.id);
    setTimeout(() => processedMessages.delete(message.id), MESSAGE_CACHE_TTL);

    const content = message.content.trim();
    const prefix = botConfig.prefix;

    // Check if message starts with prefix
    if (!content.startsWith(prefix)) return;

    // Get current aliases (dynamic from modules + GUI)
    const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
    const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);

    // Parse: "!prefix 모듈별명 명령어별명 [추가인자...]"
    const args = content.slice(prefix.length).trim().split(/\s+/);

    // 이스터에그: "할건해야제" / "ㅎㄱㅎㅇㅈ"
    if (args.length === 1 && (args[0] === '할건해야제' || args[0] === 'ㅎㄱㅎㅇㅈ')) {
        const reply = Math.random() < 0.9 ? '반드시 가야제 ㅋㅋ' : '이건 에바제...';
        return message.reply(reply);
    }

    // 이스터에그: "갈래말래" / "ㄱㄹㅁㄹ"
    if (args.length === 1 && (args[0] === '갈래말래' || args[0] === 'ㄱㄹㅁㄹ')) {
        const reply = Math.random() < 0.9 ? '반드시 가야제 ㅋㅋ' : '안감 ㅈㅈㅇㅇ';
        return message.reply(reply);
    }

    // 이스터에그: 단답 반응
    const simpleEasterEggs = {
        '물': '🫗',
        '섹스': '🔞',
        '사랑해': '❤️',
    };
    if (args.length === 1 && simpleEasterEggs[args[0]]) {
        return message.reply(simpleEasterEggs[args[0]]);
    }

    // 이스터에그: 가위바위보
    if (args.length === 1 && args[0] === '가위바위보') {
        const playRound = async (channel, userId) => {
            await channel.send('✊✌️✋ 가위/바위/보 중에 하나 고르세요!');
            const filter = m => m.author.id === userId && ['가위', '바위', '보'].includes(m.content.trim());
            const collector = channel.createMessageCollector({ filter, max: 1, time: 15000 });
            collector.on('collect', async (m) => {
                const choices = ['가위', '바위', '보'];
                const botChoice = choices[Math.floor(Math.random() * 3)];
                const userChoice = m.content.trim();
                if (userChoice === botChoice) {
                    await m.reply(`${botChoice}! 다시!`);
                    playRound(channel, userId);
                } else {
                    const botWin = (botChoice === '가위' && userChoice === '보') ||
                                   (botChoice === '바위' && userChoice === '가위') ||
                                   (botChoice === '보' && userChoice === '바위');
                    const reply = await m.reply(`${botChoice}!`);
                    await reply.react(botWin ? '😋' : '😵');
                }
            });
            collector.on('end', (collected) => {
                if (collected.size === 0) {
                    channel.send('⏰ 시간 초과! 다음에 다시 도전하세요~');
                }
            });
        };
        await playRound(message.channel, message.author.id);
        return;
    }

    // Build help message with mounted modules and their aliases
    async function buildHelpMessage() {
        const prefix = botConfig.prefix;

        // Fetch actually mounted servers
        let mountedModules = [];
        try {
            const response = await axios.get(`${IPC_BASE}/api/servers`);
            const servers = response.data.servers || [];
            mountedModules = [...new Set(servers.map(s => s.module))];
        } catch (e) {
            console.warn('[Discord] Could not fetch servers for help:', e.message);
        }

        // Build reverse alias map: moduleName -> [aliases]
        const moduleAliasMap = buildModuleAliasMap(botConfig, moduleMetadata);
        const reverseAliasMap = {};
        for (const [alias, moduleName] of Object.entries(moduleAliasMap)) {
            if (alias === moduleName) continue;
            if (!reverseAliasMap[moduleName]) reverseAliasMap[moduleName] = [];
            reverseAliasMap[moduleName].push(alias);
        }

        const helpTitle = `📖 **${prefix}**`;

        const usage = `\n\`${prefix} <모듈> <명령어>\`\n`;

        let moduleInfo = '';
        if (mountedModules.length > 0) {
            moduleInfo = '\n**📦 모듈:**\n';
            for (const mod of mountedModules) {
                const aliases = reverseAliasMap[mod] || [];
                const aliasStr = aliases.length > 0 ? ` (${aliases.join(', ')})` : '';
                moduleInfo += `• **${mod}**${aliasStr}\n`;
            }
        } else {
            moduleInfo = '\n' + i18n.t('bot:help.no_modules');
        }

        return `${helpTitle}${usage}${moduleInfo}`;
    }
    
    if (args.length === 0 || args[0] === '') {
        await message.reply(await buildHelpMessage());
        return;
    }

    const firstArg = args[0];
    const secondArg = args[1];

    // Special commands
    if (firstArg === '도움' || firstArg === 'help') {
        await message.reply(await buildHelpMessage());
        return;
    }

    // Module-specific help: "!prefix palworld" or "!prefix pw"
    if (!secondArg) {
        // 별명 충돌 검사
        const aliasCheck = checkAliasConflict(firstArg, moduleAliases);
        if (aliasCheck.isConflict) {
            const modules = aliasCheck.conflictModules.join(', ');
            await message.reply(i18n.t('bot:errors.alias_conflict', {
                alias: firstArg,
                modules,
                defaultValue: `❌ Alias '${firstArg}' is ambiguous — it matches multiple modules: ${modules}. Please use a more specific alias.`,
            }));
            return;
        }
        const moduleName = resolveAlias(firstArg, moduleAliases);
        const cmds = getModuleCommands(moduleName);
        const cmdList = Object.keys(cmds);
        
        if (cmdList.length > 0) {
            const prefix = botConfig.prefix;
            const moduleTitle = i18n.t('bot:help.module_title', { module: moduleName });
            const helpStart = i18n.t('bot:modules.help_start');
            const helpStop = i18n.t('bot:modules.help_stop');
            const helpStatus = i18n.t('bot:modules.help_status');
            const restTitle = i18n.t('bot:modules.help_rest_title');
            
            let cmdHelp = `${moduleTitle}\n`;
            cmdHelp += `• \`${prefix} ${firstArg} start\` - ${helpStart}\n`;
            cmdHelp += `• \`${prefix} ${firstArg} stop\` - ${helpStop}\n`;
            cmdHelp += `• \`${prefix} ${firstArg} status\` - ${helpStatus}\n\n`;
            cmdHelp += `${restTitle}\n`;
            
            for (const [cmdName, cmdMeta] of Object.entries(cmds)) {
                const inputsStr = cmdMeta.inputs && cmdMeta.inputs.length > 0
                    ? cmdMeta.inputs.map(i => i.required ? `<${i.name}>` : `[${i.name}]`).join(' ')
                    : '';
                cmdHelp += `• \`${prefix} ${firstArg} ${cmdName}${inputsStr ? ' ' + inputsStr : ''}\` - ${cmdMeta.label || cmdName}\n`;
            }
            
            await message.reply(cmdHelp);
        } else {
            const prefix = botConfig.prefix;
            const moduleTitle = i18n.t('bot:help.module_title', { module: moduleName });
            const helpStart = i18n.t('bot:modules.help_start');
            const helpStop = i18n.t('bot:modules.help_stop');
            const helpStatus = i18n.t('bot:modules.help_status');
            
            await message.reply(
                `${moduleTitle}\n` +
                `• \`${prefix} ${firstArg} start\` - ${helpStart}\n` +
                `• \`${prefix} ${firstArg} stop\` - ${helpStop}\n` +
                `• \`${prefix} ${firstArg} status\` - ${helpStatus}`
            );
        }
        return;
    }

    if (firstArg === '목록' || firstArg === 'list') {
        try {
            const response = await axios.get(`${IPC_BASE}/api/servers`);
            const servers = response.data.servers || [];
            if (servers.length === 0) {
                const emptyMsg = i18n.t('bot:list.empty');
                await message.reply(emptyMsg);
            } else {
                const listTitle = i18n.t('bot:list.title');
                const list = servers.map(s => {
                    const statusIcon = s.status === 'running' ? '🟢' : '⚪';
                    const statusText = s.status === 'running' 
                        ? i18n.t('bot:status.running')
                        : i18n.t('bot:status.stopped');
                    return i18n.t('bot:list.item', { name: s.name, module: s.module, status: statusText, status_icon: statusIcon });
                }).join('\n');
                await message.reply(`${listTitle}\n${list}`);
            }
        } catch (error) {
            const errorMsg = i18n.t('bot:messages.command_error');
            await message.reply(`❌ ${errorMsg}: ${error.message}`);
        }
        return;
    }

    // Module + Command pattern: "!prefix 모듈 명령어"
    // 별명 충돌 검사
    const aliasConflict = checkAliasConflict(firstArg, moduleAliases);
    if (aliasConflict.isConflict) {
        const modules = aliasConflict.conflictModules.join(', ');
        await message.reply(i18n.t('bot:errors.alias_conflict', {
            alias: firstArg,
            modules,
            defaultValue: `❌ Alias '${firstArg}' is ambiguous — it matches multiple modules: ${modules}. Please use a more specific alias.`,
        }));
        return;
    }
    const moduleName = resolveAlias(firstArg, moduleAliases);
    const commandName = resolveAlias(secondArg, commandAliases);
    const extraArgs = args.slice(2);  // 추가 인자들

    console.log(`[Discord] ${message.author.tag}: ${prefix} ${firstArg} ${secondArg} → module=${moduleName}, command=${commandName}, args=${extraArgs.join(' ')}`);

    try {
        // Find server by module name
        const serversRes = await axios.get(`${IPC_BASE}/api/servers`);
        const servers = serversRes.data.servers || [];
        const server = servers.find(s => s.module === moduleName || s.name.includes(moduleName));

        if (!server) {
            const notFoundMsg = i18n.t('bot:server.not_found', { alias: firstArg, resolved: moduleName });
            await message.reply(notFoundMsg);
            return;
        }

        // Built-in commands (start, stop, status)
        if (commandName === 'start') {
            const startMsg = i18n.t('bot:server.start_request', { name: server.name });
            const statusMsg = await message.reply(startMsg);

            // 시작 방식 결정: 인스턴스별 managed_start 설정 우선, 없으면 모듈 interaction_mode
            const modMeta = moduleMetadata[moduleName] || {};
            const interactionMode = modMeta?.protocols?.interaction_mode
                || modMeta?.module?.interaction_mode;
            const instanceManagedStart = server.module_settings?.managed_start;
            let useManaged;
            if (instanceManagedStart === true || instanceManagedStart === 'true') {
                useManaged = true;
            } else if (instanceManagedStart === false || instanceManagedStart === 'false') {
                useManaged = false;
            } else {
                // 모듈의 interaction_mode가 'console'이면 managed, 아니면 native
                useManaged = (interactionMode === 'console');
            }

            let result;
            if (useManaged) {
                // Managed 모드: stdin/stdout 캡처 (GUI의 managedStart와 동일)
                result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/managed/start`, {});
            } else {
                // Native 모드: 프로세스만 실행
                result = await axios.post(`${IPC_BASE}/api/server/${server.name}/start`, {
                    module: server.module,
                    config: {}
                });
            }

            const completeMsg = i18n.t('bot:server.start_complete', { name: server.name });
            await statusMsg.edit(completeMsg);
            return;
        } 
        else if (commandName === 'stop') {
            const stopMsg = i18n.t('bot:server.stop_request', { name: server.name });
            const statusMsg = await message.reply(stopMsg);
            const result = await axios.post(`${IPC_BASE}/api/server/${server.name}/stop`, { force: false });
            const completeMsg = i18n.t('bot:server.stop_complete', { name: server.name });
            await statusMsg.edit(completeMsg);
            return;
        }
        else if (commandName === 'status') {
            const statusText = server.status === 'running' 
                ? i18n.t('bot:status.running')
                : i18n.t('bot:status.stopped');
            const pidText = server.pid ? `PID: ${server.pid}` : '';
            const checkMsg = i18n.t('bot:server.status_check', { name: server.name, status: statusText, pid_info: pidText });
            await message.reply(checkMsg);
            return;
        }

        // Check if command exists in module.toml commands
        const cmds = getModuleCommands(moduleName);
        const cmdMeta = cmds[commandName];

        if (!cmdMeta) {
            // module.toml에 정의되지 않은 명령어 → raw string으로 서버에 직접 전달
            // 예: "!mc say hello world" → stdin/rcon으로 "say hello world" 전송
            if (server.status !== 'running') {
                const defaultMsg = i18n.t('bot:server.not_running_default');
                await message.reply(`❌ ${defaultMsg}`);
                return;
            }

            // 원본 명령어 문자열 복원 (별칭 해석 전 secondArg + 나머지 인자)
            const rawCommand = [secondArg, ...extraArgs].join(' ');
            console.log(`[Discord] Raw command forward: "${rawCommand}" → ${server.name}`);

            try {
                // managed 모드면 stdin, 아니면 rcon으로 전달
                const modMeta = moduleMetadata[moduleName] || {};
                const interactionMode = modMeta?.protocols?.interaction_mode
                    || modMeta?.module?.interaction_mode;
                const instanceManagedStart = server.module_settings?.managed_start;
                let useStdin;
                if (instanceManagedStart === true || instanceManagedStart === 'true') {
                    useStdin = true;
                } else if (instanceManagedStart === false || instanceManagedStart === 'false') {
                    useStdin = false;
                } else {
                    useStdin = (interactionMode === 'console');
                }

                let result;
                if (useStdin) {
                    result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/stdin`, { command: rawCommand });
                } else {
                    result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/rcon`, { command: rawCommand, instance_id: server.id });
                }

                const response = result.data;
                if (response.error) {
                    await message.reply(`❌ ${response.error}`);
                } else {
                    const output = formatGenericResponse(response.data || response.response || response);
                    await message.reply(`✅ ${output}`);
                }
            } catch (error) {
                console.error('[Discord] Raw command error:', error.message);
                await message.reply(`❌ ${error.response?.data?.error || error.message}`);
            }
            return;
        }

        // Execute command from module.toml (method = 'rest', 'dual', or 'rcon')
        if (cmdMeta.method === 'rest' || cmdMeta.method === 'dual' || cmdMeta.method === 'rcon') {
            // 서버 실행 상태 확인
            if (server.status !== 'running') {
                const moduleErrors = moduleMetadata[moduleName]?.errors || {};
                const defaultMsg = i18n.t('bot:server.not_running_default');
                const errorMsg = moduleErrors.server_not_running || defaultMsg;
                const notRunningMsg = i18n.t('bot:server.not_running', { name: server.name, error: errorMsg });
                await message.reply(notRunningMsg);
                return;
            }

            // Build request body from extra args and inputs schema
            const body = {};
            if (cmdMeta.inputs && cmdMeta.inputs.length > 0) {
                for (let i = 0; i < cmdMeta.inputs.length; i++) {
                    const input = cmdMeta.inputs[i];
                    if (extraArgs[i]) {
                        body[input.name] = extraArgs[i];
                    } else if (input.required) {
                        const missingMsg = i18n.t('bot:command.missing_required', { 
                            arg_name: input.name,
                            prefix,
                            alias: firstArg,
                            command: secondArg,
                            description: input.label || input.name
                        });
                        await message.reply(missingMsg);
                        return;
                    }
                }
            }

            const executingMsg = i18n.t('bot:command.executing', { name: server.name, command: commandName });
            const statusMsg = await message.reply(executingMsg);

            let result;
            
            // ── 프로토콜 라우팅 (module.toml의 method 기반, 모듈 이름 참조 없음) ──
            //   rcon → RCON 템플릿 치환 후 /rcon 엔드포인트
            //   rest → REST endpoint_template + http_method 로 /rest 엔드포인트
            //   dual → Python lifecycle 모듈이 프로토콜 선택 (/command 엔드포인트)
            if (cmdMeta.method === 'rcon') {
                // RCON 명령어 구성: rcon_template에서 입력값 치환
                let rconCmd = cmdMeta.rcon_template || commandName;
                for (const [key, value] of Object.entries(body)) {
                    rconCmd = rconCmd.replace(`{${key}}`, value);
                }
                // 치환되지 않은 선택적 파라미터 제거
                rconCmd = rconCmd.replace(/\s*\{\w+\}/g, '').trim();
                
                console.log(`[Discord] RCON call: ${rconCmd}`);
                result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/rcon`, {
                    command: rconCmd,
                    instance_id: server.id
                });
            } else if (cmdMeta.method === 'dual') {
                // 모듈 커맨드 엔드포인트 사용 (플레이어 ID 자동 변환 등 모듈별 처리)
                console.log(`[Discord] Module call: ${commandName}`, body);
                result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/command`, {
                    command: commandName,
                    args: body,
                    instance_id: server.id
                });
            } else {
                // REST 직접 호출
                const endpoint = cmdMeta.endpoint_template || `/v1/api/${commandName}`;
                const httpMethod = (cmdMeta.http_method || 'GET').toUpperCase();
                
                console.log(`[Discord] REST ${httpMethod} ${endpoint}`, body);
                result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/rest`, {
                    endpoint,
                    method: httpMethod,
                    body,
                    instance_id: server.id,
                    rest_host: server.rest_host || '127.0.0.1',
                    rest_port: server.rest_port || 8212,
                    username: server.rest_username || 'admin',
                    password: server.rest_password || ''
                });
            }

            if (result.data.success) {
                // ── 범용 응답 포맷터 (모듈/명령어 이름 분기 없음) ──
                const responseText = formatGenericResponse(result.data.data);
                const completeMsg = i18n.t('bot:command.execute_complete', { name: server.name, command: commandName, response: responseText });
                await statusMsg.edit(completeMsg);
            } else {
                // ── 구조적 에러 분류 (문자열 매칭 대신 error_code 또는 HTTP 상태 기반) ──
                const errorText = result.data.error || i18n.t('bot:errors.unknown');
                const errorCode = result.data.error_code || '';
                const moduleErrors = moduleMetadata[moduleName]?.errors || {};
                
                // 1순위: 데몬이 반환한 error_code로 모듈 에러 메시지 매칭
                // 2순위: error_code 없으면 원본 errorText를 그대로 표시
                const friendlyError = (errorCode && moduleErrors[errorCode])
                    ? moduleErrors[errorCode]
                    : errorText;
                
                const failedMsg = i18n.t('bot:command.execute_failed', { name: server.name, error: friendlyError });
                await statusMsg.edit(failedMsg);
            }
        } else {
            const unsupportedMsg = i18n.t('bot:messages.command_error');
            await message.reply(`❓ ${unsupportedMsg}: ${cmdMeta.method || 'unknown'}`);
        }

    } catch (error) {
        console.error('[Discord] Command error:', error.message);
        const moduleErrors = moduleMetadata[moduleName]?.errors || {};
        
        // ── HTTP 상태 코드 기반 에러 분류 (문자열 매칭 제거) ──
        let errorMsg;
        
        if (error.response) {
            const status = error.response.status;
            const data = error.response.data;
            
            const statusErrors = {
                401: moduleErrors.auth_failed || i18n.t('bot:errors.auth_failed'),
                403: moduleErrors.auth_failed || i18n.t('bot:errors.auth_failed'),
                404: data?.error || i18n.t('bot:errors.not_found'),
                500: moduleErrors.internal_server_error || i18n.t('bot:errors.internal_server_error'),
                503: moduleErrors.server_not_running || i18n.t('bot:errors.service_unavailable'),
            };
            
            errorMsg = statusErrors[status] || (data?.error || error.message);
        } else if (error.code) {
            // 네트워크 에러 → 에러 코드 기반 분류
            const networkErrors = {
                'ECONNREFUSED': moduleErrors.connection_refused || i18n.t('bot:errors.connection_refused'),
                'ETIMEDOUT': moduleErrors.timeout || i18n.t('bot:errors.timeout'),
                'ENOTFOUND': i18n.t('bot:errors.host_not_found'),
            };
            errorMsg = networkErrors[error.code] || error.message;
        } else {
            errorMsg = error.message;
        }
        
        await message.reply(`❌ ${i18n.t('bot:errors.error_title')}: ${errorMsg}`);
    }
});

// Slash command handler (legacy, kept for compatibility)
client.on('interactionCreate', async (interaction) => {
    if (!interaction.isChatInputCommand()) return;

    try {
        if (interaction.commandName === 'server') {
            const subcommand = interaction.options.getSubcommand();
            const response = await axios.get(`${IPC_BASE}/api/servers`);
            await interaction.reply({ content: JSON.stringify(response.data, null, 2), ephemeral: true });
        }
    } catch (error) {
        // interaction이 이미 응답된 상태인지 확인
        if (interaction.replied || interaction.deferred) {
            await interaction.followUp({ content: `Error: ${error.message}`, ephemeral: true }).catch(() => {});
        } else {
            await interaction.reply({ content: `Error: ${error.message}`, ephemeral: true }).catch(() => {});
        }
    }
});

client.once('ready', async () => {
    console.log(`Discord Bot logged in as ${client.user.tag}`);
    console.log(`Prefix: ${botConfig.prefix}`);
    console.log(`Bot config aliases: ${JSON.stringify(botConfig.moduleAliases)}`);
    
    // Load module metadata from IPC
    console.log('Loading module metadata from IPC...');
    await loadModuleMetadata();
    
    const moduleAliases = buildModuleAliasMap(botConfig, moduleMetadata);
    const commandAliases = buildCommandAliasMap(botConfig, moduleMetadata);
    
    console.log(`Module aliases (combined): ${JSON.stringify(moduleAliases)}`);
    console.log(`Command aliases (combined): ${JSON.stringify(commandAliases)}`);
    console.log('Discord Bot ready');
});

client.login(process.env.DISCORD_TOKEN);
