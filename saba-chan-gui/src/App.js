import { useCallback, useEffect, useMemo, useRef, } from 'react';
import { useTranslation } from 'react-i18next';
import './App.css';
import {
    AddInstanceModal,
    BackgroundModal,
    CommandModal,
    ConsoleDock,
    DiscordBotModal,
    FailureModal,
    Icon,
    LoadingScreen,
    NoticeModal,
    NotificationModal,
    PopoutConsole,
    QuestionModal,
    ServerCard,
    ServerSettingsModal,
    SettingsModal,
    SuccessModal,
    TitleBar,
    Toast,
    ConsoleWindow,
} from './components';
import { ExtensionProvider } from './contexts/ExtensionContext';
import { useMultiConsole } from './hooks/useMultiConsole';
import { useDragReorder } from './hooks/useDragReorder';
import useExtensionInitStatus from './hooks/useExtensionInitStatus';
import { useModalClose } from './hooks/useModalClose';
import { useServerActions } from './hooks/useServerActions';
import { useServerSettings } from './hooks/useServerSettings';
import { useWindowSize, SIDE_PANEL_MIN_WIDTH, SIDE_PANEL_MIN_HEIGHT } from './hooks/useWindowSize';
import { setDiscordI18n, useDiscordStore } from './stores/useDiscordStore';
import { setServerI18n, useServerStore } from './stores/useServerStore';
import { useSettingsStore } from './stores/useSettingsStore';
import { useUIStore } from './stores/useUIStore';
function App() {
    const { t, i18n } = useTranslation('gui');

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

    // ── 윈도우 크기 → 사이드 패널 모드 판정 ──
    const windowSize = useWindowSize();
    const discordSideMode = windowSize.width >= SIDE_PANEL_MIN_WIDTH && windowSize.height >= SIDE_PANEL_MIN_HEIGHT;

    // ── Server Store (Zustand) ─────────────────────────────
    const servers = useServerStore((s) => s.servers);
    const setServers = useServerStore((s) => s.setServers);
    const modules = useServerStore((s) => s.modules);
    const loading = useServerStore((s) => s.loading);
    const moduleAliasesPerModule = useServerStore((s) => s.moduleAliasesPerModule);
    const daemonReady = useServerStore((s) => s.daemonReady);
    const setDaemonReady = useServerStore((s) => s.setDaemonReady);
    const initStatus = useServerStore((s) => s.initStatus);
    const setInitStatus = useServerStore((s) => s.setInitStatus);
    const initProgress = useServerStore((s) => s.initProgress);
    const setInitProgress = useServerStore((s) => s.setInitProgress);
    const serversInitializing = useServerStore((s) => s.serversInitializing);
    const setServersInitializing = useServerStore((s) => s.setServersInitializing);
    const nowEpoch = useServerStore((s) => s.nowEpoch);
    const formatUptime = useServerStore((s) => s.formatUptime);

    // ── UI Store (Zustand) ─────────────────────────────────
    const modal = useUIStore((s) => s.modal);
    const setModal = useUIStore((s) => s.openModal);
    const progressBar = useUIStore((s) => s.progressBar);
    const setProgressBar = useUIStore((s) => s.setProgressBar);
    const showModuleManager = useUIStore((s) => s.showModuleManager);
    const setShowModuleManager = useUIStore((s) => s.setShowModuleManager);
    const showGuiSettingsModal = useUIStore((s) => s.showGuiSettingsModal);
    const settingsInitialView = useUIStore((s) => s.settingsInitialView);
    const showCommandModal = useUIStore((s) => s.showCommandModal);
    const setShowCommandModal = useUIStore((s) => s.setShowCommandModal);
    const commandServer = useUIStore((s) => s.commandServer);
    const setCommandServer = useUIStore((s) => s.setCommandServer);
    const contextMenu = useUIStore((s) => s.contextMenu);
    const setContextMenu = useUIStore((s) => s.setContextMenu);
    const showDiscordSection = useUIStore((s) => s.showDiscordSection);
    const setShowDiscordSection = useUIStore((s) => s.setShowDiscordSection);
    const showBackgroundSection = useUIStore((s) => s.showBackgroundSection);
    const setShowBackgroundSection = useUIStore((s) => s.setShowBackgroundSection);
    const showNoticeSection = useUIStore((s) => s.showNoticeSection);
    const setShowNoticeSection = useUIStore((s) => s.setShowNoticeSection);
    const unreadNoticeCount = useUIStore((s) => s.unreadNoticeCount);
    const setUnreadNoticeCount = useUIStore((s) => s.setUnreadNoticeCount);
    const backgroundDaemonStatus = useUIStore((s) => s.backgroundDaemonStatus);
    const setBackgroundDaemonStatus = useUIStore((s) => s.setBackgroundDaemonStatus);
    const showWaitingImage = useUIStore((s) => s.showWaitingImage);
    const setShowWaitingImage = useUIStore((s) => s.setShowWaitingImage);

    // ── Settings Store (Zustand) ──────────────────────────
    const autoRefresh = useSettingsStore((s) => s.autoRefresh);
    const refreshInterval = useSettingsStore((s) => s.refreshInterval);
    const ipcPort = useSettingsStore((s) => s.ipcPort);
    const consoleBufferSize = useSettingsStore((s) => s.consoleBufferSize);
    const consoleBufferRef = useRef(2000);
    const _settingsReady = useSettingsStore((s) => s.settingsReady);

    // ── Discord Store (Zustand) ──────────────────────────────
    const discordToken = useDiscordStore((s) => s.discordToken);
    const discordPrefix = useDiscordStore((s) => s.discordPrefix);
    const discordAutoStart = useDiscordStore((s) => s.discordAutoStart);
    const discordModuleAliases = useDiscordStore((s) => s.discordModuleAliases);
    const discordCommandAliases = useDiscordStore((s) => s.discordCommandAliases);
    const discordMusicEnabled = useDiscordStore((s) => s.discordMusicEnabled);
    const discordMusicChannelId = useDiscordStore((s) => s.discordMusicChannelId);
    const discordMusicUISettings = useDiscordStore((s) => s.discordMusicUISettings);
    const discordBotMode = useDiscordStore((s) => s.discordBotMode);
    const discordCloudRelayUrl = useDiscordStore((s) => s.discordCloudRelayUrl);
    const discordCloudHostId = useDiscordStore((s) => s.discordCloudHostId);
    const nodeSettings = useDiscordStore((s) => s.nodeSettings);
    const cloudNodes = useDiscordStore((s) => s.cloudNodes);
    const cloudMembers = useDiscordStore((s) => s.cloudMembers);
    const discordBotStatus = useDiscordStore((s) => s.discordBotStatus);
    const relayConnected = useDiscordStore((s) => s.relayConnected);
    const relayConnecting = useDiscordStore((s) => s.relayConnecting);

    // ══════════════════════════════════════════════════════════
    // ── Custom Hooks ─────────────────────────────────────────
    // ══════════════════════════════════════════════════════════

    const {
        consoles,
        consoleServer,
        consolePopoutInstanceId,
        setConsolePopoutInstanceId,
        openConsole,
        closeConsole,
        minimizeConsole,
        restoreConsole,
        focusConsole,
        togglePin,
        popinConsole,
        updatePosition,
        updateSize,
        setConsoleInput,
        sendConsoleCommand,
        isConsoleOpen,
    } = useMultiConsole({ isPopoutMode, popoutParams, consoleBufferRef });

    const { draggedName, cardRefs, skipNextClick, handleCardPointerDown } = useDragReorder(servers, setServers);

    // 익스텐션 초기화 상태 (daemon.startup hook 진행 중이면 스피너 표시)
    const { initializing: extInitializing, inProgress: extInitInProgress } = useExtensionInitStatus();

    // Discord store action aliases
    const handleStartDiscordBot = useDiscordStore((s) => s.startBot);
    const handleStopDiscordBot = useDiscordStore((s) => s.stopBot);

    const { fetchServers, handleStart, handleStop, handleAddServer, handleDeleteServer } =
        useServerActions({
            servers,
            setServers,
            modules,
            loading,
            setLoading: (val) => useServerStore.setState({ loading: val }),
            setModal,
            setProgressBar,
            openConsole,
            closeConsole,
            setShowModuleManager,
            formatUptime,
            openSettingsToExtensions: () => {
                useUIStore.getState().openSettings('extensions');
            },
        });

    // Store-based fetchModules
    const fetchModules = useServerStore((s) => s.fetchModules);

    const {
        showSettingsModal,
        settingsServer,
        settingsValues,
        settingsActiveTab,
        setSettingsActiveTab,
        advancedExpanded,
        setAdvancedExpanded,
        availableVersions,
        versionsLoading,
        versionInstalling,
        resettingServer,
        editingModuleAliases,
        setEditingModuleAliases,
        editingCommandAliases,
        setEditingCommandAliases,
        isSettingsClosing,
        requestSettingsClose,
        handleOpenSettings,
        handleSettingChange,
        handleInstallVersion,
        handleResetServer,
        handleSaveSettings,
        handleSaveAliasesForModule,
        handleResetAliasesForModule,
    } = useServerSettings({
        servers,
        modules,
        setModal,
        setProgressBar,
        moduleAliasesPerModule,
        discordModuleAliases,
        discordCommandAliases,
        setDiscordModuleAliases: (val) => useDiscordStore.getState().update({ discordModuleAliases: val }),
        setDiscordCommandAliases: (val) => useDiscordStore.getState().update({ discordCommandAliases: val }),
        discordPrefix,
        fetchServers,
    });

    // ── Modal Close Animations ─────────────────────────────
    // biome-ignore lint/correctness/useExhaustiveDependencies: useState setters are stable
    const closeDiscordSection = useCallback(() => setShowDiscordSection(false), []);
    const { isClosing: isDiscordClosing, requestClose: requestDiscordClose } = useModalClose(closeDiscordSection);
    // biome-ignore lint/correctness/useExhaustiveDependencies: useState setters are stable
    const closeBackgroundSection = useCallback(() => setShowBackgroundSection(false), []);
    const { isClosing: isBackgroundClosing, requestClose: requestBackgroundClose } =
        useModalClose(closeBackgroundSection);
    // biome-ignore lint/correctness/useExhaustiveDependencies: useState setters are stable
    const closeNoticeSection = useCallback(() => setShowNoticeSection(false), []);
    const { isClosing: isNoticeClosing, requestClose: requestNoticeClose } = useModalClose(closeNoticeSection);

    // ══════════════════════════════════════════════════════════
    // ── Effects ──────────────────────────────────────────────
    // ══════════════════════════════════════════════════════════

    // Waiting image monitoring (progress stall detection)
    const waitingTimerRef = useRef(null);
    const progressSnapshotRef = useRef(null);
    useEffect(() => {
        if (!progressBar) {
            setShowWaitingImage(false);
            if (waitingTimerRef.current) clearInterval(waitingTimerRef.current);
            progressSnapshotRef.current = null;
            return;
        }
        if (progressBar.percent === 100) {
            setShowWaitingImage(false);
            if (waitingTimerRef.current) clearInterval(waitingTimerRef.current);
            progressSnapshotRef.current = null;
            return;
        }
        if (!progressSnapshotRef.current) {
            progressSnapshotRef.current = { percent: progressBar.percent || 0, timestamp: Date.now() };
        }
        if (!waitingTimerRef.current) {
            waitingTimerRef.current = setInterval(() => {
                const snap = progressSnapshotRef.current;
                if (!snap) return;
                const elapsed = (Date.now() - snap.timestamp) / 1000;
                if (elapsed >= 5) setShowWaitingImage(true);
            }, 1000);
        }
        const currentPercent = progressBar.percent || 0;
        const snap = progressSnapshotRef.current;
        if (snap && currentPercent - snap.percent > 5) {
            progressSnapshotRef.current = { percent: currentPercent, timestamp: Date.now() };
            setShowWaitingImage(false);
        }
        return () => {
            if (waitingTimerRef.current) {
                clearInterval(waitingTimerRef.current);
                waitingTimerRef.current = null;
            }
        };
    }, [progressBar, setShowWaitingImage]);

    // Unread notice count tracking
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only event listener — setter is stable
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
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only IPC registration — setters are stable, initProgress stale fixed via functional updater
    useEffect(() => {
        // HMR / page-reload recovery: always check daemon health on mount.
        // On cold boot the daemon hasn't started yet so daemonStatus() returns
        // { running: false } and we fall through to the onStatusUpdate listener.
        // On HMR or Vite full-reload the daemon is already alive, so we skip
        // the loading screen immediately.
        if (window.api && window.api.daemonStatus) {
            window.api
                .daemonStatus()
                .then((status) => {
                    if (status && status.running) {
                        console.log('[HMR] Daemon already running, skipping loading screen');
                        window.__sabaReadyReceived = true;
                        setInitStatus('Ready!');
                        setInitProgress(100);
                        setDaemonReady(true);
                        setServersInitializing(false);
                    }
                })
                .catch(() => {});
        }

        // Remove any previously stacked listeners (prevents duplicates on Fast Refresh)
        if (window.api && window.api.offStatusUpdate) {
            window.api.offStatusUpdate();
        }

        if (window.api && window.api.onStatusUpdate) {
            window.api.onStatusUpdate((data) => {
                console.log('[Init Status]', data.step, ':', data.message);

                const statusMessages = {
                    init: 'Initialize...',
                    ui: 'UI loaded',
                    daemon: 'Daemon preparing...',
                    modules: 'Loading modules...',
                    instances: 'Loading instances...',
                    ready: 'Checking servers...',
                };
                const progressValues = {
                    init: 10,
                    ui: 20,
                    daemon: 50,
                    modules: 70,
                    instances: 85,
                    ready: 90,
                };

                setInitStatus(statusMessages[data.step] || data.message);
                setInitProgress((prev) => Math.max(prev, progressValues[data.step] || prev));

                if (data.step === 'ready') {
                    window.__sabaReadyReceived = true;
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
                        message: `📦 ${t('updates.found', { count, names: names.join(', ') || t('updates.check_required') })}`,
                        type: 'info',
                        source: 'Updater',
                        action: 'openUpdateModal',
                        dedup: true,
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
                        window.showToast(data.message || t('updates.completed'), 'success', 5000, {
                            isNotice: true,
                            source: 'saba-chan',
                        });
                    }
                    if (window.__sabaNotice) {
                        window.__sabaNotice.addNotice({
                            message: data.message || t('updates.completed'),
                            type: 'success',
                            source: 'Updater',
                        });
                    }
                }, 1500);
            });
        }

        return () => {
            // Cleanup IPC listeners on unmount / Fast Refresh re-run
            if (window.api && window.api.offStatusUpdate) {
                window.api.offStatusUpdate();
            }
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, []);

    // Settings load
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only initialization — store.getState() is stable
    useEffect(() => {
        // Start uptime clock
        useServerStore.getState().startUptimeClock();

        const loadSettings = async () => {
            const settings = await useSettingsStore.getState().load();
            if (settings) {
                consoleBufferRef.current = settings.consoleBufferSize ?? 2000;
            }

            // Load bot config into discord store (token/autoStart는 bot-config이 SSOT)
            await useDiscordStore.getState().loadConfig();

            // ── 마이그레이션: settings.json → bot-config.json ──
            // 기존 settings.json에 discordToken/discordAutoStart가 있고
            // bot-config에는 아직 없으면 이전
            if (settings) {
                const discordState = useDiscordStore.getState();
                const needsMigration =
                    (settings.discordToken && !discordState.discordToken) ||
                    (settings.discordAutoStart && !discordState.discordAutoStart);
                if (needsMigration) {
                    console.log('[Migration] Migrating discordToken/autoStart from settings to bot-config');
                    useDiscordStore.getState().update({
                        discordToken: settings.discordToken || discordState.discordToken || '',
                        _discordTokenRef: settings.discordToken || discordState._discordTokenRef || '',
                        discordAutoStart: settings.discordAutoStart ?? discordState.discordAutoStart ?? false,
                    });
                    // 즉시 bot-config에 저장
                    await useDiscordStore.getState().saveConfig();
                }
            }

            useDiscordStore.getState().update({ _settingsReady: true });

            // Start status polling and listeners
            useDiscordStore.getState().startStatusPolling();
            useDiscordStore.getState().initListeners();
        };
        loadSettings();

        return () => {
            useDiscordStore.getState().stopStatusPolling();
            useServerStore.getState().stopUptimeClock();
        };
    }, []);

    // Sync i18n for Zustand stores when language changes
    useEffect(() => {
        setDiscordI18n(t);
        setServerI18n(t, i18n);
    }, [t, i18n]);

    // Discord auto-start: monitor botStatusReady (Zustand v5 single-listener)
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only Zustand subscribe
    useEffect(() => {
        const unsub = useDiscordStore.subscribe((state, prevState) => {
            if (
                state.botStatusReady !== prevState.botStatusReady ||
                state._settingsReady !== prevState._settingsReady
            ) {
                useDiscordStore.getState().tryAutoStart();
            }
        });
        return unsub;
    }, []);

    // Finalize loading screen when server initialization completes
    // biome-ignore lint/correctness/useExhaustiveDependencies: setters are stable
    useEffect(() => {
        if (!serversInitializing && daemonReady) {
            setInitProgress(100);
            setInitStatus('Ready!');
        }
    }, [serversInitializing, daemonReady]);

    // Background Daemon status polling
    // biome-ignore lint/correctness/useExhaustiveDependencies: setter is stable
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

    const saveCurrentSettings = async () => {
        try {
            await Promise.all([useSettingsStore.getState().save(), useDiscordStore.getState().saveConfig()]);
        } catch (err) {
            console.error('[App] Failed to save settings:', err.message);
        }
    };

    // ── One-time module fetch (mount only) ──────────────────
    // biome-ignore lint/correctness/useExhaustiveDependencies: mount-only — store.getState() is stable
    useEffect(() => {
        useServerStore.getState().fetchModules();
    }, []);

    // ── Main initialization + auto-refresh ──────────────────
    // biome-ignore lint/correctness/useExhaustiveDependencies: fetchServers/t/setModal are intentionally omitted — adding them would cause interval re-registration on every action
    useEffect(() => {
        const isTest = process.env.NODE_ENV === 'test' || typeof jest !== 'undefined';
        if (!isTest) console.log('App mounted, fetching initial data...');
        fetchServers();

        // App close request handler
        if (window.api.onCloseRequest) {
            window.api.onCloseRequest(() => {
                // managed 모드로 실행 중인 인스턴스 검출
                const currentServers = useServerStore.getState().servers;
                const currentModules = useServerStore.getState().modules;
                const runningManaged = currentServers.filter((srv) => {
                    if (srv.status !== 'running') return false;
                    const mod = currentModules.find((m) => m.name === srv.module);
                    const instanceManaged = srv.module_settings?.managed_start;
                    if (instanceManaged === true) return true;
                    if (instanceManaged === false) return false;
                    return (mod?.interaction_mode || 'console') === 'console';
                });

                const doQuit = () => {
                    window.api.closeResponse('quit');
                    setModal(null);
                };

                const showExitModal = () => {
                    setModal({
                        type: 'question',
                        title: t('app_exit.confirm_title'),
                        message: t('app_exit.confirm_message'),
                        detail: t('app_exit.confirm_detail'),
                        buttons: [
                            {
                                label: t('app_exit.exit_interface_label'),
                                action: () => {
                                    window.api.closeResponse('exit-interface');
                                    setModal(null);
                                },
                            },
                            {
                                label: t('app_exit.quit_all_label'),
                                action: () => {
                                    if (runningManaged.length > 0) {
                                        // managed 인스턴스가 실행 중 → 2차 경고
                                        const names = runningManaged.map((s) => s.name).join(', ');
                                        setModal({
                                            type: 'warning',
                                            title: t('app_exit.managed_warning_title'),
                                            message: t('app_exit.managed_warning_message', { names, count: runningManaged.length }),
                                            detail: t('app_exit.managed_warning_detail'),
                                            buttons: [
                                                {
                                                    label: t('app_exit.quit_all_label'),
                                                    action: doQuit,
                                                },
                                                {
                                                    label: t('modals.cancel'),
                                                    action: () => {
                                                        window.api.closeResponse('cancel');
                                                        setModal(null);
                                                    },
                                                },
                                            ],
                                        });
                                    } else {
                                        doQuit();
                                    }
                                },
                            },
                            {
                                label: t('modals.cancel'),
                                action: () => {
                                    window.api.closeResponse('cancel');
                                    setModal(null);
                                },
                            },
                        ],
                    });
                };

                showExitModal();
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
    }, [autoRefresh, refreshInterval, daemonReady]);

    // ══════════════════════════════════════════════════════════
    // ── Render ───────────────────────────────────────────────
    // ══════════════════════════════════════════════════════════

    // Loading screen (daemon not ready or servers still initializing)
    if (!daemonReady || serversInitializing) {
        return <LoadingScreen logoSrc={logoSrc} initStatus={initStatus} initProgress={initProgress} />;
    }

    // Popout Console Mode (full-window console)
    if (isPopoutMode) {
        const popoutState = consoles[popoutParams.instanceId];
        return (
            <PopoutConsole
                popoutParams={popoutParams}
                consoleLines={popoutState?.lines || []}
                consoleInput={popoutState?.input || ''}
                setConsoleInput={(val) => setConsoleInput(popoutParams.instanceId, val)}
                sendConsoleCommand={() => sendConsoleCommand(popoutParams.instanceId)}
                consoleEndRef={null}
                stdinDisabled={popoutState?.stdinDisabled || false}
                highlightRules={(() => {
                    const srv = servers.find((s) => s.id === popoutParams.instanceId);
                    const mod = srv && modules.find((m) => m.name === srv.module);
                    return mod?.syntax_highlight?.rules || null;
                })()}
            />
        );
    }

    // ── Discord 모달 공통 props (팝업/사이드 패널 공용) ──
    const discordModalProps = {
        isOpen: showDiscordSection,
        onClose: requestDiscordClose,
        isClosing: isDiscordClosing,
        discordBotStatus,
        discordToken,
        setDiscordToken: (val) => useDiscordStore.getState().setDiscordToken(val),
        discordPrefix,
        setDiscordPrefix: (val) => useDiscordStore.getState().update({ discordPrefix: val }),
        discordAutoStart,
        setDiscordAutoStart: (val) => useDiscordStore.getState().update({ discordAutoStart: val }),
        discordMusicEnabled,
        setDiscordMusicEnabled: (val) => useDiscordStore.getState().update({ discordMusicEnabled: val }),
        discordMusicChannelId,
        setDiscordMusicChannelId: (val) => useDiscordStore.getState().update({ discordMusicChannelId: val }),
        discordMusicUISettings,
        setDiscordMusicUISettings: (val) => useDiscordStore.getState().update({ discordMusicUISettings: val }),
        discordBotMode,
        setDiscordBotMode: (val) => useDiscordStore.getState().switchMode(val),
        discordCloudRelayUrl,
        setDiscordCloudRelayUrl: (val) => useDiscordStore.getState().update({ discordCloudRelayUrl: val }),
        discordCloudHostId,
        setDiscordCloudHostId: (val) => useDiscordStore.getState().update({ discordCloudHostId: val }),
        relayConnected,
        relayConnecting,
        handleStartDiscordBot,
        handleStopDiscordBot,
        saveCurrentSettings,
        servers,
        modules,
        moduleAliasesPerModule,
        nodeSettings,
        setNodeSettings: (valOrFn) => {
            const prev = useDiscordStore.getState().nodeSettings;
            const next = typeof valOrFn === 'function' ? valOrFn(prev) : valOrFn;
            useDiscordStore.getState().update({ nodeSettings: next });
        },
        cloudNodes,
        setCloudNodes: (val) => useDiscordStore.getState().update({ cloudNodes: val }),
        cloudMembers,
        setCloudMembers: (valOrFn) => {
            const prev = useDiscordStore.getState().cloudMembers;
            const next = typeof valOrFn === 'function' ? valOrFn(prev) : valOrFn;
            useDiscordStore.getState().update({ cloudMembers: next });
        },
    };

    return (
        <ExtensionProvider>
            <div className="App">
                {/* Discord overlay backdrop (팝업 모드에서만) */}
                {showDiscordSection && !discordSideMode && <div className="discord-backdrop" onClick={requestDiscordClose} />}
                {/* Background overlay backdrop */}
                {showBackgroundSection && <div className="discord-backdrop" onClick={requestBackgroundClose} />}
                {/* Notice overlay backdrop */}
                {showNoticeSection && <div className="discord-backdrop" onClick={requestNoticeClose} />}
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
                            {/* 익스텐션 초기화 스피너 */}
                            {extInitializing && (
                                <div
                                    className="ext-init-spinner-wrapper"
                                    title={Object.values(extInitInProgress).join(', ') || t('common:initializing', { defaultValue: 'Initializing extensions…' })}
                                >
                                    <span className="ext-init-spinner" />
                                </div>
                            )}
                            <div className="notice-button-wrapper">
                                <button
                                    className="btn-settings-icon-solo"
                                    onClick={() =>
                                        showNoticeSection ? requestNoticeClose() : setShowNoticeSection(true)
                                    }
                                    title={t('notice_modal.tooltip')}
                                >
                                    <Icon name="bell" size="lg" />
                                </button>
                                {unreadNoticeCount > 0 && (
                                    <span className="notice-badge-dot">
                                        {unreadNoticeCount > 9 ? '9+' : unreadNoticeCount}
                                    </span>
                                )}
                                <NoticeModal
                                    isOpen={showNoticeSection}
                                    onClose={requestNoticeClose}
                                    isClosing={isNoticeClosing}
                                    onOpenUpdateModal={() => {
                                        useUIStore.getState().openSettings('update');
                                    }}
                                />
                            </div>
                            <button
                                className="btn-settings-icon-solo"
                                onClick={() => useUIStore.getState().openSettings()}
                                title={t('settings.gui_settings_tooltip')}
                            >
                                <Icon name="cog" size="lg" />
                            </button>
                        </div>
                    </div>

                    {/* 두 번째 줄: 기능 버튼들 */}
                    <div className="header-row header-row-controls">
                        <button className="btn btn-add" onClick={() => setShowModuleManager(!showModuleManager)}>
                            <Icon name="plus" size="sm" /> Add Server
                        </button>
                        <div className="header-spacer"></div>
                        <div className="discord-button-wrapper">
                            <button
                                className={`btn btn-discord ${discordBotStatus === 'running' ? 'btn-discord-active' : ''}`}
                                onClick={() =>
                                    showDiscordSection ? requestDiscordClose() : setShowDiscordSection(true)
                                }
                            >
                                <span
                                    className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : discordBotStatus === 'connecting' ? 'status-connecting' : 'status-offline'}`}
                                ></span>
                                Discord Bot
                            </button>
                            {!discordSideMode && (
                                <DiscordBotModal
                                    {...discordModalProps}
                                    displayMode="popup"
                                />
                            )}
                        </div>
                        <div className="background-button-wrapper">
                            <button
                                className={`btn btn-background ${backgroundDaemonStatus === 'running' ? 'btn-background-active' : ''}`}
                                onClick={() =>
                                    showBackgroundSection ? requestBackgroundClose() : setShowBackgroundSection(true)
                                }
                            >
                                <span
                                    className={`status-indicator ${
                                        backgroundDaemonStatus === 'running'
                                            ? 'status-online'
                                            : backgroundDaemonStatus === 'checking'
                                              ? 'status-checking'
                                              : 'status-offline'
                                    }`}
                                ></span>
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

                {/* AddInstanceModal — 새 서버 / 마이그레이션 선택 */}
                <AddInstanceModal
                    isOpen={showModuleManager}
                    onClose={() => setShowModuleManager(false)}
                    extensions={modules}
                    servers={servers}
                    onRefreshextensions={fetchModules}
                    onAddServer={handleAddServer}
                />

                <div className="app-body">
                <main className={`app-main${Object.keys(consoles).length > 0 ? ' app-main-dock-active' : ''}`}>
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
                                    isConsoleOpen={isConsoleOpen}
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
                                    onContextMenu={(e) => {
                                        e.preventDefault();
                                        setContextMenu({ x: e.clientX, y: e.clientY, server });
                                    }}
                                />
                            ))
                        )}
                    </div>

                    {/* 플로팅 콘솔 윈도우들 */}
                    {Object.entries(consoles).map(([instanceId, state]) => {
                        if (consolePopoutInstanceId === instanceId) return null;
                        const srv = servers.find((s) => s.id === instanceId);
                        const mod = srv && modules.find((m) => m.name === srv.module);
                        const highlightRules = mod?.syntax_highlight?.rules || null;
                        return (
                            <ConsoleWindow
                                key={instanceId}
                                instanceId={instanceId}
                                state={state}
                                focusConsole={focusConsole}
                                minimizeConsole={minimizeConsole}
                                closeConsole={closeConsole}
                                togglePin={togglePin}
                                updatePosition={updatePosition}
                                updateSize={updateSize}
                                setConsoleInput={setConsoleInput}
                                sendConsoleCommand={sendConsoleCommand}
                                setConsolePopoutInstanceId={setConsolePopoutInstanceId}
                                highlightRules={highlightRules}
                                servers={servers}
                            />
                        );
                    })}

                    {/* 콘솔 독 (하단 작업 표시줄) */}
                    <ConsoleDock
                        consoles={consoles}
                        restoreConsole={restoreConsole}
                        focusConsole={focusConsole}
                        closeConsole={closeConsole}
                        popinConsole={popinConsole}
                        consolePopoutInstanceId={consolePopoutInstanceId}
                        servers={servers}
                        hasProgressBar={!!progressBar}
                    />
                </main>

                {/* 사이드 패널 모드 디스코드 봇 (app-main 우측 서랍) */}
                {discordSideMode && (
                    <DiscordBotModal
                        {...discordModalProps}
                        displayMode="side"
                    />
                )}
                </div>{/* /app-body */}

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
                        <div
                            className="context-menu-overlay"
                            onClick={() => setContextMenu(null)}
                            onContextMenu={(e) => {
                                e.preventDefault();
                                setContextMenu(null);
                            }}
                        />
                        <div className="context-menu" style={{ top: contextMenu.y, left: contextMenu.x }}>
                            <div
                                className="context-menu-item"
                                onClick={() => {
                                    handleOpenSettings(contextMenu.server);
                                    setContextMenu(null);
                                }}
                            >
                                <Icon name="settings" size="sm" />
                                {t('context_menu.settings', { defaultValue: 'Settings' })}
                            </div>
                            <div
                                className="context-menu-item"
                                onClick={() => {
                                    const srv = contextMenu.server;
                                    const dir = srv?.module_settings?.working_dir ||
                                        srv?.working_dir ||
                                        null;
                                    if (dir) {
                                        window.api?.shellOpenPath?.(dir)?.catch?.((err) =>
                                            console.error('[App] shellOpenPath failed:', err.message));
                                    }
                                    setContextMenu(null);
                                }}
                            >
                                <Icon name="folder" size="sm" />
                                {t('context_menu.open_folder', { defaultValue: 'Open in File Explorer' })}
                            </div>
                            <div className="context-menu-separator" />
                            <div
                                className="context-menu-item danger"
                                onClick={() => {
                                    handleDeleteServer(contextMenu.server);
                                    setContextMenu(null);
                                }}
                            >
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
                {modal && (modal.type === 'question' || modal.type === 'warning') && (
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
                    onClose={() => {
                        useUIStore.getState().closeSettings();
                    }}
                    refreshInterval={refreshInterval}
                    onRefreshIntervalChange={(val) => useSettingsStore.getState().update({ refreshInterval: val })}
                    ipcPort={ipcPort}
                    onIpcPortChange={(val) => useSettingsStore.getState().update({ ipcPort: val })}
                    consoleBufferSize={consoleBufferSize}
                    onConsoleBufferSizeChange={(val) => {
                        useSettingsStore.getState().update({ consoleBufferSize: val });
                        consoleBufferRef.current = val;
                    }}
                    discordCloudRelayUrl={discordCloudRelayUrl}
                    onDiscordCloudRelayUrlChange={(val) =>
                        useDiscordStore.getState().update({ discordCloudRelayUrl: val })
                    }
                    onTestModal={setModal}
                    onTestProgressBar={setProgressBar}
                    initialView={settingsInitialView}
                    onTestWaitingImage={() => {
                        setShowWaitingImage(true);
                        setTimeout(() => setShowWaitingImage(false), 4000);
                    }}
                    onTestLoadingScreen={() => {
                        useUIStore.getState().closeSettings();
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
