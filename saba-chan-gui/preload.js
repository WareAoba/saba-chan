const { contextBridge, ipcRenderer } = require('electron');

const invoke = (channel) => (...args) => ipcRenderer.invoke(channel, ...args);

// main → renderer 이벤트 구독. 반환값을 호출하면 그 리스너만 해제한다
// (removeAllListeners로 다른 구독자까지 지우지 않도록). IPC event 객체는 렌더러에 넘기지 않는다.
const subscribe = (channel) => (callback) => {
    const handler = (_event, ...args) => callback(...args);
    ipcRenderer.on(channel, handler);
    return () => ipcRenderer.removeListener(channel, handler);
};

contextBridge.exposeInMainWorld('api', {
    // Servers
    serverList: invoke('server:list'),
    serverStart: (name, options = {}) => ipcRenderer.invoke('server:start', name, options),
    serverStop: (name, options = {}) => ipcRenderer.invoke('server:stop', name, options),
    serverStatus: invoke('server:status'),
    // Modules
    moduleList: invoke('module:list'),
    moduleRefresh: invoke('module:refresh'),
    moduleGetMetadata: invoke('module:getMetadata'),
    moduleGetLocales: invoke('module:getLocales'),
    moduleListVersions: invoke('module:listVersions'),
    moduleInstallServer: invoke('module:installServer'),
    moduleManifest: invoke('module:manifest'),
    moduleInstallFromManifest: invoke('module:installFromManifest'),
    moduleRemove: invoke('module:remove'),
    // Instances
    instanceCreate: invoke('instance:create'),
    instanceProvisionProgress: invoke('instance:provisionProgress'),
    instanceDismissProvision: invoke('instance:dismissProvision'),
    instanceCheckUpdate: invoke('instance:checkUpdate'),
    instanceApplyUpdate: invoke('instance:applyUpdate'),
    instanceDelete: invoke('instance:delete'),
    instanceReorder: invoke('instance:reorder'),
    instanceUpdateSettings: invoke('instance:updateSettings'),
    instanceReadLiveSettings: invoke('instance:readLiveSettings'),
    instanceResetServer: invoke('instance:resetServer'),
    instanceGetInstalledVersion: invoke('instance:getInstalledVersion'),
    instanceOpenFolder: invoke('instance:openFolder'),
    executeCommand: invoke('instance:executeCommand'),
    // Extensions
    extensionList: invoke('extension:list'),
    extensionEnable: invoke('extension:enable'),
    extensionDisable: invoke('extension:disable'),
    extensionI18n: invoke('extension:i18n'),
    extensionGuiBundle: invoke('extension:guiBundle'),
    extensionGuiStyles: invoke('extension:guiStyles'),
    extensionIcon: invoke('extension:icon'),
    extensionFetchManifest: invoke('extension:fetchManifest'),
    extensionInstall: invoke('extension:install'),
    extensionRemove: invoke('extension:remove'),
    extensionCheckUpdates: invoke('extension:checkUpdates'),
    extensionRescan: invoke('extension:rescan'),
    extensionInitStatus: invoke('extension:initStatus'),
    // Managed Process API (console capture)
    managedStart: (instanceId, options = {}) => ipcRenderer.invoke('managed:start', instanceId, options),
    managedConsole: invoke('managed:console'),
    managedStdin: invoke('managed:stdin'),
    // Console Popout (PiP)
    consolePopout: invoke('console:popout'),
    consoleFocusPopout: invoke('console:focusPopout'),
    consolePopin: invoke('console:popin'),
    consoleToggleAlwaysOnTop: invoke('console:toggleAlwaysOnTop'),
    onConsolePopoutOpened: subscribe('console:popoutOpened'),
    onConsolePopoutClosed: subscribe('console:popoutClosed'),
    onConsolePopinRequest: subscribe('console:popinRequest'),
    // Updater — 데몬 HTTP API를 통한 업데이트 관리
    updaterCheck: invoke('updater:check'),
    updaterStatus: invoke('updater:status'),
    updaterDownload: invoke('updater:download'),
    updaterDownloadProgress: invoke('updater:downloadProgress'),
    updaterApply: invoke('updater:apply'),
    updaterLaunchApply: invoke('updater:launchApply'),
    updaterGetConfig: invoke('updater:getConfig'),
    updaterSetConfig: invoke('updater:setConfig'),
    onUpdatesAvailable: subscribe('updates:available'),
    onUpdateCompleted: subscribe('updates:completed'),
    // Settings
    settingsLoad: invoke('settings:load'),
    settingsSave: invoke('settings:save'),
    // Dialogs
    openFileDialog: invoke('dialog:openFile'),
    openFolderDialog: invoke('dialog:openFolder'),
    migrationScanDir: invoke('migration:scanDir'),
    // Bot Config (릴레이 봇 설정)
    botConfigLoad: invoke('botConfig:load'),
    botConfigSave: invoke('botConfig:save'),
    // Node Token (cloud pairing)
    saveNodeToken: invoke('nodeToken:save'),
    deleteNodeToken: invoke('nodeToken:delete'),
    // Relay server proxy (cloud mode) — all relay comms go through daemon
    relayCheckHostStatus: invoke('relay:checkHostStatus'),
    relayListHostNodes: invoke('relay:listHostNodes'),
    relayListNodeMembers: invoke('relay:listNodeMembers'),
    relayGetNodeSettings: invoke('relay:getNodeSettings'),
    relayUpdateNodeSettings: invoke('relay:updateNodeSettings'),
    relayInitiatePairing: invoke('relay:initiatePairing'),
    relayPollPairingStatus: invoke('relay:pollPairingStatus'),
    // App
    launchUninstaller: invoke('app:launchUninstaller'),
    getAppVersion: invoke('app:getVersion'),
    getComponentInfo: invoke('app:getComponentInfo'),
    onCloseRequest: subscribe('app:closeRequest'),
    closeResponse: (choice) => ipcRenderer.send('app:closeResponse', choice),
    onStatusUpdate: subscribe('status:update'),
    // Daemon
    daemonStatus: invoke('daemon:status'),
    daemonRestart: invoke('daemon:restart'),
});

contextBridge.exposeInMainWorld('electron', {
    // Window Controls (Title Bar)
    minimizeWindow: () => ipcRenderer.send('window:minimize'),
    maximizeWindow: () => ipcRenderer.send('window:maximize'),
    closeWindow: () => ipcRenderer.send('window:close'),
    // Language
    getLanguage: invoke('language:get'),
    setLanguage: invoke('language:set'),
    // Shell
    shell: {
        openExternal: invoke('shell:openExternal'),
    },
});
