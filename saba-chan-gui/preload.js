const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('api', {
    serverList: () => ipcRenderer.invoke('server:list'),
    serverStart: (name, options = {}) => ipcRenderer.invoke('server:start', name, options),
    serverStop: (name, options = {}) => ipcRenderer.invoke('server:stop', name, options),
    serverStatus: (name) => ipcRenderer.invoke('server:status', name),
    moduleList: () => ipcRenderer.invoke('module:list'),
    moduleRefresh: () => ipcRenderer.invoke('module:refresh'),
    moduleGetMetadata: (name) => ipcRenderer.invoke('module:getMetadata', name),
    moduleGetLocales: (name) => ipcRenderer.invoke('module:getLocales', name),
    moduleListVersions: (name, options) => ipcRenderer.invoke('module:listVersions', name, options),
    moduleInstallServer: (name, config) => ipcRenderer.invoke('module:installServer', name, config),
    instanceCreate: (data) => ipcRenderer.invoke('instance:create', data),
    instanceProvisionProgress: (name) => ipcRenderer.invoke('instance:provisionProgress', name),
    instanceDelete: (id) => ipcRenderer.invoke('instance:delete', id),
    instanceReorder: (orderedIds) => ipcRenderer.invoke('instance:reorder', orderedIds),
    instanceUpdateSettings: (id, settings) => ipcRenderer.invoke('instance:updateSettings', id, settings),
    instanceResetProperties: (id) => ipcRenderer.invoke('instance:resetProperties', id),
    instanceResetServer: (id) => ipcRenderer.invoke('instance:resetServer', id),
    executeCommand: (id, command) => ipcRenderer.invoke('instance:executeCommand', id, command),
    // Managed Process API (console capture)
    managedStart: (instanceId) => ipcRenderer.invoke('managed:start', instanceId),
    managedConsole: (instanceId, since, count) => ipcRenderer.invoke('managed:console', instanceId, since, count),
    managedStdin: (instanceId, command) => ipcRenderer.invoke('managed:stdin', instanceId, command),
    // Console Popout (PiP)
    consolePopout: (instanceId, serverName) => ipcRenderer.invoke('console:popout', instanceId, serverName),
    consoleFocusPopout: (instanceId) => ipcRenderer.invoke('console:focusPopout', instanceId),
    onConsolePopoutOpened: (callback) => ipcRenderer.on('console:popoutOpened', (event, instanceId) => callback(instanceId)),
    offConsolePopoutOpened: () => ipcRenderer.removeAllListeners('console:popoutOpened'),
    onConsolePopoutClosed: (callback) => ipcRenderer.on('console:popoutClosed', (event, instanceId) => callback(instanceId)),
    offConsolePopoutClosed: () => ipcRenderer.removeAllListeners('console:popoutClosed'),
    // Updater — 데몬 HTTP API를 통한 업데이트 관리
    updaterCheck: () => ipcRenderer.invoke('updater:check'),
    updaterStatus: () => ipcRenderer.invoke('updater:status'),
    updaterDownload: (components) => ipcRenderer.invoke('updater:download', components),
    updaterApply: (components) => ipcRenderer.invoke('updater:apply', components),
    updaterLaunchApply: (targets) => ipcRenderer.invoke('updater:launchApply', targets),
    updaterGetConfig: () => ipcRenderer.invoke('updater:getConfig'),
    updaterSetConfig: (config) => ipcRenderer.invoke('updater:setConfig', config),
    // Mock Server (테스트용)
    mockServerStart: (options) => ipcRenderer.invoke('mockServer:start', options),
    mockServerStop: () => ipcRenderer.invoke('mockServer:stop'),
    mockServerStatus: () => ipcRenderer.invoke('mockServer:status'),
    // Updater events (from main process background checker)
    onUpdatesAvailable: (callback) => ipcRenderer.on('updates:available', (event, data) => callback(data)),
    offUpdatesAvailable: () => ipcRenderer.removeAllListeners('updates:available'),
    onUpdateCompleted: (callback) => ipcRenderer.on('updates:completed', (event, data) => callback(data)),
    offUpdateCompleted: () => ipcRenderer.removeAllListeners('updates:completed'),
    // Settings API
    settingsLoad: () => ipcRenderer.invoke('settings:load'),
    settingsSave: (settings) => ipcRenderer.invoke('settings:save', settings),
    settingsGetPath: () => ipcRenderer.invoke('settings:getPath'),
    // Dialog API
    openFileDialog: (options) => ipcRenderer.invoke('dialog:openFile', options),
    openFolderDialog: () => ipcRenderer.invoke('dialog:openFolder'),
    // Discord Bot API
    discordBotStatus: () => ipcRenderer.invoke('discord:status'),
    discordBotStart: (config) => ipcRenderer.invoke('discord:start', config),
    discordBotStop: () => ipcRenderer.invoke('discord:stop'),
    // Bot Config API
    botConfigLoad: () => ipcRenderer.invoke('botConfig:load'),
    botConfigSave: (config) => ipcRenderer.invoke('botConfig:save', config),
    // Logs API
    logsGetPath: () => ipcRenderer.invoke('logs:getPath'),
    logsOpenFolder: () => ipcRenderer.invoke('logs:openFolder'),
    // App Lifecycle
    onCloseRequest: (callback) => ipcRenderer.on('app:closeRequest', callback),
    offCloseRequest: () => ipcRenderer.removeAllListeners('app:closeRequest'),
    closeResponse: (choice) => ipcRenderer.send('app:closeResponse', choice),
    // Bot Relaunch (when language changes)
    onBotRelaunch: (callback) => ipcRenderer.on('bot:relaunch', (event, config) => callback(config)),
    offBotRelaunch: () => ipcRenderer.removeAllListeners('bot:relaunch'),
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
    getSystemLanguage: () => ipcRenderer.invoke('language:getSystem')
});
