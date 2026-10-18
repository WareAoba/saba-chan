const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('api', {
    serverList: () => ipcRenderer.invoke('server:list'),
    instanceStart: (instanceId) => ipcRenderer.invoke('instance:start', instanceId),
    serverStop: (name, options = {}) => ipcRenderer.invoke('server:stop', name, options),
    serverStatus: (name) => ipcRenderer.invoke('server:status', name),
    moduleList: () => ipcRenderer.invoke('module:list'),
    moduleRefresh: () => ipcRenderer.invoke('module:refresh'),
    moduleGetMetadata: (name) => ipcRenderer.invoke('module:getMetadata', name),
    moduleAudit: (name) => ipcRenderer.invoke('module:audit', name),
    instanceEvents: (id) => ipcRenderer.invoke('instance:events', id),
    moduleGetLocales: (name) => ipcRenderer.invoke('module:getLocales', name),
    moduleListVersions: (name, options) => ipcRenderer.invoke('module:listVersions', name, options),
    moduleInstallServer: (name, config) => ipcRenderer.invoke('module:installServer', name, config),
    instanceGetInstalledVersion: (instanceId) => ipcRenderer.invoke('instance:getInstalledVersion', instanceId),
    // Module Manifest (사바 스토리지 — 모듈 탭)
    moduleManifest: () => ipcRenderer.invoke('module:manifest'),
    moduleInstallFromManifest: (moduleId) => ipcRenderer.invoke('module:installFromManifest', moduleId),
    moduleRemove: (moduleId) => ipcRenderer.invoke('module:remove', moduleId),
    instanceCreate: (data) => ipcRenderer.invoke('instance:create', data),
    instanceProvisionProgress: (name) => ipcRenderer.invoke('instance:provisionProgress', name),
    instanceDismissProvision: (name) => ipcRenderer.invoke('instance:dismissProvision', name),
    instanceCheckUpdate: (id) => ipcRenderer.invoke('instance:checkUpdate', id),
    instanceApplyUpdate: (id) => ipcRenderer.invoke('instance:applyUpdate', id),
    instanceDelete: (id) => ipcRenderer.invoke('instance:delete', id),
    instanceReorder: (orderedIds) => ipcRenderer.invoke('instance:reorder', orderedIds),
    instanceUpdateSettings: (id, settings) => ipcRenderer.invoke('instance:updateSettings', id, settings),
    // 실행 인자 미리보기: (id, 설정 초안 {필드: 값}, 추가 인자 문자열|배열?) →
    // {success, has_launch_args, command_line, program, args, settings_from_args, extra_args, extra_args_text, errors}
    instanceLaunchPreview: (id, settings, extraArgs) => ipcRenderer.invoke('instance:launchPreview', id, settings, extraArgs),
    instanceResetServer: (id) => ipcRenderer.invoke('instance:resetServer', id),
    executeCommand: (id, command) => ipcRenderer.invoke('instance:executeCommand', id, command),
    instanceStop: (id, opts) => ipcRenderer.invoke('instance:stop', id, opts),
    instanceRestart: (id, opts) => ipcRenderer.invoke('instance:restart', id, opts),
    instanceRename: (id, name) => ipcRenderer.invoke('instance:rename', id, name),
    instanceCommands: (id) => ipcRenderer.invoke('instance:commands', id),
    instanceReadLiveSettings: (id) => ipcRenderer.invoke('instance:readLiveSettings', id),
    runtimePreflight: () => ipcRenderer.invoke('runtime:preflight'),
    systemMetrics: (opts) => ipcRenderer.invoke('system:metrics', opts),
    configDefaults: (kind) => ipcRenderer.invoke('config:defaults', kind),
    configReset: (kind, keys) => ipcRenderer.invoke('config:reset', kind, keys),
    // Extension API
    extensionList: () => ipcRenderer.invoke('extension:list'),
    extensionEnable: (extId) => ipcRenderer.invoke('extension:enable', extId),
    extensionDisable: (extId) => ipcRenderer.invoke('extension:disable', extId),
    extensionAudit: (extId) => ipcRenderer.invoke('extension:audit', extId),
    extensionI18n: (extId, locale) => ipcRenderer.invoke('extension:i18n', extId, locale),
    extensionGuiBundle: (extId) => ipcRenderer.invoke('extension:guiBundle', extId),
    extensionGuiStyles: (extId) => ipcRenderer.invoke('extension:guiStyles', extId),
    extensionIcon: (extId) => ipcRenderer.invoke('extension:icon', extId),
    // Extension Manifest & Version Management API
    extensionFetchManifest: () => ipcRenderer.invoke('extension:fetchManifest'),
    extensionInstall: (extId, opts) => ipcRenderer.invoke('extension:install', extId, opts),
    extensionRemove: (extId) => ipcRenderer.invoke('extension:remove', extId),
    extensionCheckUpdates: () => ipcRenderer.invoke('extension:checkUpdates'),
    extensionRescan: () => ipcRenderer.invoke('extension:rescan'),
    extensionInitStatus: () => ipcRenderer.invoke('extension:initStatus'),
    // Managed Process API (console capture)
    managedConsole: (instanceId, since, count, opts) => ipcRenderer.invoke('managed:console', instanceId, since, count, opts),
    managedStdin: (instanceId, command) => ipcRenderer.invoke('managed:stdin', instanceId, command),
    // Console Popout (PiP)
    consolePopout: (instanceId, serverName) => ipcRenderer.invoke('console:popout', instanceId, serverName),
    // 지금 열린 콘솔 팝아웃의 instanceId 목록 (메인 창을 다시 불러온 뒤 상태를 맞출 때)
    consolePopoutList: () => ipcRenderer.invoke('console:popoutList'),
    // 데몬 자신의 로그 버퍼: {since?, count?} → {lines: [{id, timestamp, level, target, message}], count}
    daemonConsole: (opts) => ipcRenderer.invoke('daemon:console', opts),
    consoleFocusPopout: (instanceId) => ipcRenderer.invoke('console:focusPopout', instanceId),
    consolePopin: (instanceId) => ipcRenderer.invoke('console:popin', instanceId),
    consoleToggleAlwaysOnTop: (instanceId, pinned) => ipcRenderer.invoke('console:toggleAlwaysOnTop', instanceId, pinned),
    onConsolePopoutOpened: (callback) => ipcRenderer.on('console:popoutOpened', (event, instanceId) => callback(instanceId)),
    offConsolePopoutOpened: () => ipcRenderer.removeAllListeners('console:popoutOpened'),
    onConsolePopoutClosed: (callback) => ipcRenderer.on('console:popoutClosed', (event, instanceId) => callback(instanceId)),
    offConsolePopoutClosed: () => ipcRenderer.removeAllListeners('console:popoutClosed'),
    onConsolePopinRequest: (callback) => ipcRenderer.on('console:popinRequest', (event, instanceId, serverName) => callback(instanceId, serverName)),
    offConsolePopinRequest: () => ipcRenderer.removeAllListeners('console:popinRequest'),
    // Updater — 데몬 HTTP API를 통한 업데이트 관리
    updaterCheck: () => ipcRenderer.invoke('updater:check'),
    updaterStatus: () => ipcRenderer.invoke('updater:status'),
    updaterDownload: (components) => ipcRenderer.invoke('updater:download', components),
    updaterDownloadProgress: () => ipcRenderer.invoke('updater:downloadProgress'),
    updaterApply: (components) => ipcRenderer.invoke('updater:apply', components),
    updaterLaunchApply: (targets) => ipcRenderer.invoke('updater:launchApply', targets),
    updaterLastResult: (opts) => ipcRenderer.invoke('updater:lastResult', opts),
    updaterGetConfig: () => ipcRenderer.invoke('updater:getConfig'),
    updaterSetConfig: (config) => ipcRenderer.invoke('updater:setConfig', config),
    // Updater events (from main process background checker)
    onUpdatesAvailable: (callback) => ipcRenderer.on('updates:available', (event, data) => callback(data)),
    onUpdateCompleted: (callback) => ipcRenderer.on('updates:completed', (event, data) => callback(data)),
    // Settings API
    settingsLoad: () => ipcRenderer.invoke('settings:load'),
    settingsSave: (settings) => ipcRenderer.invoke('settings:save', settings),
    settingsGetPath: () => ipcRenderer.invoke('settings:getPath'),
    // Fixed Paths API
    // Dialog API
    openFileDialog: (options) => ipcRenderer.invoke('dialog:openFile', options),
    openFolderDialog: () => ipcRenderer.invoke('dialog:openFolder'),
    // Migration API
    migrationScanDir: (dirPath) => ipcRenderer.invoke('migration:scanDir', dirPath),
    // Discord 릴레이 에이전트 API (데몬 내장)
    discordBotStatus: () => ipcRenderer.invoke('discord:status'),
    discordBotStart: () => ipcRenderer.invoke('discord:start'),
    discordBotStop: () => ipcRenderer.invoke('discord:stop'),
    // Bot Config API
    botConfigLoad: () => ipcRenderer.invoke('botConfig:load'),
    botConfigSave: (config) => ipcRenderer.invoke('botConfig:save', config),
    // GUI Config sync to daemon
    // Node Token API (cloud pairing)
    saveNodeToken: (token) => ipcRenderer.invoke('nodeToken:save', token),
    loadNodeToken: () => ipcRenderer.invoke('nodeToken:load'),
    deleteNodeToken: () => ipcRenderer.invoke('nodeToken:delete'),
    // 감사 로그 (A10): {limit?, instance?} → {entries: [{ts, source, user?, userName?, guild?, guildName?, action, instanceId, instance?, command?, args?, ok, status}]}
    // 비밀 인자·설정값은 "***"로 가려져 있다
    auditList: (opts) => ipcRenderer.invoke('audit:list', opts),
    // 디스코드 명령 권한: {guildId, ownerDiscordId, relayOwnerDiscordId, defaultLevel, preset, chatbotCap, members, commandLevels, presets}
    permissionsGet: (guildId) => ipcRenderer.invoke('permissions:get', guildId),
    permissionsUpdate: (guildId, patch) => ipcRenderer.invoke('permissions:update', guildId, patch),
    permissionsSetMember: (guildId, discordId, level) => ipcRenderer.invoke('permissions:setMember', guildId, discordId, level),
    permissionsRemoveMember: (guildId, discordId) => ipcRenderer.invoke('permissions:removeMember', guildId, discordId),
    // 격리를 준비할 수 없을 때 그 구성요소를 격리 없이 돌리기 (파일이 바뀌면 다시 묻는다): {success, component, fingerprint}
    // 동의 목록: {components: [{component, fingerprint, current}]} (current=false면 파일이 바뀌어 다시 묻는다)
    sandboxListUnisolated: () => ipcRenderer.invoke('sandbox:listUnisolated'),
    sandboxAllowUnisolated: (component) => ipcRenderer.invoke('sandbox:allowUnisolated', component),
    sandboxRevokeUnisolated: (component) => ipcRenderer.invoke('sandbox:revokeUnisolated', component),
    // 액션 카탈로그 (P2): {actions:[{id, kind, risk, surfaces, scope, input, title, description?}], locale}
    actionsList: (opts) => ipcRenderer.invoke('actions:list', opts),
    // {input, instance} → {planId, summary:[{labelKey, label, value}], risk, needsConfirm, expiresAt}
    actionsPlan: (id, body) => ipcRenderer.invoke('actions:plan', id, body),
    // {input, instance} 또는 {planId} → {success, actionId, status, result}
    actionsRun: (id, body) => ipcRenderer.invoke('actions:run', id, body),
    // 디스코드 길드 설정 (I2): {guildId, locale, sleeping, music:{enabled, ui, queueLines, refreshInterval, normalize}}
    discordSettingsGet: (guild) => ipcRenderer.invoke('discord:settings:get', guild),
    // 바꿀 키만 → {success, settings, sync:{guildId, ok, status?, error?}}
    discordSettingsSet: (guild, patch) => ipcRenderer.invoke('discord:settings:set', guild, patch),
    // 터미널로 열기 (I4): {view: 'console'|'shell', instance?} → {success, pid} 또는 {error, error_code}
    // (unsupported_platform·cli_not_found·not_found·invalid_input)
    openInTerminal: (opts) => ipcRenderer.invoke('clients:launch', opts),
    // 데몬 이벤트 스트림 (P4): cb({id, type, data}). type 'resync'면 목록을 다시 읽는다. 돌려주는 함수로 해제
    onDaemonEvent: (callback) => {
        const listener = (event, data) => callback(data);
        ipcRenderer.on('events:message', listener);
        return () => ipcRenderer.removeListener('events:message', listener);
    },
    // 설정·서버 목록 내보내기/가져오기 (A9): {secrets} / {path?, dryRun, overwrite}. 취소하면 {canceled: true}
    dataExport: (opts) => ipcRenderer.invoke('dataTransfer:export', opts),
    dataImport: (opts) => ipcRenderer.invoke('dataTransfer:import', opts),
    // Relay server proxy (cloud mode) — all relay comms go through daemon
    relayCheckHostStatus: (hostId, relayUrl) => ipcRenderer.invoke('relay:checkHostStatus', hostId, relayUrl),
    relayListHostNodes: (hostId, relayUrl) => ipcRenderer.invoke('relay:listHostNodes', hostId, relayUrl),
    relayListNodeMembers: (guildId, relayUrl) => ipcRenderer.invoke('relay:listNodeMembers', guildId, relayUrl),
    relayBotInvite: (relayUrl) => ipcRenderer.invoke('relay:botInvite', relayUrl),
    // 채널 배정: {guildId, chatbot: [id], music: id|null, botAvailable, channels: [{id, name, category}]}. patch = {chatbot?, music?}
    relayNodeChannels: (guildId) => ipcRenderer.invoke('relay:nodeChannels', guildId),
    relayNodeChannelsSave: (guildId, patch) => ipcRenderer.invoke('relay:nodeChannelsSave', guildId, patch),
    instanceAgreements: (id, accept) => ipcRenderer.invoke('instance:agreements', id, accept),
    instanceBackups: (id) => ipcRenderer.invoke('instance:backups', id),
    instanceBackupCreate: (id) => ipcRenderer.invoke('instance:backupCreate', id),
    instanceBackupDelete: (id, name) => ipcRenderer.invoke('instance:backupDelete', id, name),
    instanceBackupRestore: (id, name) => ipcRenderer.invoke('instance:backupRestore', id, name),
    instanceScheduleGet: (id) => ipcRenderer.invoke('instance:scheduleGet', id),
    instanceScheduleSave: (id, body) => ipcRenderer.invoke('instance:scheduleSave', id, body),
    chatbotGet: () => ipcRenderer.invoke('chatbot:get'),
    chatbotSave: (patch) => ipcRenderer.invoke('chatbot:save', patch),
    chatbotClear: () => ipcRenderer.invoke('chatbot:clear'),
    chatbotClearMemory: (guildId) => ipcRenderer.invoke('chatbot:clearMemory', guildId),
    relayInitiatePairing: (payload) => ipcRenderer.invoke('relay:initiatePairing', payload),
    relayPollPairingStatus: (code, secret, relayUrl) => ipcRenderer.invoke('relay:pollPairingStatus', code, secret, relayUrl),
    relayLinkRequests: () => ipcRenderer.invoke('relay:linkRequests'),
    relayLinkRequestRespond: (requestId, accept) => ipcRenderer.invoke('relay:linkRequestRespond', requestId, accept),
    // Shell API
    shellOpenPath: (folderPath) => ipcRenderer.invoke('shell:openPath', folderPath),
    // Logs API
    logsGetPath: () => ipcRenderer.invoke('logs:getPath'),
    logsOpenFolder: () => ipcRenderer.invoke('logs:openFolder'),
    // Uninstall
    launchUninstaller: () => ipcRenderer.invoke('app:launchUninstaller'),
    getAppVersion: () => ipcRenderer.invoke('app:getVersion'),
    getComponentInfo: () => ipcRenderer.invoke('app:getComponentInfo'),
    // App Lifecycle
    onCloseRequest: (callback) => ipcRenderer.on('app:closeRequest', callback),
    offCloseRequest: () => ipcRenderer.removeAllListeners('app:closeRequest'),
    closeResponse: (choice) => ipcRenderer.send('app:closeResponse', choice),
    // Status Update Events
    onStatusUpdate: (callback) => ipcRenderer.on('status:update', (event, data) => callback(data)),
    offStatusUpdate: () => ipcRenderer.removeAllListeners('status:update'),
    // Daemon Status
    daemonStatus: () => ipcRenderer.invoke('daemon:status'),
    daemonRestart: () => ipcRenderer.invoke('daemon:restart'),
    // Window Controls (Title Bar)
    minimizeWindow: () => ipcRenderer.send('window:minimize'),
    maximizeWindow: () => ipcRenderer.send('window:maximize'),
    closeWindow: () => ipcRenderer.send('window:close')
});

// window.electron 객체로도 노출
contextBridge.exposeInMainWorld('electron', {
    minimizeWindow: () => ipcRenderer.send('window:minimize'),
    maximizeWindow: () => ipcRenderer.send('window:maximize'),
    closeWindow: () => ipcRenderer.send('window:close'),
    // Language settings
    getLanguage: () => ipcRenderer.invoke('language:get'),
    setLanguage: (language) => ipcRenderer.invoke('language:set', language),
    // Shell
    shell: {
        openExternal: (url) => ipcRenderer.invoke('shell:openExternal', url)
    }
});
