const { contextBridge, ipcRenderer } = require('electron');

contextBridge.exposeInMainWorld('api', {
    serverList: () => ipcRenderer.invoke('server:list'),
    serverStart: (name, options = {}) => ipcRenderer.invoke('server:start', name, options),
    serverStop: (name, options = {}) => ipcRenderer.invoke('server:stop', name, options),
    serverStatus: (name) => ipcRenderer.invoke('server:status', name),
    moduleList: () => ipcRenderer.invoke('module:list'),
    instanceCreate: (data) => ipcRenderer.invoke('instance:create', data),
    instanceDelete: (id) => ipcRenderer.invoke('instance:delete', id),
    // Settings API
    settingsLoad: () => ipcRenderer.invoke('settings:load'),
    settingsSave: (settings) => ipcRenderer.invoke('settings:save', settings),
    settingsGetPath: () => ipcRenderer.invoke('settings:getPath')
});
