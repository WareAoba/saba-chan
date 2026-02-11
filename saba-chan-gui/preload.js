const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('api', {
    serverList: () => ipcRenderer.invoke('server:list'),
    serverStart: (name, options = {}) => ipcRenderer.invoke('server:start', name, options),
    serverStop: (name, options = {}) => ipcRenderer.invoke('server:stop', name, options),
    serverStatus: (name) => ipcRenderer.invoke('server:status', name),
    moduleList: () => ipcRenderer.invoke('module:list'),
    moduleRefresh: () => ipcRenderer.invoke('module:refresh'),
    moduleGetMetadata: (name) => ipcRenderer.invoke('module:getMetadata', name),
    instanceCreate: (data) => ipcRenderer.invoke('instance:create', data),
    instanceDelete: (id) => ipcRenderer.invoke('instance:delete', id),
    instanceReorder: (orderedIds) => ipcRenderer.invoke('instance:reorder', orderedIds),
    instanceUpdateSettings: (id, settings) => ipcRenderer.invoke('instance:updateSettings', id, settings),
    executeCommand: (id, command) => ipcRenderer.invoke('instance:executeCommand', id, command),
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
    closeResponse: (choice) => ipcRenderer.send('app:closeResponse', choice),
    // Bot Relaunch (when language changes)
    onBotRelaunch: (callback) => ipcRenderer.on('bot:relaunch', (event, config) => callback(config)),
    // Status Update Events
    onStatusUpdate: (callback) => ipcRenderer.on('status:update', (event, data) => callback(data)),
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
