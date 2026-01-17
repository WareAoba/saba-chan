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

// Module metadata (loaded from IPC)
let moduleMetadata = {};

// Load all module aliases from IPC
async function loadModuleMetadata() {
    try {
        const response = await axios.get(`${IPC_BASE}/api/modules`);
        const modules = response.data.modules || [];
        
        for (const module of modules) {
            try {
                const metaRes = await axios.get(`${IPC_BASE}/api/module/${module.name}`);
                const toml = metaRes.data.toml || {};
                moduleMetadata[module.name] = toml;
                console.log(`[Discord] Loaded aliases for module: ${module.name}`);
            } catch (e) {
                console.warn(`[Discord] Could not load metadata for module ${module.name}:`, e.message);
            }
        }
    } catch (error) {
        console.error('[Discord] Failed to load module metadata:', error.message);
    }
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

// 메시지 리스닝
client.on('messageCreate', async (message) => {
    if (message.author.bot) return;

    const content = message.content.trim();
    const prefix = botConfig.prefix;

    // Check if message starts with prefix
    if (!content.startsWith(prefix)) return;

    // Get current aliases (dynamic from modules + GUI)
    const moduleAliases = getModuleAliases();
    const commandAliases = getCommandAliases();

    // Parse: "!prefix 모듈별명 명령어별명 [추가인자...]"
    const args = content.slice(prefix.length).trim().split(/\s+/);
    
    if (args.length === 0 || args[0] === '') {
        // Just prefix, show help
        // 사용자 커스텀 별명만 수집
        const userModuleAliases = [];
        for (const [moduleName, aliasStr] of Object.entries(botConfig.moduleAliases || {})) {
            if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
                const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
                userModuleAliases.push(...aliases);
            }
        }
        const moduleList = [...new Set([...Object.keys(moduleMetadata), ...userModuleAliases])].join(', ');

        const userCommandAliases = [];
        for (const [moduleName, cmds] of Object.entries(botConfig.commandAliases || {})) {
            if (typeof cmds === 'object') {
                for (const [cmd, aliasStr] of Object.entries(cmds)) {
                    if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
                        const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
                        userCommandAliases.push(...aliases);
                    }
                }
            }
        }
        const commandList = [...new Set(['start', 'stop', 'status', ...userCommandAliases])].join(', ');

        await message.reply(
            `📖 **${prefix} 사용법**\n` +
            `• \`${prefix} 목록\` - 서버 목록 조회\n` +
            `• \`${prefix} <모듈> 실행\` - 서버 시작\n` +
            `• \`${prefix} <모듈> 정지\` - 서버 정지\n` +
            `• \`${prefix} <모듈> 상태\` - 서버 상태\n` +
            `• \`${prefix} 도움\` - 이 도움말\n\n` +
            `**사용 가능한 모듈:** ${moduleList || '없음'}\n` +
            `**사용 가능한 명령어:** ${commandList || '없음'}`
        );
        return;
    }

    const firstArg = args[0];
    const secondArg = args[1];

    // Special commands
    if (firstArg === '도움' || firstArg === 'help') {
        // 사용자 커스텀 별명만 수집
        const userModuleAliases = [];
        for (const [moduleName, aliasStr] of Object.entries(botConfig.moduleAliases || {})) {
            if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
                const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
                userModuleAliases.push(...aliases);
            }
        }
        const moduleList = [...new Set([...Object.keys(moduleMetadata), ...userModuleAliases])].join(', ');

        const userCommandAliases = [];
        for (const [moduleName, cmds] of Object.entries(botConfig.commandAliases || {})) {
            if (typeof cmds === 'object') {
                for (const [cmd, aliasStr] of Object.entries(cmds)) {
                    if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
                        const aliases = aliasStr.split(',').map(a => a.trim()).filter(a => a.length > 0);
                        userCommandAliases.push(...aliases);
                    }
                }
            }
        }
        const commandList = [...new Set(['start', 'stop', 'status', ...userCommandAliases])].join(', ');

        await message.reply(
            `📖 **${prefix} 사용법**\n` +
            `• \`${prefix} 목록\` - 서버 목록 조회\n` +
            `• \`${prefix} <모듈> 실행\` - 서버 시작\n` +
            `• \`${prefix} <모듈> 정지\` - 서버 정지\n` +
            `• \`${prefix} <모듈> 상태\` - 서버 상태\n\n` +
            `**사용 가능한 모듈:** ${moduleList || '없음'}\n` +
            `**사용 가능한 명령어:** ${commandList || '없음'}`
        );
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
    if (!secondArg) {
        await message.reply(`❓ 명령어가 필요합니다. 예: \`${prefix} ${firstArg} 실행\``);
        return;
    }

    const moduleName = resolveAlias(firstArg, moduleAliases);
    const commandName = resolveAlias(secondArg, commandAliases);

    console.log(`[Discord] ${message.author.tag}: ${prefix} ${firstArg} ${secondArg} → module=${moduleName}, command=${commandName}`);

    try {
        // Find server by module name
        const serversRes = await axios.get(`${IPC_BASE}/api/servers`);
        const servers = serversRes.data.servers || [];
        const server = servers.find(s => s.module === moduleName || s.name.includes(moduleName));

        if (!server) {
            await message.reply(`❌ 모듈 "${firstArg}" (${moduleName})에 해당하는 서버를 찾을 수 없습니다.`);
            return;
        }

        // Execute command
        if (commandName === 'start') {
            await message.reply(`⏳ **${server.name}** 서버를 시작합니다...`);
            const result = await axios.post(`${IPC_BASE}/api/server/${server.name}/start`, {
                module: server.module,
                config: {}
            });
            await message.reply(`✅ **${server.name}** 시작 요청 완료!`);
        } 
        else if (commandName === 'stop') {
            await message.reply(`⏳ **${server.name}** 서버를 정지합니다...`);
            const result = await axios.post(`${IPC_BASE}/api/server/${server.name}/stop`, { force: false });
            await message.reply(`✅ **${server.name}** 정지 요청 완료!`);
        }
        else if (commandName === 'status') {
            const statusText = server.status === 'running' ? '🟢 실행 중' : '⚪ 정지됨';
            const pidText = server.pid ? `PID: ${server.pid}` : '';
            await message.reply(`📊 **${server.name}** 상태: ${statusText} ${pidText}`);
        }
        else {
            await message.reply(`❓ 알 수 없는 명령어: "${secondArg}" (${commandName})`);
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
