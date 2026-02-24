import React, { useState, useEffect, useRef, useMemo, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import './App.css';
import { 
    SuccessModal, 
    FailureModal, 
    NotificationModal, 
    QuestionModal,
    CommandModal,
    Toast,
    TitleBar,
    SettingsModal,
    DiscordBotModal,
    BackgroundModal,
    AddServerModal,
    NoticeModal,
    Icon,
    CustomDropdown,
    ServerCard,
    ServerSettingsModal,
    LoadingScreen,
    ConsolePanel,
    PopoutConsole
} from './components';
import { useModalClose } from './hooks/useModalClose';
import { useWaitingImage } from './hooks/useWaitingImage';
import { useConsole } from './hooks/useConsole';
import { useDragReorder } from './hooks/useDragReorder';
import { useDiscordBot } from './hooks/useDiscordBot';
import { useServerActions } from './hooks/useServerActions';
import { useServerSettings } from './hooks/useServerSettings';
import { safeShowToast, createTranslateError, retryWithBackoff, waitForDaemon, debugLog, debugWarn } from './utils/helpers';
import { ExtensionProvider } from './contexts/ExtensionContext';

function App() {
    const { t, i18n } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // ── Console Popout Mode Detection ──────────────────────
    const popoutParams = useMemo(() => {
        const params = new URLSearchParams(window.location.search);
        const instanceId = params.get('console-popout');
        const name = params.get('name');
        if (instanceId && name) return { instanceId, name };
        return null;
    }, []);
    const isPopoutMode = !!popoutParams;

    // 언어별 로고 이미지 선택
    const logoSrc = useMemo(() => {
        const lang = (i18n.language || 'en').toLowerCase();
        if (lang.startsWith('ko')) return './logo-kr.png';
        if (lang.startsWith('ja')) return './logo-jp.png';
        return './logo-en.png';
    }, [i18n.language]);

    // ── Core Shared State ──────────────────────────────────
    const [servers, setServers] = useState([]);
    const [modules, setModules] = useState([]);
    const [loading, setLoading] = useState(true);
    const [modal, setModal] = useState(null);
    const [progressBar, setProgressBar] = useState(null);

    // ── Init State ─────────────────────────────────────────
    const [daemonReady, setDaemonReady] = useState(false);
    const [initStatus, setInitStatus] = useState('Initialize...');
    const [initProgress, setInitProgress] = useState(0);
    const [serversInitializing, setServersInitializing] = useState(true);

    // ── Uptime Clock ───────────────────────────────────────
    const [nowEpoch, setNowEpoch] = useState(() => Math.floor(Date.now() / 1000));
    useEffect(() => {
        const timer = setInterval(() => setNowEpoch(Math.floor(Date.now() / 1000)), 1000);
        return () => clearInterval(timer);
    }, []);

    const formatUptime = (startTime) => {
        if (!startTime) return null;
        const elapsed = Math.max(0, nowEpoch - startTime);
        const h = String(Math.floor(elapsed / 3600)).padStart(2, '0');
        const m = String(Math.floor((elapsed % 3600) / 60)).padStart(2, '0');
        const s = String(elapsed % 60).padStart(2, '0');
        return `${h}:${m}:${s}`;
    };

    // ── App Settings State ─────────────────────────────────
    const [autoRefresh, setAutoRefresh] = useState(true);
    const [refreshInterval, setRefreshInterval] = useState(2000);
    const [ipcPort, setIpcPort] = useState(57474);
    const [consoleBufferSize, setConsoleBufferSize] = useState(2000);
    const consoleBufferRef = useRef(2000);
    const [modulesPath, setModulesPath] = useState('');
    const [settingsPath, setSettingsPath] = useState('');

    // ── Module Manager State ───────────────────────────────
    const [showModuleManager, setShowModuleManager] = useState(false);
    const [settingsInitialView, setSettingsInitialView] = useState(null);
    const [showCommandModal, setShowCommandModal] = useState(false);
    const [commandServer, setCommandServer] = useState(null);
    const [showGuiSettingsModal, setShowGuiSettingsModal] = useState(false);

    // ── Context Menu State ─────────────────────────────────
    const [contextMenu, setContextMenu] = useState(null);

    // ── Discord Config State ───────────────────────────────
    const [discordToken, setDiscordToken] = useState('');
    const [showDiscordSection, setShowDiscordSection] = useState(false);
    const [showBackgroundSection, setShowBackgroundSection] = useState(false);
    const [showNoticeSection, setShowNoticeSection] = useState(false);
    const [unreadNoticeCount, setUnreadNoticeCount] = useState(0);
    const [discordPrefix, setDiscordPrefix] = useState('!saba');
    const [discordAutoStart, setDiscordAutoStart] = useState(false);
    const [discordModuleAliases, setDiscordModuleAliases] = useState({});
    const [discordCommandAliases, setDiscordCommandAliases] = useState({});
    const [discordMusicEnabled, setDiscordMusicEnabled] = useState(true);
    const discordTokenRef = useRef('');

    // ── Discord Cloud Mode State ───────────────────────────
    const [discordBotMode, setDiscordBotMode] = useState('local');       // 'local' | 'cloud'
    const [discordCloudRelayUrl, setDiscordCloudRelayUrl] = useState('');
    const [discordCloudHostId, setDiscordCloudHostId] = useState('');

    // ── Per-node settings (client-side) ──────────────────────
    // { [guildId|"local"]: { allowedInstances: string[], memberPermissions: { [userId]: { [serverId]: string[] } } } }
    const [nodeSettings, setNodeSettings] = useState({});

    // ── Cloud cache (로컬 저장 — 서버 페치 없이 즉시 표시) ──
    // cloudNodes: [{ guildId, guildName, hostId, ... }]
    const [cloudNodes, setCloudNodes] = useState([]);
    // cloudMembers: { [guildId]: [{ id, username, displayName }] }
    const [cloudMembers, setCloudMembers] = useState({});

    // ── Background Daemon State ────────────────────────────
    const [backgroundDaemonStatus, setBackgroundDaemonStatus] = useState('checking');

    // ── Init Flags ─────────────────────────────────────────
    const [settingsReady, setSettingsReady] = useState(false);

    // ── Module Aliases from module.toml ────────────────────
    const [moduleAliasesPerModule, setModuleAliasesPerModule] = useState({});

    // ══════════════════════════════════════════════════════════
    // ── Custom Hooks ─────────────────────────────────────────
    // ══════════════════════════════════════════════════════════

    const { showWaitingImage, setShowWaitingImage } = useWaitingImage(progressBar);

    const {
        consoleServer, consoleLines, consoleInput, setConsoleInput,
        consoleEndRef, consolePopoutInstanceId, setConsolePopoutInstanceId,
        openConsole, closeConsole, sendConsoleCommand,
    } = useConsole({ isPopoutMode, popoutParams, consoleBufferRef });

    const { draggedName, cardRefs, skipNextClick, handleCardPointerDown } = useDragReorder(servers, setServers);

    const {
        discordBotStatus, setDiscordBotStatus, botStatusReady,
        relayConnected, relayConnecting,
        handleStartDiscordBot, handleStopDiscordBot,
    } = useDiscordBot({
        discordToken, discordPrefix, discordAutoStart,
        discordModuleAliases, discordCommandAliases,
        discordBotMode, discordCloudRelayUrl, discordCloudHostId,
        nodeSettings,
        settingsReady, discordTokenRef,
        setModal,
    });

    const {
        fetchServers, handleStart, handleStop, handleStatus,
        handleAddServer, handleDeleteServer,
    } = useServerActions({
        servers, setServers, modules, loading, setLoading,
        setModal, setProgressBar,
        consoleServer, openConsole, closeConsole,
        setShowModuleManager,
        formatUptime,
        openSettingsToExtensions: () => {
            setSettingsInitialView('extensions');
            setShowGuiSettingsModal(true);
        },
    });

    const {
        showSettingsModal, settingsServer, settingsValues,
        settingsActiveTab, setSettingsActiveTab,
        advancedExpanded, setAdvancedExpanded,
        availableVersions, versionsLoading, versionInstalling,
        resettingServer,
        editingModuleAliases, setEditingModuleAliases,
        editingCommandAliases, setEditingCommandAliases,
        isSettingsClosing, requestSettingsClose,
        handleOpenSettings, handleSettingChange, handleInstallVersion,
        handleResetServer, handleSaveSettings,
        handleSaveAliasesForModule, handleResetAliasesForModule,
    } = useServerSettings({
        servers, modules,
        setModal, setProgressBar,
        moduleAliasesPerModule,
        discordModuleAliases, discordCommandAliases,
        setDiscordModuleAliases, setDiscordCommandAliases,
        discordPrefix,
        fetchServers,
    });

    // ── Modal Close Animations ─────────────────────────────
    const closeDiscordSection = useCallback(() => setShowDiscordSection(false), []);
    const { isClosing: isDiscordClosing, requestClose: requestDiscordClose } = useModalClose(closeDiscordSection);
    const closeBackgroundSection = useCallback(() => setShowBackgroundSection(false), []);
    const { isClosing: isBackgroundClosing, requestClose: requestBackgroundClose } = useModalClose(closeBackgroundSection);
    const closeNoticeSection = useCallback(() => setShowNoticeSection(false), []);
    const { isClosing: isNoticeClosing, requestClose: requestNoticeClose } = useModalClose(closeNoticeSection);

    // ══════════════════════════════════════════════════════════
    // ── Effects ──────────────────────────────────────────────
    // ══════════════════════════════════════════════════════════

    // Unread notice count tracking
    useEffect(() => {
        const updateCount = () => {
            if (window.__sabaNotice) {
                setUnreadNoticeCount(window.__sabaNotice.getUnreadCount());
            }
        };
        updateCount();
        window.addEventListener('saba-notice-update', updateCount);
        return () => window.removeEventListener('saba-notice-update', updateCount);
    }, []);

    // Initialization status monitoring
    useEffect(() => {
        // HMR: if daemon is already running, skip loading screen
        if (window.api && window.api.daemonStatus) {
            window.api.daemonStatus().then((status) => {
                if (status && status.running) {
                    console.log('[HMR] Daemon already running, skipping loading screen');
                    setInitStatus('Ready!');
                    setInitProgress(100);
                    setDaemonReady(true);
                    setServersInitializing(false);
                }
            }).catch(() => {});
        }

        if (window.api && window.api.onStatusUpdate) {
            window.api.onStatusUpdate((data) => {
                console.log('[Init Status]', data.step, ':', data.message);

                const statusMessages = {
                    init: 'Initialize...', ui: 'UI loaded', daemon: 'Daemon preparing...',
                    modules: 'Loading modules...', instances: 'Loading instances...', ready: 'Checking servers...'
                };
                const progressValues = {
                    init: 10, ui: 20, daemon: 50, modules: 70, instances: 85, ready: 90
                };

                setInitStatus(statusMessages[data.step] || data.message);
                setInitProgress(progressValues[data.step] || initProgress);

                if (data.step === 'ready') {
                    setTimeout(() => setDaemonReady(true), 600);
                    setTimeout(() => setServersInitializing(false), 3500);
                }
            });
        }

        // Update notifications
        if (window.api && window.api.onUpdatesAvailable) {
            window.api.onUpdatesAvailable((data) => {
                console.log('[Updater] Updates available notification:', data);
                const count = data.count || data.updates_available || 0;
                const names = data.names || data.update_names || [];
                if (count > 0 && window.__sabaNotice) {
                    window.__sabaNotice.addNotice({
                        message: `📦 ${count}개 업데이트 발견: ${names.join(', ') || '확인 필요'}`,
                        type: 'info', source: 'Updater', action: 'openUpdateModal', dedup: true,
                    });
                }
            });
        }

        // Post-update completion notification
        if (window.api && window.api.onUpdateCompleted) {
            window.api.onUpdateCompleted((data) => {
                console.log('[Updater] Update completed notification:', data);
                setTimeout(() => {
                    if (typeof window.showToast === 'function') {
                        window.showToast(data.message || '업데이트가 완료되었습니다!', 'success', 5000, { isNotice: true, source: 'saba-chan' });
                    }
                    if (window.__sabaNotice) {
                        window.__sabaNotice.addNotice({
                            message: data.message || '업데이트가 완료되었습니다!',
                            type: 'success', source: 'Updater',
                        });
                    }
                }, 1500);
            });
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // Settings load
    useEffect(() => {
        const loadSettings = async () => {
            try {
                const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';

                const settings = await window.api.settingsLoad();
                if (!isTest) console.log('[Settings] Loaded:', settings);
                if (settings) {
                    setAutoRefresh(settings.autoRefresh ?? true);
                    setRefreshInterval(settings.refreshInterval ?? 2000);
                    setIpcPort(settings.ipcPort ?? 57474);
                    setConsoleBufferSize(settings.consoleBufferSize ?? 2000);
                    consoleBufferRef.current = settings.consoleBufferSize ?? 2000;
                    setModulesPath(settings.modulesPath || '');
                    setDiscordToken(settings.discordToken || '');
                    discordTokenRef.current = settings.discordToken || '';
                    setDiscordAutoStart(settings.discordAutoStart ?? false);
                    if (!isTest) console.log('[Settings] discordAutoStart:', settings.discordAutoStart, 'discordToken:', settings.discordToken ? 'YES' : 'NO');
                }
                const path = await window.api.settingsGetPath();
                setSettingsPath(path);
                if (!isTest) console.log('[Settings] GUI settings loaded from:', path);

                const botCfg = await window.api.botConfigLoad();
                if (botCfg) {
                    setDiscordPrefix(botCfg.prefix || '!saba');
                    setDiscordModuleAliases(botCfg.moduleAliases || {});
                    setDiscordCommandAliases(botCfg.commandAliases || {});
                    setDiscordMusicEnabled(botCfg.musicEnabled !== false);
                    // ★ 클라우드 모드 설정 로드
                    setDiscordBotMode(botCfg.mode || 'local');
                    setDiscordCloudRelayUrl(botCfg.cloud?.relayUrl || '');
                    setDiscordCloudHostId(botCfg.cloud?.hostId || '');
                    // ★ nodeSettings 로드 (기존 allowedInstances → local 노드로 마이그레이션)
                    if (botCfg.nodeSettings && typeof botCfg.nodeSettings === 'object') {
                        setNodeSettings(botCfg.nodeSettings);
                    } else if (Array.isArray(botCfg.allowedInstances)) {
                        setNodeSettings({ local: { allowedInstances: botCfg.allowedInstances, memberPermissions: {} } });
                    }
                    // ★ 클라우드 캐시 로드
                    if (Array.isArray(botCfg.cloudNodes)) setCloudNodes(botCfg.cloudNodes);
                    if (botCfg.cloudMembers && typeof botCfg.cloudMembers === 'object') setCloudMembers(botCfg.cloudMembers);
                    if (!isTest) console.log('[Settings] Bot config loaded, prefix:', botCfg.prefix, 'mode:', botCfg.mode || 'local');
                }

                setSettingsReady(true);
                if (!isTest) console.log('[Settings] Ready flag set to true');
            } catch (error) {
                console.error('[Settings] Failed to load settings:', error);
                setSettingsReady(true);
            }
        };
        loadSettings();
    }, []);

    // Finalize loading screen when server initialization completes
    useEffect(() => {
        if (!serversInitializing && daemonReady) {
            setInitProgress(100);
            setInitStatus('Ready!');
        }
    }, [serversInitializing, daemonReady]);

    // Background Daemon status polling
    useEffect(() => {
        if (!daemonReady) return;
        const checkDaemonStatus = async () => {
            try {
                if (window.api && window.api.daemonStatus) {
                    const status = await window.api.daemonStatus();
                    setBackgroundDaemonStatus(status.running ? 'running' : 'stopped');
                } else {
                    setBackgroundDaemonStatus('error');
                }
            } catch (error) {
                console.error('Failed to check daemon status:', error);
                setBackgroundDaemonStatus('error');
            }
        };
        checkDaemonStatus();
        const interval = setInterval(checkDaemonStatus, 5000);
        return () => clearInterval(interval);
    }, [daemonReady]);

    // ── Settings Save Functions ─────────────────────────────

    const loadBotConfig = async () => {
        try {
            const botCfg = await window.api.botConfigLoad();
            if (botCfg) {
                setDiscordPrefix(botCfg.prefix || '!saba');
                setDiscordModuleAliases(botCfg.moduleAliases || {});
                setDiscordCommandAliases(botCfg.commandAliases || {});
                setDiscordMusicEnabled(botCfg.musicEnabled !== false);
                setDiscordBotMode(botCfg.mode || 'local');
                setDiscordCloudRelayUrl(botCfg.cloud?.relayUrl || '');
                setDiscordCloudHostId(botCfg.cloud?.hostId || '');
                if (botCfg.nodeSettings && typeof botCfg.nodeSettings === 'object') {
                    setNodeSettings(botCfg.nodeSettings);
                } else if (Array.isArray(botCfg.allowedInstances)) {
                    setNodeSettings({ local: { allowedInstances: botCfg.allowedInstances, memberPermissions: {} } });
                }
                if (Array.isArray(botCfg.cloudNodes)) setCloudNodes(botCfg.cloudNodes);
                if (botCfg.cloudMembers && typeof botCfg.cloudMembers === 'object') setCloudMembers(botCfg.cloudMembers);
            }
        } catch (err) {
            console.error('Failed to load bot config:', err);
        }
    };

    const saveCurrentSettings = async () => {
        if (!settingsPath) {
            console.warn('[Settings] Settings path not initialized, skipping save');
            return;
        }
        try {
            await window.api.settingsSave({
                autoRefresh, refreshInterval, ipcPort, consoleBufferSize,
                modulesPath, discordToken, discordAutoStart
            });
            console.log('[Settings] GUI settings saved');
        } catch (error) {
            console.error('[Settings] Failed to save GUI settings:', error);
        }
    };

    const saveBotConfig = async (newPrefix = discordPrefix) => {
        try {
            const payload = {
                prefix: newPrefix || '!saba',
                mode: discordBotMode,
                cloud: {
                    relayUrl: discordCloudRelayUrl,
                    hostId: discordCloudHostId,
                },
                moduleAliases: discordModuleAliases,
                commandAliases: discordCommandAliases,
                musicEnabled: discordMusicEnabled,
                nodeSettings,
                cloudNodes,
                cloudMembers,
            };
            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                console.error('[Settings] Failed to save bot config:', res.error);
                safeShowToast(t('settings.save_error', '설정 저장 실패'), 'error');
            } else {
                console.log('[Settings] Bot config saved, prefix:', newPrefix);
            }
        } catch (error) {
            console.error('[Settings] Failed to save bot config:', error);
            safeShowToast(t('settings.save_error', '설정 저장 실패'), 'error');
        }
    };

    // ── Auto-save effects ───────────────────────────────────
    const prevSettingsRef = useRef(null);
    const prevPrefixRef = useRef(null);
    const prevCloudSettingsRef = useRef(null);

    useEffect(() => {
        if (!settingsReady || !settingsPath) return;
        const currentSettings = { autoRefresh, refreshInterval, ipcPort, consoleBufferSize };
        if (prevSettingsRef.current === null) {
            prevSettingsRef.current = currentSettings;
            return;
        }
        if (prevSettingsRef.current.autoRefresh !== autoRefresh ||
            prevSettingsRef.current.refreshInterval !== refreshInterval ||
            prevSettingsRef.current.ipcPort !== ipcPort ||
            prevSettingsRef.current.consoleBufferSize !== consoleBufferSize) {
            console.log('[Settings] Settings changed, saving...');
            saveCurrentSettings();
            prevSettingsRef.current = currentSettings;
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [settingsReady, autoRefresh, refreshInterval, ipcPort, consoleBufferSize]);

    useEffect(() => {
        if (!settingsReady || !settingsPath || !modulesPath) return;
        console.log('[Settings] Modules path changed, saving...', modulesPath);
        saveCurrentSettings();
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [modulesPath]);

    useEffect(() => {
        if (!settingsReady || !settingsPath) return;
        if (!discordPrefix || !discordPrefix.trim()) return;
        if (prevPrefixRef.current === null) {
            prevPrefixRef.current = discordPrefix;
            return;
        }
        if (prevPrefixRef.current !== discordPrefix) {
            console.log('[Settings] Prefix changed, saving bot config:', discordPrefix);
            saveBotConfig(discordPrefix);
            prevPrefixRef.current = discordPrefix;
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [settingsReady, discordPrefix]);

    // ★ 클라우드/노드 설정 변경 시 자동 저장 (mode, relayUrl, hostId, nodeSettings, cloudNodes, cloudMembers)
    useEffect(() => {
        if (!settingsReady || !settingsPath) return;
        const current = { discordBotMode, discordCloudRelayUrl, discordCloudHostId, nodeSettings, cloudNodes, cloudMembers };
        if (prevCloudSettingsRef.current === null) {
            prevCloudSettingsRef.current = current;
            return;
        }
        if (prevCloudSettingsRef.current.discordBotMode !== discordBotMode ||
            prevCloudSettingsRef.current.discordCloudRelayUrl !== discordCloudRelayUrl ||
            prevCloudSettingsRef.current.discordCloudHostId !== discordCloudHostId ||
            JSON.stringify(prevCloudSettingsRef.current.nodeSettings) !== JSON.stringify(nodeSettings) ||
            JSON.stringify(prevCloudSettingsRef.current.cloudNodes) !== JSON.stringify(cloudNodes) ||
            JSON.stringify(prevCloudSettingsRef.current.cloudMembers) !== JSON.stringify(cloudMembers)) {
            console.log('[Settings] Cloud/node settings changed, saving bot config:', { mode: discordBotMode, hostId: discordCloudHostId });
            saveBotConfig();
            prevCloudSettingsRef.current = current;
        }
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [settingsReady, discordBotMode, discordCloudRelayUrl, discordCloudHostId, nodeSettings, cloudNodes, cloudMembers]);

    // ── fetchModules ────────────────────────────────────────
    const fetchModules = async () => {
        try {
            console.log('Fetching modules...');
            try {
                await waitForDaemon(5000);
            } catch (err) {
                debugWarn('Daemon not ready, but continuing:', err.message);
            }

            const data = await retryWithBackoff(
                () => window.api.moduleList(),
                3, 800
            );

            console.log('Module data received:', data);
            if (data && data.modules) {
                console.log('Setting modules:', data.modules.length, 'modules');
                setModules(data.modules);

                // Register module locales
                for (const module of data.modules) {
                    try {
                        if (window.api.moduleGetLocales) {
                            const locales = await window.api.moduleGetLocales(module.name);
                            if (locales && typeof locales === 'object') {
                                for (const [lang, localeData] of Object.entries(locales)) {
                                    i18n.addResourceBundle(lang, `mod_${module.name}`, localeData, true, true);
                                }
                                console.log(`Module locales registered for ${module.name}:`, Object.keys(locales));
                            }
                        }
                    } catch (e) {
                        console.warn(`Failed to load locales for module ${module.name}:`, e);
                    }
                }

                // Load module metadata (aliases)
                const aliasesMap = {};
                for (const module of data.modules) {
                    try {
                        const metadata = await window.api.moduleGetMetadata(module.name);
                        if (metadata && metadata.toml) {
                            const aliases = metadata.toml.aliases || {};
                            const aliasCommands = aliases.commands || {};
                            const commandFields = metadata.toml.commands?.fields || [];
                            const mergedCommands = {};

                            for (const [cmdName, cmdData] of Object.entries(aliasCommands)) {
                                mergedCommands[cmdName] = {
                                    aliases: cmdData.aliases || [],
                                    description: cmdData.description || '',
                                    label: cmdName
                                };
                            }

                            for (const cmdField of commandFields) {
                                const cmdName = cmdField.name;
                                if (!mergedCommands[cmdName]) {
                                    mergedCommands[cmdName] = {
                                        aliases: [],
                                        description: cmdField.description || '',
                                        label: cmdField.label || cmdName
                                    };
                                } else {
                                    if (!mergedCommands[cmdName].description && cmdField.description) {
                                        mergedCommands[cmdName].description = cmdField.description;
                                    }
                                    if (cmdField.label) {
                                        mergedCommands[cmdName].label = cmdField.label;
                                    }
                                }
                            }

                            aliasesMap[module.name] = { ...aliases, commands: mergedCommands };
                        }
                    } catch (e) {
                        console.warn(`Failed to load metadata for module ${module.name}:`, e);
                    }
                }
                setModuleAliasesPerModule(aliasesMap);
                console.log('Module aliases loaded:', aliasesMap);
            } else if (data && data.error) {
                console.error('Module fetch error:', data.error);
                safeShowToast(t('modules.load_failed_toast', { error: translateError(data.error) }), 'error', 4000);
            } else {
                debugWarn('No modules data:', data);
                safeShowToast(t('modules.list_empty'), 'warning', 3000);
            }
        } catch (error) {
            console.error('Failed to fetch modules:', error);
            safeShowToast(t('modules.fetch_failed_toast', { error: translateError(error.message) }), 'error', 5000);
            setModal({ type: 'failure', title: t('modules.load_error_title'), message: translateError(error.message) });
        }
    };

    // ── Main initialization effect ──────────────────────────
    useEffect(() => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        if (!isTest) console.log('App mounted, fetching initial data...');
        fetchServers();
        fetchModules();
        loadBotConfig();

        // App close request handler
        if (window.api.onCloseRequest) {
            window.api.onCloseRequest(() => {
                setModal({
                    type: 'question',
                    title: t('app_exit.confirm_title'),
                    message: t('app_exit.confirm_message'),
                    detail: t('app_exit.confirm_detail'),
                    buttons: [
                        {
                            label: t('app_exit.hide_only_label'),
                            action: () => { window.api.closeResponse('hide'); setModal(null); }
                        },
                        {
                            label: t('app_exit.quit_all_label'),
                            action: () => { window.api.closeResponse('quit'); setModal(null); }
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => { window.api.closeResponse('cancel'); setModal(null); }
                        }
                    ]
                });
            });
        }

        // Auto-refresh (데몬 준비 전에는 스킵)
        const interval = setInterval(() => {
            if (autoRefresh && daemonReady) {
                fetchServers();
            }
        }, refreshInterval);

        return () => {
            clearInterval(interval);
            if (window.api.offCloseRequest) window.api.offCloseRequest();
        };
    // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [autoRefresh, refreshInterval, daemonReady]);

    // ══════════════════════════════════════════════════════════
    // ── Render ───────────────────────────────────────────────
    // ══════════════════════════════════════════════════════════

    // Loading screen (daemon not ready or servers still initializing)
    if (!daemonReady || serversInitializing) {
        return (
            <LoadingScreen logoSrc={logoSrc} initStatus={initStatus} initProgress={initProgress} />
        );
    }

    // Popout Console Mode (full-window console)
    if (isPopoutMode) {
        return (
            <PopoutConsole
                popoutParams={popoutParams}
                consoleLines={consoleLines}
                consoleInput={consoleInput}
                setConsoleInput={setConsoleInput}
                sendConsoleCommand={sendConsoleCommand}
                consoleEndRef={consoleEndRef}
                highlightRules={(() => {
                    const srv = servers.find(s => s.id === popoutParams.instanceId);
                    const mod = srv && modules.find(m => m.name === srv.module);
                    return mod?.syntax_highlight?.rules || null;
                })()}
            />
        );
    }

    return (
        <ExtensionProvider>
        <div className="App">
            {/* Discord overlay backdrop */}
            {showDiscordSection && (
                <div className="discord-backdrop" onClick={requestDiscordClose} />
            )}
            {/* Background overlay backdrop */}
            {showBackgroundSection && (
                <div className="discord-backdrop" onClick={requestBackgroundClose} />
            )}
            {/* Notice overlay backdrop */}
            {showNoticeSection && (
                <div className="discord-backdrop" onClick={requestNoticeClose} />
            )}
            <TitleBar />
            <Toast />
            <header className="app-header">
                {/* 첫 번째 줄: 타이틀과 설정 */}
                <div className="header-row header-row-title">
                    <div className="app-title-section">
                        <img src="./icon.png" alt="" className="app-logo-icon" />
                        <img src={logoSrc} alt={t('common:app_name')} className="app-logo-text" />
                    </div>
                    <div className="header-actions">
                        <div className="notice-button-wrapper">
                            <button 
                                className="btn-settings-icon-solo"
                                onClick={() => showNoticeSection ? requestNoticeClose() : setShowNoticeSection(true)}
                                title={t('notice_modal.tooltip')}
                            >
                                <Icon name="bell" size="lg" />
                            </button>
                            {unreadNoticeCount > 0 && (
                                <span className="notice-badge-dot">{unreadNoticeCount > 9 ? '9+' : unreadNoticeCount}</span>
                            )}
                            <NoticeModal
                                isOpen={showNoticeSection}
                                onClose={requestNoticeClose}
                                isClosing={isNoticeClosing}
                                onOpenUpdateModal={() => {
                                    setSettingsInitialView('update');
                                    setShowGuiSettingsModal(true);
                                }}
                            />
                        </div>
                        <button 
                            className="btn-settings-icon-solo"
                            onClick={() => setShowGuiSettingsModal(true)}
                            title={t('settings.gui_settings_tooltip')}
                        >
                            <Icon name="cog" size="lg" />
                        </button>
                    </div>
                </div>
                
                {/* 두 번째 줄: 기능 버튼들 */}
                <div className="header-row header-row-controls">
                    <button 
                        className="btn btn-add"
                        onClick={() => setShowModuleManager(!showModuleManager)}
                    >
                        <Icon name="plus" size="sm" /> Add Server
                    </button>
                    <div className="header-spacer"></div>
                    <div className="discord-button-wrapper">
                        <button 
                            className={`btn btn-discord ${discordBotStatus === 'running' ? 'btn-discord-active' : ''}`}
                            onClick={() => showDiscordSection ? requestDiscordClose() : setShowDiscordSection(true)}
                        >
                            <span className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : discordBotStatus === 'connecting' ? 'status-connecting' : 'status-offline'}`}></span>
                            Discord Bot
                        </button>
                        <DiscordBotModal
                            isOpen={showDiscordSection}
                            onClose={requestDiscordClose}
                            isClosing={isDiscordClosing}
                            discordBotStatus={discordBotStatus}
                            discordToken={discordToken}
                            setDiscordToken={(val) => { setDiscordToken(val); discordTokenRef.current = val; }}
                            discordPrefix={discordPrefix}
                            setDiscordPrefix={setDiscordPrefix}
                            discordAutoStart={discordAutoStart}
                            setDiscordAutoStart={setDiscordAutoStart}
                            discordMusicEnabled={discordMusicEnabled}
                            setDiscordMusicEnabled={setDiscordMusicEnabled}
                            discordBotMode={discordBotMode}
                            setDiscordBotMode={setDiscordBotMode}
                            discordCloudRelayUrl={discordCloudRelayUrl}
                            setDiscordCloudRelayUrl={setDiscordCloudRelayUrl}
                            discordCloudHostId={discordCloudHostId}
                            setDiscordCloudHostId={setDiscordCloudHostId}
                            relayConnected={relayConnected}
                            relayConnecting={relayConnecting}
                            handleStartDiscordBot={handleStartDiscordBot}
                            handleStopDiscordBot={handleStopDiscordBot}
                            saveCurrentSettings={saveCurrentSettings}
                            servers={servers}
                            modules={modules}
                            moduleAliasesPerModule={moduleAliasesPerModule}
                            nodeSettings={nodeSettings}
                            setNodeSettings={setNodeSettings}
                            cloudNodes={cloudNodes}
                            setCloudNodes={setCloudNodes}
                            cloudMembers={cloudMembers}
                            setCloudMembers={setCloudMembers}
                        />
                    </div>
                    <div className="background-button-wrapper">
                        <button 
                            className={`btn btn-background ${backgroundDaemonStatus === 'running' ? 'btn-background-active' : ''}`}
                            onClick={() => showBackgroundSection ? requestBackgroundClose() : setShowBackgroundSection(true)}
                        >
                            <span className={`status-indicator ${
                                backgroundDaemonStatus === 'running' ? 'status-online' : 
                                backgroundDaemonStatus === 'checking' ? 'status-checking' : 
                                'status-offline'
                            }`}></span>
                            Background
                        </button>
                        <BackgroundModal
                            isOpen={showBackgroundSection}
                            onClose={requestBackgroundClose}
                            isClosing={isBackgroundClosing}
                            ipcPort={ipcPort}
                        />
                    </div>
                </div>
            </header>

            {/* AddServerModal */}
            <AddServerModal
                isOpen={showModuleManager}
                onClose={() => setShowModuleManager(false)}
                extensions={modules}
                servers={servers}
                extensionsPath={modulesPath}
                settingsPath={settingsPath}
                onextensionsPathChange={setModulesPath}
                onRefreshextensions={fetchModules}
                onAddServer={handleAddServer}
            />

            <main className="app-main">
                <div className="server-list">
                {servers.length === 0 ? (
                    <div className="no-servers">
                        <p>{t('servers.no_servers_configured', { defaultValue: 'No servers configured' })}</p>
                    </div>
                ) : (
                    servers.map((server, index) => (
                        <ServerCard
                            key={server.name}
                            server={server}
                            index={index}
                            modules={modules}
                            servers={servers}
                            cardRefs={cardRefs}
                            draggedName={draggedName}
                            skipNextClick={skipNextClick}
                            consoleServer={consoleServer}
                            consolePopoutInstanceId={consolePopoutInstanceId}
                            handleCardPointerDown={handleCardPointerDown}
                            handleStart={handleStart}
                            handleStop={handleStop}
                            handleOpenSettings={handleOpenSettings}
                            handleDeleteServer={handleDeleteServer}
                            openConsole={openConsole}
                            closeConsole={closeConsole}
                            setCommandServer={setCommandServer}
                            setShowCommandModal={setShowCommandModal}
                            setServers={setServers}
                            formatUptime={formatUptime}
                            nowEpoch={nowEpoch}
                            onContextMenu={(e) => {
                                e.preventDefault();
                                setContextMenu({ x: e.clientX, y: e.clientY, server });
                            }}
                        />
                    ))
                )}
                </div>

                {/* 콘솔 패널 — 팝아웃 중이면 숨김 */}
                {consoleServer && !consolePopoutInstanceId && (
                    <ConsolePanel
                        consoleServer={consoleServer}
                        consoleLines={consoleLines}
                        consoleInput={consoleInput}
                        setConsoleInput={setConsoleInput}
                        sendConsoleCommand={sendConsoleCommand}
                        consoleEndRef={consoleEndRef}
                        closeConsole={closeConsole}
                        setConsolePopoutInstanceId={setConsolePopoutInstanceId}
                        highlightRules={(() => {
                            const srv = servers.find(s => s.id === consoleServer.id);
                            const mod = srv && modules.find(m => m.name === srv.module);
                            return mod?.syntax_highlight?.rules || null;
                        })()}
                    />
                )}

            </main>

            {showSettingsModal && settingsServer && (
                <ServerSettingsModal
                    settingsServer={settingsServer}
                    settingsValues={settingsValues}
                    settingsActiveTab={settingsActiveTab}
                    setSettingsActiveTab={setSettingsActiveTab}
                    modules={modules}
                    advancedExpanded={advancedExpanded}
                    setAdvancedExpanded={setAdvancedExpanded}
                    availableVersions={availableVersions}
                    versionsLoading={versionsLoading}
                    versionInstalling={versionInstalling}
                    handleSettingChange={handleSettingChange}
                    handleInstallVersion={handleInstallVersion}
                    handleSaveSettings={handleSaveSettings}
                    handleResetServer={handleResetServer}
                    resettingServer={resettingServer}
                    editingModuleAliases={editingModuleAliases}
                    setEditingModuleAliases={setEditingModuleAliases}
                    editingCommandAliases={editingCommandAliases}
                    setEditingCommandAliases={setEditingCommandAliases}
                    handleSaveAliasesForModule={handleSaveAliasesForModule}
                    handleResetAliasesForModule={handleResetAliasesForModule}
                    isClosing={isSettingsClosing}
                    onClose={requestSettingsClose}
                    servers={servers}
                    moduleAliasesPerModule={moduleAliasesPerModule}
                    discordModuleAliases={discordModuleAliases}
                />
            )}

            {/* Context Menu */}
            {contextMenu && (
                <>
                    <div className="context-menu-overlay" onClick={() => setContextMenu(null)} onContextMenu={(e) => { e.preventDefault(); setContextMenu(null); }} />
                    <div className="context-menu" style={{ top: contextMenu.y, left: contextMenu.x }}>
                        <div className="context-menu-item" onClick={() => { handleOpenSettings(contextMenu.server); setContextMenu(null); }}>
                            <Icon name="settings" size="sm" />
                            {t('context_menu.settings', { defaultValue: 'Settings' })}
                        </div>
                        <div className="context-menu-separator" />
                        <div className="context-menu-item danger" onClick={() => { handleDeleteServer(contextMenu.server); setContextMenu(null); }}>
                            <Icon name="trash" size="sm" />
                            {t('context_menu.delete', { defaultValue: 'Delete' })}
                        </div>
                    </div>
                </>
            )}

            {/* 모달 렌더링 */}
            {modal && modal.type === 'success' && (
                <SuccessModal title={modal.title} message={modal.message} onClose={() => setModal(null)} />
            )}
            {modal && modal.type === 'failure' && (
                <FailureModal title={modal.title} message={modal.message} onClose={() => setModal(null)} />
            )}
            {modal && modal.type === 'notification' && (
                <NotificationModal title={modal.title} message={modal.message} onClose={() => setModal(null)} />
            )}
            {modal && modal.type === 'question' && (
                <QuestionModal
                    title={modal.title}
                    message={modal.message}
                    detail={modal.detail}
                    buttons={modal.buttons}
                    onConfirm={modal.onConfirm}
                    onCancel={() => setModal(null)}
                />
            )}

            {/* SettingsModal 렌더링 */}
            <SettingsModal 
                isOpen={showGuiSettingsModal} 
                onClose={() => { setShowGuiSettingsModal(false); setSettingsInitialView(null); }}
                refreshInterval={refreshInterval}
                onRefreshIntervalChange={setRefreshInterval}
                ipcPort={ipcPort}
                onIpcPortChange={setIpcPort}
                consoleBufferSize={consoleBufferSize}
                onConsoleBufferSizeChange={(val) => { setConsoleBufferSize(val); consoleBufferRef.current = val; }}
                discordCloudRelayUrl={discordCloudRelayUrl}
                onDiscordCloudRelayUrlChange={setDiscordCloudRelayUrl}
                onTestModal={setModal}
                onTestProgressBar={setProgressBar}
                initialView={settingsInitialView}
                onTestWaitingImage={() => {
                    setShowWaitingImage(true);
                    setTimeout(() => setShowWaitingImage(false), 4000);
                }}
                onTestLoadingScreen={() => {
                    setShowGuiSettingsModal(false);
                    setDaemonReady(false);
                    setServersInitializing(true);
                    setInitStatus('Loading test...');
                    setInitProgress(0);
                    let p = 0;
                    const iv = setInterval(() => {
                        p += Math.random() * 20 + 10;
                        if (p >= 100) {
                            p = 100;
                            setInitStatus('Ready!');
                            setInitProgress(100);
                            clearInterval(iv);
                            setTimeout(() => {
                                setDaemonReady(true);
                                setServersInitializing(false);
                            }, 600);
                        } else {
                            setInitStatus(`Loading test... ${Math.round(p)}%`);
                            setInitProgress(p);
                        }
                    }, 500);
                }}
            />

            {/* CommandModal 렌더링 */}
            {showCommandModal && commandServer && (
                <CommandModal
                    server={commandServer}
                    modules={modules}
                    onClose={() => setShowCommandModal(false)}
                    onExecute={setModal}
                />
            )}

            {/* waiting.png (느린 진행 감지) */}
            {showWaitingImage && (
                <div className="waiting-image-overlay" onClick={() => setShowWaitingImage(false)}>
                    <img src="./waiting.png" alt="waiting" className="waiting-image" />
                </div>
            )}

            {/* 글로벌 프로그레스바 */}
            {progressBar && (
                <div className="global-progress-bar">
                    <div className="global-progress-content">
                        <span className="global-progress-message">{progressBar.message}</span>
                        {progressBar.percent != null && !progressBar.indeterminate && (
                            <span className="global-progress-percent">{Math.round(progressBar.percent)}%</span>
                        )}
                    </div>
                    <div className="global-progress-track">
                        <div
                            className={`global-progress-fill ${progressBar.indeterminate ? 'indeterminate' : ''} ${progressBar.percent === 100 ? 'complete' : ''}`}
                            style={progressBar.indeterminate ? {} : { width: `${progressBar.percent || 0}%` }}
                        />
                    </div>
                </div>
            )}
        </div>
        </ExtensionProvider>
    );
}

export default App;
