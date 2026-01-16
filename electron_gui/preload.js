const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('api', {
    serverList: () => ipcRenderer.invoke('server:list'),
    serverStart: (name, options = {}) => ipcRenderer.invoke('server:start', name, options),
    serverStop: (name, options = {}) => ipcRenderer.invoke('server:stop', name, options),
    serverStatus: (name) => ipcRenderer.invoke('server:status', name),
    moduleList: () => ipcRenderer.invoke('module:list'),
    moduleGetMetadata: (name) => ipcRenderer.invoke('module:getMetadata', name),
    instanceCreate: (data) => ipcRenderer.invoke('instance:create', data),
    instanceDelete: (id) => ipcRenderer.invoke('instance:delete', id),
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
    // App Lifecycle
    onCloseRequest: (callback) => ipcRenderer.on('app:closeRequest', callback),
    closeResponse: (choice) => ipcRenderer.send('app:closeResponse', choice)
});
