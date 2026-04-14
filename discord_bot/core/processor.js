/**
 * ⚙️ 프로세서 — 명령어 해석 및 디스패치
 * 
 * messageCreate 이벤트를 처리합니다:
 *   1. prefix 확인 → 토큰 파싱
 *   2. 핸들러(익스텐션) 우선 시도
 *   3. 내장 명령(help, list) 처리
 *   4. 모듈 + 명령어 패턴 → IPC 라우팅
 */

const i18n = require('../i18n');
const resolver = require('./resolver');
const handler = require('./handler');
const ipc = require('./ipc');
const musicExtension = require('../extensions/music');

// ── 중복 메시지 방지 ──
const processedMessages = new Set();
const MESSAGE_CACHE_TTL = 5000;

// ── 다중 길드 불허가 (로컬 모드) ──
/**
 * 봇이 2개 이상의 길드에 소속되어 있으면 명령어를 거부합니다.
 * 릴레이 모드에서는 client가 없으므로 이 체크를 건너뜁니다.
 * @returns {boolean} true이면 차단됨 (메시지 이미 전송)
 */
async function checkMultiGuildBlock(message) {
    const client = message.client;
    if (!client) return false;
    const guildCount = client.guilds.cache.size;
    if (guildCount <= 1) return false;
    console.warn(`[Processor] Multi-guild block: bot is in ${guildCount} guilds`);
    await message.reply(i18n.t('bot:errors.local_single_guild_only', { count: guildCount }));
    return true;
}

// ── 노드별 인스턴스 필터링 헬퍼 ──

/**
 * 노드(guildId)에서 허용된 인스턴스만 필터링하여 반환
 * nodeSettings에 해당 노드 설정이 없으면 전체 인스턴스 반환 (제한 없음)
 */
async function getFilteredServers(guildId) {
    const servers = await ipc.getServers(guildId);
    const allowed = resolver.getAllowedInstances(guildId);
    if (!allowed) {
        console.log(`[Processor] getFilteredServers(${guildId}): no restriction — ${servers.length} server(s)`);
        return servers; // 제한 없음
    }
    const filtered = servers.filter(s => allowed.includes(s.id));
    console.log(`[Processor] getFilteredServers(${guildId}): allowed=${JSON.stringify(allowed)}, total=${servers.length}, filtered=${filtered.length}`);
    return filtered;
}

/**
 * 모듈 별명(또는 인스턴스 이름)이 필터링된 서버 목록에 존재하는지 확인.
 * 비활성화된 인스턴스의 모듈은 "마운트되지 않은 것"으로 처리.
 */
async function isModuleMounted(moduleAlias, guildId) {
    try {
        const moduleName = resolver.resolveModule(moduleAlias, guildId);
        const servers = await getFilteredServers(guildId);
        return servers.some(s =>
            s.module === moduleName ||
            s.name.toLowerCase() === moduleAlias.toLowerCase()
        );
    } catch {
        return false;
    }
}

/**
 * 메시지 프로세서 진입점
 * @param {import('discord.js').Message} message
 */
async function process(message) {
    try {
    if (message.author.bot) return;

    // 중복 메시지 처리 방지
    if (processedMessages.has(message.id)) {
        console.log(`[Processor] Duplicate message: ${message.id}`);
        return;
    }
    processedMessages.add(message.id);
    setTimeout(() => processedMessages.delete(message.id), MESSAGE_CACHE_TTL);

    const content = message.content.trim();
    const botConfig = resolver.getConfig();
    const prefix = botConfig.prefix;
    const guildId = message.guildId;   // ★ 길드별 메타데이터 해석용

    // ★ 전용 음악 채널 — prefix 없이 바로 처리
    if (guildId && musicExtension.channelUI.isMusicChannel(guildId, message.channel.id, botConfig)) {
        // 다중 길드 체크는 prefix 있는 명령에서만 수행 (전용 채널은 아래에서 체크)
        if (await checkMultiGuildBlock(message)) return;
        const handled = await musicExtension.handleMusicChannelMessage(message, botConfig);
        if (handled) return;
    }

    if (!content.startsWith(prefix)) return;

    // ★ 다중 길드 불허가 (로컬 모드)
    if (await checkMultiGuildBlock(message)) return;

    // ★ 설정 파일 변경 감지 → 핫 리로드 (GUI에서 저장한 nodeSettings 즉시 반영)
    await resolver.reloadConfigIfChanged();

    await resolver.ensureGuildMetadata(guildId);

    const args = content.slice(prefix.length).trim().split(/\s+/);

    // ① 핸들러(익스텐션) 우선 시도
    if (await handler.handle(message, args, botConfig)) return;

    // ── 로컬 명령어 처리 ──

    // ② 빈 명령 또는 help → 도움말
    if (args.length === 0 || args[0] === '') {
        await message.reply(await buildHelpMessage(guildId));
        return;
    }

    const firstArg = args[0];
    const secondArg = args[1];

    if (firstArg === '도움' || firstArg === 'help') {
        await message.reply(await buildHelpMessage(guildId));
        return;
    }

    // ③ 목록 명령
    if (firstArg === '목록' || firstArg === 'list') {
        await handleListCommand(message, guildId);
        return;
    }

    // ④ 모듈만 (명령어 없음)
    if (!secondArg) {
        // 알려진 모듈 별명인지 + 필터링된 서버에 해당 모듈이 있는지 확인
        if (!resolver.isKnownModuleAlias(firstArg, guildId) || !(await isModuleMounted(firstArg, guildId))) {
            // 케이스 1: 알 수 없는 입력 또는 비활성 모듈
            await message.reply(i18n.t('bot:errors.unknown_input'));
            return;
        }
        // 케이스 2: 모듈 별명은 맞지만 명령어 없음 → 명령어 목록 안내
        await handleModuleHelp(message, firstArg, guildId);
        return;
    }

    // ⑤ 모듈 + 명령어 → IPC 라우팅 (비활성 모듈이면 알 수 없는 명령어 처리)
    if (resolver.isKnownModuleAlias(firstArg, guildId) && !(await isModuleMounted(firstArg, guildId))) {
        await message.reply(i18n.t('bot:errors.unknown_input'));
        return;
    }
    await handleModuleCommand(message, firstArg, secondArg, args.slice(2), guildId);
    } catch (e) {
        console.error('[Processor] Unhandled process error:', e.message);
        try {
            await message.reply(`❌ ${i18n.t('bot:errors.error_title')}`);
        } catch (_) { /* reply itself failed, nothing more to do */ }
    }
}

// ──────────────────────────────────────────
//  도움말
// ──────────────────────────────────────────

async function buildHelpMessage(guildId) {
    const botConfig = resolver.getConfig();
    const prefix = botConfig.prefix;

    let mountedModules = [];
    try {
        const servers = await getFilteredServers(guildId);
        mountedModules = [...new Set(servers.map(s => s.module))];
    } catch (e) {
        console.warn('[Processor] Could not fetch servers for help:', e.message);
    }

    const moduleAliasMap = resolver.getModuleAliases(guildId);
    const reverseAliasMap = {};
    for (const [alias, moduleName] of Object.entries(moduleAliasMap)) {
        if (alias === moduleName || alias.startsWith('__')) continue;
        if (!reverseAliasMap[moduleName]) reverseAliasMap[moduleName] = [];
        reverseAliasMap[moduleName].push(alias);
    }

    const helpTitle = `📖 **${prefix}**`;
    const usage = '\n' + i18n.t('bot:help.usage_format', { prefix }) + '\n';

    let moduleInfo = '';
    if (mountedModules.length > 0) {
        moduleInfo = '\n' + i18n.t('bot:help.modules_header') + '\n';
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

// ──────────────────────────────────────────
//  목록 (list)
// ──────────────────────────────────────────

async function handleListCommand(message, guildId) {
    try {
        const servers = await getFilteredServers(guildId);
        if (servers.length === 0) {
            await message.reply(i18n.t('bot:list.empty'));
        } else {
            const listTitle = i18n.t('bot:list.title');
            const list = servers.map(s => {
                const statusIcon = s.status === 'running' ? '🟢' : '⚪';
                const statusText = s.status === 'running'
                    ? i18n.t('bot:status.running')
                    : i18n.t('bot:status.stopped');
                return i18n.t('bot:list.item', {
                    name: s.name,
                    module: s.module,
                    status: statusText,
                    status_icon: statusIcon,
                });
            }).join('\n');
            await message.reply(`${listTitle}\n${list}`);
        }
    } catch (error) {
        console.error('[Processor] handleListCommand error:', error.message);
        await message.reply(`❌ ${i18n.t('bot:messages.command_error')}`).catch(() => {});
    }
}

// ──────────────────────────────────────────
//  모듈 도움말 (prefix + 모듈만)
// ──────────────────────────────────────────

async function handleModuleHelp(message, moduleAlias, guildId) {
    const moduleAliases = resolver.getModuleAliases(guildId);

    // 별명 충돌 검사
    const conflict = resolver.checkModuleConflict(moduleAlias, guildId);
    if (conflict.isConflict) {
        const modules = conflict.conflictModules.join(', ');
        await message.reply(i18n.t('bot:errors.alias_conflict', {
            alias: moduleAlias,
            modules,
            defaultValue: `❌ Alias '${moduleAlias}' is ambiguous — it matches multiple modules: ${modules}. Please use a more specific alias.`,
        }));
        return;
    }

    const moduleName = resolver.resolveModule(moduleAlias, guildId);
    const botConfig = resolver.getConfig();
    const prefix = botConfig.prefix;

    // 다중 인스턴스 경고
    let multiInstanceWarning = '';
    try {
        const servers = await getFilteredServers(guildId);
        const matched = servers.filter(s => s.module === moduleName);
        if (matched.length > 1) {
            multiInstanceWarning = '\n\n' + i18n.t('bot:errors.multiple_instances', {
                module: moduleName,
                defaultValue: `⚠️ Multiple '${moduleName}' instances exist. Use an instance name instead of the module alias.`,
            });
        }
    } catch (_) {}

    const cmds = resolver.getModuleCommands(moduleName, guildId);
    const cmdList = Object.keys(cmds);

    const moduleTitle = i18n.t('bot:help.module_title', { module: moduleName });
    const helpStart = i18n.t('bot:modules.help_start');
    const helpStop = i18n.t('bot:modules.help_stop');
    const helpStatus = i18n.t('bot:modules.help_status');
    const enterCommand = i18n.t('bot:modules.enter_command');

    // 케이스 2: 명령어를 입력해주세요 + 사용 가능한 명령어 목록
    let cmdHelp = `${enterCommand}\n\n${moduleTitle}\n`;
    cmdHelp += `• \`${prefix} ${moduleAlias} start\` - ${helpStart}\n`;
    cmdHelp += `• \`${prefix} ${moduleAlias} stop\` - ${helpStop}\n`;
    cmdHelp += `• \`${prefix} ${moduleAlias} status\` - ${helpStatus}\n`;

    if (cmdList.length > 0) {
        const restTitle = i18n.t('bot:modules.help_rest_title');
        cmdHelp += `\n${restTitle}\n`;
        for (const [cmdName, cmdMeta] of Object.entries(cmds)) {
            const inputsStr = cmdMeta.inputs && cmdMeta.inputs.length > 0
                ? cmdMeta.inputs.map(i => i.required ? `<${i.name}>` : `[${i.name}]`).join(' ')
                : '';
            cmdHelp += `• \`${prefix} ${moduleAlias} ${cmdName}${inputsStr ? ' ' + inputsStr : ''}\` - ${cmdMeta.label || cmdName}\n`;
        }
    }

    await message.reply(cmdHelp + multiInstanceWarning);
}

// ──────────────────────────────────────────
//  모듈 + 명령어 실행
// ──────────────────────────────────────────

async function handleModuleCommand(message, moduleAlias, commandAlias, extraArgs, guildId) {
    const botConfig = resolver.getConfig();
    const prefix = botConfig.prefix;

    // 별명 충돌 검사
    const conflict = resolver.checkModuleConflict(moduleAlias, guildId);
    if (conflict.isConflict) {
        const modules = conflict.conflictModules.join(', ');
        await message.reply(i18n.t('bot:errors.alias_conflict', {
            alias: moduleAlias,
            modules,
            defaultValue: `❌ Alias '${moduleAlias}' is ambiguous — it matches multiple modules: ${modules}. Please use a more specific alias.`,
        }));
        return;
    }

    const moduleName = resolver.resolveModule(moduleAlias, guildId);
    const commandName = resolver.resolveCommand(commandAlias, guildId);

    console.log(`[Processor] ${message.author.tag}: ${prefix} ${moduleAlias} ${commandAlias} → module=${moduleName}, command=${commandName}, args=${extraArgs.join(' ')}`);

    try {
        // 서버 찾기 (노드별 필터링 적용)
        const servers = await getFilteredServers(guildId);

        // 1) 인스턴스 이름으로 정확히 매칭
        let server = servers.find(s => s.name.toLowerCase() === moduleAlias.toLowerCase());

        if (!server) {
            // 2) 모듈명/별명으로 매칭
            const matched = servers.filter(s => s.module === moduleName || s.name.includes(moduleName));
            if (matched.length > 1) {
                await message.reply(i18n.t('bot:errors.multiple_instances', {
                    module: moduleName,
                    defaultValue: `⚠️ Multiple '${moduleName}' instances exist. Use an instance name instead of the module alias.`,
                }));
                return;
            }
            server = matched[0];
        }

        if (!server) {
            await message.reply(i18n.t('bot:server.not_found', { alias: moduleAlias, resolved: moduleName }));
            return;
        }

        // ── 모듈 존재 여부 확인 ──
        const modMeta = resolver.getModuleMeta(server.module, guildId);
        if (!modMeta) {
            await message.reply(i18n.t('bot:server.module_missing', {
                module: server.module,
                defaultValue: `❌ Module '${server.module}' is not installed. This server cannot be controlled.`,
            }));
            return;
        }

        // ── 멤버 권한 체크 (차단 목록 기반) ──
        const userId = message.author.id;
        if (resolver.isMemberManaged(guildId, userId)) {
            const deniedCmds = resolver.getMemberDeniedCommands(guildId, userId, server.id);
            if (deniedCmds !== null && deniedCmds.includes(commandName)) {
                console.log(`[Processor] Permission denied: user=${userId} server=${server.id} command=${commandName}`);
                await message.reply(i18n.t('bot:errors.permission_denied', {
                    defaultValue: '❌ 해당 명령어를 사용할 권한이 없습니다.',
                }));
                return;
            }
        }

        // ── 내장 명령어 (start, stop, status) ──
        if (commandName === 'start') {
            await executeStart(message, server, moduleName, guildId);
            return;
        }
        if (commandName === 'stop') {
            await executeStop(message, server);
            return;
        }
        if (commandName === 'status') {
            await executeStatus(message, server);
            return;
        }

        // ── module.toml 정의 명령어 ──
        const cmds = resolver.getModuleCommands(moduleName, guildId);
        const cmdMeta = cmds[commandName];

        if (!cmdMeta) {
            // 케이스 3: 존재하지 않는 명령어
            await message.reply(i18n.t('bot:errors.command_not_found'));
            return;
        }

        // rest / dual / rcon 명령어 실행
        await executeDefinedCommand(message, server, moduleName, commandName, cmdMeta, moduleAlias, commandAlias, extraArgs, guildId);

    } catch (error) {
        console.error('[Processor] Command error:', error.message);
        const moduleErrors = resolver.getModuleMeta(moduleName, guildId)?.errors || {};

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
            errorMsg = statusErrors[status] || (data?.error ? i18n.t('bot:errors.error_title') : i18n.t('bot:errors.error_title'));
        } else if (error.code) {
            const networkErrors = {
                'ECONNREFUSED': moduleErrors.connection_refused || i18n.t('bot:errors.connection_refused'),
                'ETIMEDOUT': moduleErrors.timeout || i18n.t('bot:errors.timeout'),
                'ENOTFOUND': i18n.t('bot:errors.host_not_found'),
            };
            errorMsg = networkErrors[error.code] || i18n.t('bot:errors.error_title');
        } else {
            errorMsg = i18n.t('bot:errors.error_title');
        }

        await message.reply(`❌ ${i18n.t('bot:errors.error_title')}: ${errorMsg}`).catch(replyErr => {
            console.error('[Processor] Failed to send error reply:', replyErr.message);
        });
    }
}

// ── 내장 명령어 구현 ──

function determineUseManaged(server, moduleName, guildId) {
    const modMeta = resolver.getModuleMeta(moduleName, guildId);
    const interactionMode = modMeta?.protocols?.interaction_mode
        || modMeta?.module?.interaction_mode;
    const instanceManagedStart = server.module_settings?.managed_start;
    if (instanceManagedStart === true || instanceManagedStart === 'true') return true;
    if (instanceManagedStart === false || instanceManagedStart === 'false') return false;
    return (interactionMode === 'console');
}

async function executeStart(message, server, moduleName, guildId) {
    const startMsg = i18n.t('bot:server.start_request', { name: server.name });
    const statusMsg = await message.reply(startMsg);
    try {
        const useManaged = determineUseManaged(server, moduleName, guildId);
        const result = await ipc.startServer(server.id, server.name, server.module, useManaged);
        const data = result?.data;

        // 데몬이 HTTP 200으로 응답했지만 실패/조치필요인 경우 처리
        if (data && data.success === false) {
            const errMsg = data.message || data.error || i18n.t('bot:errors.error_title');
            console.error(`[Processor] executeStart daemon error: ${errMsg}`);
            await statusMsg.edit(`❌ ${errMsg}`).catch(() => {});
            return;
        }
        if (data && data.action_required) {
            const detail = data.message || `Action required: ${data.action_required}`;
            console.warn(`[Processor] executeStart action_required: ${data.action_required}`);
            await statusMsg.edit(`⚠️ ${detail}`).catch(() => {});
            return;
        }

        const completeMsg = i18n.t('bot:server.start_complete', { name: server.name });
        await statusMsg.edit(completeMsg);
    } catch (e) {
        console.error('[Processor] executeStart error:', e.message);
        await statusMsg.edit(`❌ ${i18n.t('bot:errors.error_title')}`).catch(() => {});
    }
}

async function executeStop(message, server) {
    const stopMsg = i18n.t('bot:server.stop_request', { name: server.name });
    const statusMsg = await message.reply(stopMsg);
    try {
        await ipc.stopServer(server.name);
        const completeMsg = i18n.t('bot:server.stop_complete', { name: server.name });
        await statusMsg.edit(completeMsg);
    } catch (e) {
        console.error('[Processor] executeStop error:', e.message);
        await statusMsg.edit(`❌ ${i18n.t('bot:errors.error_title')}`).catch(() => {});
    }
}

async function executeStatus(message, server) {
    try {
        const statusText = server.status === 'running'
            ? i18n.t('bot:status.running')
            : i18n.t('bot:status.stopped');
        const pidText = server.pid ? `PID: ${server.pid}` : '';
        const checkMsg = i18n.t('bot:server.status_check', { name: server.name, status: statusText, pid_info: pidText });
        await message.reply(checkMsg);
    } catch (e) {
        console.error('[Processor] executeStatus error:', e.message);
    }
}

// ── Raw command (module.toml에 미정의) ──

async function executeRawCommand(message, server, moduleName, secondArg, extraArgs, guildId) {
    if (server.status !== 'running') {
        await message.reply(`❌ ${i18n.t('bot:server.not_running_default')}`);
        return;
    }

    const rawCommand = [secondArg, ...extraArgs].join(' ');
    console.log(`[Processor] Raw command forward: "${rawCommand}" → ${server.name}`);

    try {
        const useStdin = determineUseManaged(server, moduleName, guildId);
        let result;
        if (useStdin) {
            result = await ipc.sendStdin(server.id, rawCommand);
        } else {
            result = await ipc.sendRcon(server.id, rawCommand);
        }

        const response = result.data;
        if (response.error) {
            await message.reply(i18n.t('bot:messages.raw_command_error', { error: response.error }));
        } else {
            const output = ipc.formatResponse(response.data || response.response || response);
            await message.reply(i18n.t('bot:messages.raw_command_success', { output }));
        }
    } catch (error) {
        console.error('[Processor] Raw command error:', error.message);
        await message.reply(i18n.t('bot:messages.raw_command_error', { error: i18n.t('bot:errors.error_title') }));
    }
}

// ── 정의된 명령어 실행 (rest/rcon/dual) ──

async function executeDefinedCommand(message, server, moduleName, commandName, cmdMeta, moduleAlias, commandAlias, extraArgs, guildId) {
    const botConfig = resolver.getConfig();
    const prefix = botConfig.prefix;

    if (cmdMeta.method === 'rest' || cmdMeta.method === 'dual' || cmdMeta.method === 'rcon') {
        // 서버 실행 상태 확인
        if (server.status !== 'running') {
            const moduleErrors = resolver.getModuleMeta(moduleName, guildId)?.errors || {};
            const defaultMsg = i18n.t('bot:server.not_running_default');
            const errorMsg = moduleErrors.server_not_running || defaultMsg;
            await message.reply(i18n.t('bot:server.not_running', { name: server.name, error: errorMsg }));
            return;
        }

        // 입력값 빌드
        const body = {};
        if (cmdMeta.inputs && cmdMeta.inputs.length > 0) {
            for (let i = 0; i < cmdMeta.inputs.length; i++) {
                const input = cmdMeta.inputs[i];
                if (extraArgs[i]) {
                    body[input.name] = extraArgs[i];
                } else if (input.required) {
                    await message.reply(i18n.t('bot:command.missing_required', {
                        arg_name: input.name,
                        prefix,
                        alias: moduleAlias,
                        command: commandAlias,
                        description: input.label || input.name,
                    }));
                    return;
                }
            }
        }

        const executingMsg = i18n.t('bot:command.executing', { name: server.name, command: commandName });
        const statusMsg = await message.reply(executingMsg);

        let result;

        if (cmdMeta.method === 'rcon') {
            let rconCmd = cmdMeta.rcon_template || commandName;
            for (const [key, value] of Object.entries(body)) {
                rconCmd = rconCmd.replace(`{${key}}`, value);
            }
            rconCmd = rconCmd.replace(/\s*\{\w+\}/g, '').trim();
            console.log(`[Processor] RCON: ${rconCmd}`);
            result = await ipc.sendRcon(server.id, rconCmd);
        } else if (cmdMeta.method === 'dual') {
            console.log(`[Processor] Module command: ${commandName}`, body);
            result = await ipc.sendModuleCommand(server.id, commandName, body);
        } else {
            const endpoint = cmdMeta.endpoint_template || `/v1/api/${commandName}`;
            const httpMethod = (cmdMeta.http_method || 'GET').toUpperCase();
            console.log(`[Processor] REST ${httpMethod} ${endpoint}`, body);
            result = await ipc.sendRestCommand(server.id, endpoint, httpMethod, body, server);
        }

        if (result.data.success) {
            const responseText = ipc.formatResponse(result.data.data);
            const completeMsg = i18n.t('bot:command.execute_complete', { name: server.name, command: commandName, response: responseText });
            await statusMsg.edit(completeMsg);
        } else {
            const errorText = result.data.error || i18n.t('bot:errors.unknown');
            const errorCode = result.data.error_code || '';
            const moduleErrors = resolver.getModuleMeta(moduleName, guildId)?.errors || {};
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
}

module.exports = { process };
