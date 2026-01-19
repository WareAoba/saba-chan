require('dotenv').config();
const { Client, GatewayIntentBits, Collection } = require('discord.js');
const axios = require('axios');
const fs = require('fs');
const path = require('path');

const client = new Client({ 
    intents: [
        GatewayIntentBits.Guilds, 
        GatewayIntentBits.GuildMessages,
        GatewayIntentBits.MessageContent
    ] 
});
const IPC_BASE = process.env.IPC_BASE || 'http://localhost:57474';

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

// Get module aliases: GUI > module.toml > default (module name)
function getModuleAliases() {
    const combined = { ...botConfig.moduleAliases };
    
    // Add default: module name itself as alias
    for (const moduleName of Object.keys(moduleMetadata)) {
        if (!Object.values(combined).includes(moduleName)) {
            combined[moduleName] = moduleName;
        }
    }
    
    // Add all module aliases from module.toml [aliases].module_aliases
    for (const [moduleName, metadata] of Object.entries(moduleMetadata)) {
        if (metadata.aliases && metadata.aliases.module_aliases) {
            for (const alias of metadata.aliases.module_aliases) {
                combined[alias] = moduleName;
            }
        }
    }
    
    // Add custom GUI aliases with default fallback
    for (const [moduleName, customAlias] of Object.entries(botConfig.moduleAliases || {})) {
        const aliasStr = customAlias.trim();
        if (aliasStr.length > 0) {
            // User provided custom alias
            combined[aliasStr] = moduleName;
        } else {
            // Empty: use default (module name)
            combined[moduleName] = moduleName;
        }
    }
    
    return combined;
}

function getCommandAliases() {
    const combined = {};
    
    // Add default: command name itself as alias
    const defaultCommands = ['start', 'stop', 'status', 'difficulty'];
    for (const cmd of defaultCommands) {
        combined[cmd] = cmd;
    }
    
    // Add all command aliases from module.toml [aliases].commands
    for (const [moduleName, metadata] of Object.entries(moduleMetadata)) {
        if (metadata.aliases && metadata.aliases.commands) {
            for (const [cmdName, cmdData] of Object.entries(metadata.aliases.commands)) {
                // Default: command name itself
                combined[cmdName] = cmdName;
                
                // Handle both array format (legacy) and object format (new)
                const aliases = cmdData.aliases || (Array.isArray(cmdData) ? cmdData : []);
                for (const alias of aliases) {
                    combined[alias] = cmdName;
                }
            }
        }
    }
    
    // Add custom GUI aliases from bot-config.json (flatten nested structure)
    // botConfig.commandAliases: {module: {cmd: "alias1,alias2"}}
    for (const [moduleName, moduleCommands] of Object.entries(botConfig.commandAliases || {})) {
        if (typeof moduleCommands === 'object' && moduleCommands !== null) {
            for (const [cmdName, aliasStr] of Object.entries(moduleCommands)) {
                // Always add command name itself
                combined[cmdName] = cmdName;
                
                if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
                    // Parse comma-separated aliases
                    const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
                    for (const alias of aliases) {
                        combined[alias] = cmdName;
                    }
                }
            }
        }
    }
    
    return combined;
}

// Reverse lookup helper (case-insensitive)
function resolveAlias(input, aliases) {
    const lowerInput = input.toLowerCase();
    
    // Check if input is an alias (case-insensitive)
    for (const [key, value] of Object.entries(aliases)) {
        if (key.toLowerCase() === lowerInput) {
            // Ensure value is a string
            return typeof value === 'string' ? value : String(value);
        }
    }
    
    // Check if input is already the actual value (case-insensitive)
    const values = Object.values(aliases);
    for (const val of values) {
        // Skip non-string values
        if (typeof val !== 'string') continue;
        if (val.toLowerCase() === lowerInput) {
            return val;
        }
    }
    
    // Return input as-is (might be direct module/command name)
    return input;
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
    const moduleAliases = getModuleAliases();
    const commandAliases = getCommandAliases();

    // Parse: "!prefix 모듈별명 명령어별명 [추가인자...]"
    const args = content.slice(prefix.length).trim().split(/\s+/);
    
    // Build help message with module commands
    function buildHelpMessage() {
        const moduleList = Object.keys(moduleMetadata).join(', ') || '없음';
        
        // Collect all commands from all modules
        let moduleCommandsHelp = '';
        for (const [modName, cmds] of Object.entries(moduleCommands)) {
            const cmdNames = Object.keys(cmds);
            if (cmdNames.length > 0) {
                moduleCommandsHelp += `\n• **${modName}**: ${cmdNames.map(c => `\`${c}\``).join(', ')}`;
            }
        }

        return (
            `📖 **${prefix} 사용법**\n` +
            `• \`${prefix} 목록\` - 서버 목록 조회\n` +
            `• \`${prefix} <모듈> start\` - 서버 시작\n` +
            `• \`${prefix} <모듈> stop\` - 서버 정지\n` +
            `• \`${prefix} <모듈> status\` - 서버 상태\n` +
            `• \`${prefix} <모듈> <명령어>\` - REST 명령어 실행\n` +
            `• \`${prefix} 도움\` - 이 도움말\n\n` +
            `**사용 가능한 모듈:** ${moduleList}\n` +
            `**모듈별 명령어:**${moduleCommandsHelp || ' (없음)'}`
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
            let cmdHelp = `📖 **${moduleName} 명령어**\n`;
            cmdHelp += `• \`${prefix} ${firstArg} start\` - 서버 시작\n`;
            cmdHelp += `• \`${prefix} ${firstArg} stop\` - 서버 정지\n`;
            cmdHelp += `• \`${prefix} ${firstArg} status\` - 서버 상태\n\n`;
            cmdHelp += `**REST 명령어:**\n`;
            
            for (const [cmdName, cmdMeta] of Object.entries(cmds)) {
                const inputsStr = cmdMeta.inputs && cmdMeta.inputs.length > 0
                    ? cmdMeta.inputs.map(i => i.required ? `<${i.name}>` : `[${i.name}]`).join(' ')
                    : '';
                cmdHelp += `• \`${prefix} ${firstArg} ${cmdName}${inputsStr ? ' ' + inputsStr : ''}\` - ${cmdMeta.label || cmdName}\n`;
            }
            
            await message.reply(cmdHelp);
        } else {
            await message.reply(
                `📖 **${moduleName} 명령어**\n` +
                `• \`${prefix} ${firstArg} start\` - 서버 시작\n` +
                `• \`${prefix} ${firstArg} stop\` - 서버 정지\n` +
                `• \`${prefix} ${firstArg} status\` - 서버 상태`
            );
        }
        return;
    }

    if (firstArg === '목록' || firstArg === 'list') {
        try {
            const response = await axios.get(`${IPC_BASE}/api/servers`);
            const servers = response.data.servers || [];
            if (servers.length === 0) {
                await message.reply('📭 등록된 서버가 없습니다.');
            } else {
                const list = servers.map(s => `• **${s.name}** (${s.module}) - ${s.status === 'running' ? '🟢' : '⚪'} ${s.status}`).join('\n');
                await message.reply(`🎮 **서버 목록**\n${list}`);
            }
        } catch (error) {
            await message.reply(`❌ 오류: ${error.message}`);
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
            await message.reply(`❌ 모듈 "${firstArg}" (${moduleName})에 해당하는 서버를 찾을 수 없습니다.`);
            return;
        }

        // Built-in commands (start, stop, status)
        if (commandName === 'start') {
            const statusMsg = await message.reply(`⏳ **${server.name}** 서버를 시작합니다...`);
            const result = await axios.post(`${IPC_BASE}/api/server/${server.name}/start`, {
                module: server.module,
                config: {}
            });
            await statusMsg.edit(`✅ **${server.name}** 시작 요청 완료!`);
            return;
        } 
        else if (commandName === 'stop') {
            const statusMsg = await message.reply(`⏳ **${server.name}** 서버를 정지합니다...`);
            const result = await axios.post(`${IPC_BASE}/api/server/${server.name}/stop`, { force: false });
            await statusMsg.edit(`✅ **${server.name}** 정지 요청 완료!`);
            return;
        }
        else if (commandName === 'status') {
            const statusText = server.status === 'running' ? '🟢 실행 중' : '⚪ 정지됨';
            const pidText = server.pid ? `PID: ${server.pid}` : '';
            await message.reply(`📊 **${server.name}** 상태: ${statusText} ${pidText}`);
            return;
        }

        // Check if command exists in module.toml commands
        const cmds = getModuleCommands(moduleName);
        const cmdMeta = cmds[commandName];

        if (!cmdMeta) {
            // List available commands
            const availableCmds = Object.keys(cmds);
            if (availableCmds.length > 0) {
                await message.reply(
                    `❓ 알 수 없는 명령어: "${secondArg}" (${commandName})\n` +
                    `**사용 가능한 명령어:** ${availableCmds.map(c => `\`${c}\``).join(', ')}`
                );
            } else {
                await message.reply(`❓ 알 수 없는 명령어: "${secondArg}" (${commandName})`);
            }
            return;
        }

        // Execute REST command from module.toml
        if (cmdMeta.method === 'rest') {
            const endpoint = cmdMeta.endpoint_template || `/v1/api/${commandName}`;
            const httpMethod = (cmdMeta.http_method || 'GET').toUpperCase();
            
            // Build request body from extra args and inputs schema
            const body = {};
            if (cmdMeta.inputs && cmdMeta.inputs.length > 0) {
                for (let i = 0; i < cmdMeta.inputs.length; i++) {
                    const input = cmdMeta.inputs[i];
                    if (extraArgs[i]) {
                        body[input.name] = extraArgs[i];
                    } else if (input.required) {
                        await message.reply(
                            `❌ 필수 인자가 부족합니다: \`${input.name}\`\n` +
                            `사용법: \`${prefix} ${firstArg} ${secondArg} <${input.name}>\`\n` +
                            `설명: ${input.label || input.name}`
                        );
                        return;
                    }
                }
            }

            const statusMsg = await message.reply(`⏳ **${server.name}** - \`${commandName}\` 실행 중...`);

            // Call REST API via daemon
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

            const result = await axios.post(`${IPC_BASE}/api/instance/${server.id}/rest`, payload);

            if (result.data.success) {
                // Format response based on command type
                let responseText = '';
                const data = result.data.data;

                if (commandName === 'players' && data?.response?.players) {
                    const players = data.response.players;
                    if (players.length === 0) {
                        responseText = '현재 접속 중인 플레이어가 없습니다.';
                    } else {
                        responseText = `**접속 중인 플레이어 (${players.length}명)**\n`;
                        responseText += players.map(p => 
                            `• **${p.name}** - Lv.${p.level || '?'} (Ping: ${p.ping || '?'}ms)`
                        ).join('\n');
                    }
                } else if (commandName === 'info' && data?.response) {
                    const info = data.response;
                    responseText = `**서버 정보**\n` +
                        `• 버전: ${info.version || 'N/A'}\n` +
                        `• 서버명: ${info.servername || 'N/A'}\n` +
                        `• 설명: ${info.description || 'N/A'}`;
                } else if (commandName === 'metrics' && data?.response) {
                    const m = data.response;
                    responseText = `**서버 메트릭**\n` +
                        `• 현재 플레이어: ${m.currentplayernum || 0}/${m.maxplayernum || 0}\n` +
                        `• 서버 FPS: ${m.serverfps || 'N/A'}\n` +
                        `• 가동 시간: ${m.uptime ? Math.floor(m.uptime / 60) + '분' : 'N/A'}`;
                } else if (data?.response_text) {
                    responseText = data.response_text || '(응답 없음)';
                } else {
                    responseText = '✅ 명령어 실행 완료!';
                }

                await statusMsg.edit(`📡 **${server.name}** - \`${commandName}\`\n${responseText}`);
            } else {
                await statusMsg.edit(`❌ 실행 실패: ${result.data.error || '알 수 없는 오류'}`);
            }
        } else {
            await message.reply(`❓ 지원되지 않는 명령어 타입: ${cmdMeta.method || 'unknown'}`);
        }

    } catch (error) {
        console.error('[Discord] Command error:', error.message);
        await message.reply(`❌ 오류: ${error.response?.data?.error || error.message}`);
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
    
    const moduleAliases = getModuleAliases();
    const commandAliases = getCommandAliases();
    
    console.log(`Module aliases (combined): ${JSON.stringify(moduleAliases)}`);
    console.log(`Command aliases (combined): ${JSON.stringify(commandAliases)}`);
    console.log('Discord Bot ready');
});

client.login(process.env.DISCORD_TOKEN);
