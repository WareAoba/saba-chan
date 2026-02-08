// require('dotenv').config();  // GUI에서 환경 변수로 전달하므로 불필요
const { Client, GatewayIntentBits, Collection } = require('discord.js');
const axios = require('axios');
const fs = require('fs');
const path = require('path');
const { buildModuleAliasMap, buildCommandAliasMap, resolveAlias } = require('./utils/aliasResolver');
const i18n = require('./i18n'); // Initialize i18n

const client = new Client({ 
    intents: [
        GatewayIntentBits.Guilds, 
        GatewayIntentBits.GuildMessages,
        GatewayIntentBits.MessageContent
    ] 
});
const IPC_BASE = process.env.IPC_BASE || 'http://127.0.0.1:57474';

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
    
    // Build help message with module commands
    function buildHelpMessage() {
        const moduleList = Object.keys(moduleMetadata).length > 0 
            ? Object.keys(moduleMetadata).join(', ') 
            : i18n.t('bot:help.no_modules');
        
        // Collect all commands from all modules
        let moduleCommandsHelp = '';
        for (const [modName, cmds] of Object.entries(moduleCommands)) {
            const cmdNames = Object.keys(cmds);
            if (cmdNames.length > 0) {
                moduleCommandsHelp += `\n• **${modName}**: ${cmdNames.map(c => `\`${c}\``).join(', ')}`;
            }
        }

        const prefix = botConfig.prefix;
        const helpTitle = `📖 **${prefix} ${i18n.t('bot:help.title')}**`;
        const helpList = `\`${prefix} list\` - ${i18n.t('bot:help.list')}`;
        const helpStart = `\`${prefix} <module> start\` - ${i18n.t('bot:help.start')}`;
        const helpStop = `\`${prefix} <module> stop\` - ${i18n.t('bot:help.stop')}`;
        const helpStatus = `\`${prefix} <module> status\` - ${i18n.t('bot:help.status')}`;
        const helpRest = `\`${prefix} <module> <command>\` - ${i18n.t('bot:help.rest_command')}`;
        const helpHelp = `\`${prefix} help\` - ${i18n.t('bot:help.help')}`;
        const availableModules = i18n.t('bot:help.available_modules', { modules: moduleList });
        const moduleCommandsTitle = i18n.t('bot:help.module_commands', { commands: moduleCommandsHelp || ' (none)' });

        return (
            `${helpTitle}\n` +
            `• ${helpList}\n` +
            `• ${helpStart}\n` +
            `• ${helpStop}\n` +
            `• ${helpStatus}\n` +
            `• ${helpRest}\n` +
            `• ${helpHelp}\n\n` +
            `${availableModules}\n` +
            `${moduleCommandsTitle}`
        );
    }
    
    if (args.length === 0 || args[0] === '') {
        await message.reply(buildHelpMessage());
        return;
    }

    const firstArg = args[0];
    const secondArg = args[1];

    // Special commands
    if (firstArg === '도움' || firstArg === 'help') {
        await message.reply(buildHelpMessage());
        return;
    }

    // Module-specific help: "!prefix palworld" or "!prefix pw"
    if (!secondArg) {
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
            const result = await axios.post(`${IPC_BASE}/api/server/${server.name}/start`, {
                module: server.module,
                config: {}
            });
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
            // List available commands
            const availableCmds = Object.keys(cmds);
            if (availableCmds.length > 0) {
                const unknownMsg = i18n.t('bot:command.unknown_command', { command: secondArg, resolved: commandName });
                const availableMsg = i18n.t('bot:help.available_commands', { commands: availableCmds.map(c => `\`${c}\``).join(', ') });
                await message.reply(`${unknownMsg}\n${availableMsg}`);
            } else {
                const unknownMsg = i18n.t('bot:command.no_available', { command: secondArg, resolved: commandName });
                await message.reply(unknownMsg);
            }
            return;
        }

        // Execute REST command from module.toml (method = 'rest' or 'dual')
        if (cmdMeta.method === 'rest' || cmdMeta.method === 'dual') {
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
            
            // 'dual' 메서드는 Python 모듈을 통해 실행 (플레이어 ID 변환 포함)
            // 'rest' 메서드는 REST API 직접 호출
            if (cmdMeta.method === 'dual') {
                // 모듈 커맨드 엔드포인트 사용 (플레이어 ID 자동 변환)
                const payload = {
                    command: commandName,
                    args: body,
                    instance_id: server.id
                };
                console.log(`[Discord] Module call: ${commandName}`, payload);
                result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/command`, payload);
            } else {
                // REST 직접 호출
                const endpoint = cmdMeta.endpoint_template || `/v1/api/${commandName}`;
                const httpMethod = (cmdMeta.http_method || 'GET').toUpperCase();
                
                const payload = {
                    endpoint,
                    method: httpMethod,
                    body,
                    instance_id: server.id,
                    rest_host: server.rest_host || '127.0.0.1',
                    rest_port: server.rest_port || 8212,
                    username: server.rest_username || 'admin',
                    password: server.rest_password || ''
                };
                console.log(`[Discord] REST call: ${httpMethod} ${endpoint}`, payload);
                result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/rest`, payload);
            }

            if (result.data.success) {
                // Format response based on command type
                let responseText = '';
                const data = result.data.data;

                console.log(`[Discord] Response data structure:`, JSON.stringify(data, null, 2));

                // Palworld REST API 응답은 data 안에 바로 들어있음
                const apiResponse = data;

                if (commandName === 'players') {
                    // players 응답: data.players 배열
                    const players = apiResponse?.players || [];
                    if (players.length === 0) {
                        responseText = i18n.t('bot:responses.players_empty');
                    } else {
                        const playersTitle = i18n.t('bot:responses.players_title', { count: players.length });
                        const playersList = players.map(p => 
                            i18n.t('bot:responses.players_item', { name: p.name, level: p.level || '?', id: p.userid || 'Unknown ID' })
                        ).join('\n');
                        responseText = `${playersTitle}\n${playersList}`;
                    }
                } else if (commandName === 'info') {
                    // info 응답: data에 바로 서버 정보
                    const infoTitle = i18n.t('bot:responses.info_title');
                    const infoVersion = i18n.t('bot:responses.info_version', { version: apiResponse?.version || 'N/A' });
                    const infoName = i18n.t('bot:responses.info_name', { name: apiResponse?.servername || 'N/A' });
                    const infoDesc = i18n.t('bot:responses.info_description', { description: apiResponse?.description || 'N/A' });
                    responseText = `${infoTitle}\n${infoVersion}\n${infoName}\n${infoDesc}`;
                } else if (commandName === 'metrics') {
                    // metrics 응답
                    const metricsTitle = i18n.t('bot:responses.metrics_title');
                    const metricsPlayers = i18n.t('bot:responses.metrics_players', { current: apiResponse?.currentplayernum || 0, max: apiResponse?.maxplayernum || 0 });
                    const metricsFps = i18n.t('bot:responses.metrics_fps', { fps: apiResponse?.serverfps || 'N/A' });
                    const uptime = apiResponse?.uptime ? Math.floor(apiResponse.uptime / 60) : 'N/A';
                    const metricsUptime = i18n.t('bot:responses.metrics_uptime', { uptime });
                    responseText = `${metricsTitle}\n${metricsPlayers}\n${metricsFps}\n${metricsUptime}`;
                } else if (commandName === 'announce') {
                    responseText = i18n.t('bot:responses.announce_success');
                } else if (commandName === 'save') {
                    responseText = i18n.t('bot:responses.save_success');
                } else if (commandName === 'kick' || commandName === 'ban') {
                    responseText = i18n.t('bot:responses.command_executed');
                } else {
                    // 기타 명령어는 data를 그대로 표시하거나 성공 메시지
                    if (typeof apiResponse === 'string') {
                        responseText = apiResponse;
                    } else if (apiResponse && Object.keys(apiResponse).length > 0) {
                        responseText = `\`\`\`json\n${JSON.stringify(apiResponse, null, 2)}\n\`\`\``;
                    } else {
                        responseText = i18n.t('bot:responses.command_complete');
                    }
                }

                const completeMsg = i18n.t('bot:command.execute_complete', { name: server.name, command: commandName, response: responseText });
                await statusMsg.edit(completeMsg);
            } else {
                // 에러 메시지를 모듈별 정의에서 가져오기
                const errorText = result.data.error || i18n.t('bot:errors.unknown');
                const moduleErrors = moduleMetadata[moduleName]?.errors || {};
                
                let friendlyError = errorText;
                // 모듈에 정의된 에러 메시지 매칭
                if (errorText.includes('인증') || errorText.includes('auth')) {
                    friendlyError = moduleErrors.auth_failed || i18n.t('bot:errors.auth_failed');
                } else if (errorText.includes('플레이어') || errorText.includes('player')) {
                    friendlyError = moduleErrors.player_not_found || i18n.t('bot:errors.player_not_found');
                } else if (errorText.includes('내부 오류') || errorText.includes('500')) {
                    friendlyError = moduleErrors.internal_server_error || i18n.t('bot:errors.internal_server_error');
                } else if (errorText.includes('REST API')) {
                    friendlyError = moduleErrors.rest_api_disabled || i18n.t('bot:errors.rest_api_disabled');
                } else if (errorText.includes('RCON')) {
                    friendlyError = moduleErrors.rcon_disabled || i18n.t('bot:errors.rcon_disabled');
                }
                
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
        
        let errorMsg = error.message;
        
        // 네트워크 에러 구분
        if (error.code === 'ECONNREFUSED') {
            errorMsg = moduleErrors.connection_refused || i18n.t('bot:errors.auth_failed');
        } else if (error.code === 'ETIMEDOUT') {
            errorMsg = moduleErrors.timeout || i18n.t('bot:errors.unknown');
        } else if (error.code === 'ENOTFOUND') {
            errorMsg = i18n.t('bot:errors.unknown');
        } else if (error.response) {
            // HTTP 에러 응답이 있는 경우
            const status = error.response.status;
            const data = error.response.data;
            
            if (status === 401 || status === 403) {
                errorMsg = moduleErrors.auth_failed || i18n.t('bot:errors.auth_failed');
            } else if (status === 404) {
                errorMsg = i18n.t('bot:errors.unknown');
            } else if (status === 500) {
                errorMsg = moduleErrors.internal_server_error || i18n.t('bot:errors.internal_server_error');
            } else if (status === 503) {
                errorMsg = moduleErrors.server_not_running || i18n.t('bot:errors.unknown');
            } else {
                errorMsg = data?.error || error.message;
            }
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
        await interaction.reply({ content: `Error: ${error.message}`, ephemeral: true });
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
