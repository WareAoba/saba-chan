import { useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast, createTranslateError, retryWithBackoff } from '../utils/helpers';

/**
 * Manages server CRUD operations: fetch, start, stop, status, add, delete.
 *
 * @param {Object} params
 * @param {Array} params.servers - Current server list
 * @param {Function} params.setServers - Server state setter
 * @param {Array} params.modules - Current module list
 * @param {boolean} params.loading - Whether initial load is in progress
 * @param {Function} params.setLoading - Loading state setter
 * @param {Function} params.setModal - Modal state setter
 * @param {Function} params.setProgressBar - Progress bar state setter
 * @param {Object|null} params.consoleServer - Currently open console server
 * @param {Function} params.openConsole - Open console for a server
 * @param {Function} params.closeConsole - Close the console panel
 * @param {Function} params.setShowModuleManager - Module manager visibility setter
 * @param {Function} params.formatUptime - Uptime formatter
 * @returns {Object} Server action handlers and fetchServers
 */
export function useServerActions({
    servers,
    setServers,
    modules,
    loading,
    setLoading,
    setModal,
    setProgressBar,
    consoleServer,
    openConsole,
    closeConsole,
    setShowModuleManager,
    formatUptime,
}) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // -- Internal refs for change detection --
    const guiInitiatedOpsRef = useRef(new Set());
    const lastErrorToastRef = useRef(0);
    const firstFetchDoneRef = useRef(false);

    // ── fetchServers ────────────────────────────────────────
    const fetchServers = async () => {
        try {
            const data = await retryWithBackoff(
                () => window.api.serverList(),
                3,
                800
            );
            if (data && data.servers) {
                setServers(prev => {
                    if (!firstFetchDoneRef.current) {
                        firstFetchDoneRef.current = true;
                        return data.servers.map(newServer => {
                            const existing = prev.find(s => s.name === newServer.name);
                            return { ...newServer, expanded: existing?.expanded || false };
                        });
                    }

                    // Detect state changes (crash / external start·stop)
                    for (const newServer of data.servers) {
                        const existing = prev.find(s => s.name === newServer.name);
                        if (!existing) continue;

                        const wasRunning = existing.status === 'running';
                        const nowStopped = newServer.status === 'stopped';
                        const nowRunning = newServer.status === 'running';
                        const wasStopped = existing.status === 'stopped';
                        const isGuiOp = guiInitiatedOpsRef.current.has(newServer.name);

                        const apiAction = newServer.last_api_action;
                        const isApiOp = apiAction && (Date.now() - apiAction < 60000);

                        if (wasRunning && nowStopped && !isGuiOp) {
                            if (isApiOp) {
                                safeShowToast(
                                    t('servers.unexpected_stop_toast', { name: newServer.name }),
                                    'info', 3000
                                );
                            } else {
                                safeShowToast(
                                    t('servers.unexpected_stop_toast', { name: newServer.name }),
                                    'error', 5000,
                                    { isNotice: true, source: newServer.name }
                                );
                            }
                        } else if (wasStopped && nowRunning && !isGuiOp) {
                            if (isApiOp) {
                                safeShowToast(
                                    t('servers.external_start_toast', { name: newServer.name }),
                                    'info', 3000
                                );
                            } else {
                                safeShowToast(
                                    t('servers.external_start_toast', { name: newServer.name }),
                                    'info', 3000,
                                    { isNotice: true, source: newServer.name }
                                );
                            }
                        }

                        if (isGuiOp && (nowStopped || nowRunning) && existing.status !== newServer.status) {
                            guiInitiatedOpsRef.current.delete(newServer.name);
                        }
                    }

                    return data.servers.map(newServer => {
                        const existing = prev.find(s => s.name === newServer.name);
                        return {
                            ...newServer,
                            expanded: existing?.expanded || false
                        };
                    });
                });
            } else if (data && data.error) {
                console.error('Server list error:', data.error);
                const now = Date.now();
                if (!loading && (now - lastErrorToastRef.current) > 5000) {
                    safeShowToast(t('servers.fetch_failed_toast', { error: translateError(data.error) }), 'warning', 3000);
                    lastErrorToastRef.current = now;
                }
            } else {
                if (loading) {
                    setServers([]);
                }
            }
        } catch (error) {
            console.error('Failed to fetch servers:', error);
            const errorMsg = translateError(error.message);
            const now = Date.now();
            if (!loading && (now - lastErrorToastRef.current) > 5000) {
                safeShowToast(t('servers.fetch_update_failed_toast', { error: errorMsg }), 'warning', 3000);
                lastErrorToastRef.current = now;
            }
        } finally {
            setLoading(false);
        }
    };

    // ── handleStart ─────────────────────────────────────────
    const handleStart = async (name, module) => {
        try {
            const srv = servers.find(s => s.name === name);
            if (!srv) {
                safeShowToast(t('servers.start_failed_toast', { error: 'Instance not found' }), 'error', 4000);
                return;
            }

            // Determine start mode
            const mod = modules.find(m => m.name === module);
            const instanceManagedStart = srv.module_settings?.managed_start;
            let interactionMode;
            if (instanceManagedStart === true) {
                interactionMode = 'console';
            } else if (instanceManagedStart === false) {
                interactionMode = 'commands';
            } else {
                interactionMode = mod?.interaction_mode || 'console';
            }

            let result;
            if (interactionMode === 'console') {
                result = await window.api.managedStart(srv.id);
            } else {
                result = await window.api.serverStart(name, { module });
            }

            // ── action_required: server jar not found ──
            if (result.action_required === 'server_jar_not_found') {
                setModal({
                    type: 'question',
                    title: t('servers.jar_not_found_title'),
                    message: result.configured_path
                        ? t('servers.jar_not_found_message_with_path', { path: result.configured_path })
                        : t('servers.jar_not_found_message'),
                    buttons: [
                        {
                            label: t('servers.jar_action_update_path'),
                            action: async () => {
                                setModal(null);
                                try {
                                    const filePath = await window.api.openFileDialog({
                                        filters: [{ name: 'JAR', extensions: ['jar'] }],
                                        title: t('servers.select_server_jar'),
                                    });
                                    if (filePath) {
                                        const s = servers.find(s => s.name === name);
                                        if (s) {
                                            await window.api.instanceUpdateSettings(s.id, { executable_path: filePath });
                                            safeShowToast(t('servers.jar_path_updated'), 'success', 3000);
                                            await fetchServers();
                                            handleStart(name, module);
                                        }
                                    }
                                } catch (err) {
                                    safeShowToast(translateError(err.message), 'error', 4000);
                                }
                            }
                        },
                        {
                            label: t('servers.jar_action_install_new'),
                            action: async () => {
                                setModal(null);
                                try {
                                    const installDir = await window.api.openFolderDialog();
                                    if (!installDir) return;

                                    setProgressBar({ message: t('servers.progress_fetching_versions'), indeterminate: true });

                                    const versions = await window.api.moduleListVersions(module, { per_page: 1 });
                                    const latestVersion = versions?.latest?.release;
                                    if (!latestVersion) {
                                        setProgressBar(null);
                                        safeShowToast(t('servers.version_fetch_failed'), 'error', 4000);
                                        return;
                                    }

                                    setProgressBar({ message: t('servers.progress_downloading', { version: latestVersion }), percent: 0 });

                                    const installResult = await window.api.moduleInstallServer(module, {
                                        version: latestVersion,
                                        install_dir: installDir,
                                        accept_eula: true,
                                    });

                                    if (installResult.error || installResult.success === false) {
                                        setProgressBar(null);
                                        safeShowToast(installResult.error || installResult.message, 'error', 4000);
                                        return;
                                    }

                                    setProgressBar({ message: t('servers.progress_configuring'), percent: 90 });

                                    const s = servers.find(s => s.name === name);
                                    if (s && installResult.jar_path) {
                                        await window.api.instanceUpdateSettings(s.id, {
                                            executable_path: installResult.jar_path,
                                            working_dir: installResult.install_path,
                                        });
                                    }

                                    setProgressBar({ message: t('servers.progress_complete'), percent: 100 });
                                    setTimeout(() => setProgressBar(null), 2000);

                                    const msg = installResult.java_warning
                                        ? `${t('servers.install_completed', { version: latestVersion })}\n⚠️ ${installResult.java_warning}`
                                        : t('servers.install_completed', { version: latestVersion });
                                    safeShowToast(msg, 'success', 5000);
                                    await fetchServers();

                                    if (!installResult.java_warning) {
                                        handleStart(name, module);
                                    }
                                } catch (err) {
                                    setProgressBar(null);
                                    safeShowToast(translateError(err.message), 'error', 4000);
                                }
                            }
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => setModal(null)
                        }
                    ]
                });
                return;
            }

            if (result.error) {
                const errorMsg = translateError(result.error);
                safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
            } else {
                guiInitiatedOpsRef.current.add(name);
                setProgressBar({ message: t('servers.starting_toast', { name }), indeterminate: true });
                if (interactionMode === 'console') {
                    openConsole(srv.id, name);
                }

                // Poll until running (max 30s)
                let attempts = 0;
                const maxAttempts = 60;
                const delay = 500;
                let resolved = false;

                const checkStatus = async () => {
                    if (resolved) return;
                    attempts++;
                    try {
                        const statusResult = await window.api.serverStatus(name);
                        if (statusResult.status === 'running') {
                            resolved = true;
                            setProgressBar(null);
                            safeShowToast(t('servers.start_completed_toast', { name }), 'success', 3000, { isNotice: true, source: name });
                            fetchServers();
                            return;
                        }
                    } catch (error) { /* ignore */ }
                    if (attempts >= maxAttempts) {
                        resolved = true;
                        setProgressBar(null);
                        safeShowToast(t('servers.start_timeout_toast', { name }), 'warning', 3000);
                        fetchServers();
                        return;
                    }
                    if (!resolved) setTimeout(checkStatus, delay);
                };
                setTimeout(checkStatus, delay);
            }
        } catch (error) {
            setProgressBar(null);
            const errorMsg = translateError(error.message);
            safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
        }
    };

    // ── handleStop ──────────────────────────────────────────
    const handleStop = async (name) => {
        setModal({
            type: 'question',
            title: t('servers.stop_confirm_title'),
            message: t('servers.stop_confirm_message', { name }),
            onConfirm: async () => {
                setModal(null);
                try {
                    const srv = servers.find(s => s.name === name);
                    const useGraceful = srv?.module_settings?.graceful_stop;
                    const forceStop = useGraceful === false;

                    const result = await window.api.serverStop(name, { force: forceStop });
                    if (result.error) {
                        const errorMsg = translateError(result.error);
                        safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
                    } else {
                        guiInitiatedOpsRef.current.add(name);
                        if (srv && consoleServer?.id === srv.id) {
                            closeConsole();
                        }
                        setProgressBar({ message: t('servers.stopping_toast', { name }), indeterminate: true });

                        // Poll until stopped (max 10s)
                        let attempts = 0;
                        const maxAttempts = 20;
                        const delay = 500;
                        let resolved = false;

                        const checkStatus = async () => {
                            if (resolved) return;
                            attempts++;
                            try {
                                const statusResult = await window.api.serverStatus(name);
                                if (statusResult.status === 'stopped') {
                                    resolved = true;
                                    setProgressBar(null);
                                    safeShowToast(t('servers.stop_completed_toast', { name }), 'success', 3000, { isNotice: true, source: name });
                                    fetchServers();
                                    return;
                                }
                            } catch (error) { /* ignore */ }
                            if (attempts >= maxAttempts) {
                                resolved = true;
                                setProgressBar(null);
                                safeShowToast(t('servers.stop_timeout_toast', { name }), 'warning', 3000);
                                fetchServers();
                                return;
                            }
                            if (!resolved) setTimeout(checkStatus, delay);
                        };
                        setTimeout(checkStatus, delay);
                    }
                } catch (error) {
                    setProgressBar(null);
                    const errorMsg = translateError(error.message);
                    safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
                }
            },
            onCancel: () => setModal(null)
        });
    };

    // ── handleStatus ────────────────────────────────────────
    const handleStatus = async (name) => {
        try {
            const result = await window.api.serverStatus(name);
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.status_check_failed_title'), message: errorMsg });
            } else {
                const uptime = result.start_time ? formatUptime(result.start_time) : 'N/A';
                const statusInfo = `Status: ${result.status}\nPID: ${result.pid || 'N/A'}\nUptime: ${uptime}`;
                setModal({ type: 'notification', title: name, message: statusInfo });
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.status_check_failed_title'), message: errorMsg });
        }
    };

    // ── handleAddServer ─────────────────────────────────────
    const handleAddServer = async (serverName, moduleName) => {
        if (!serverName || !serverName.trim()) {
            setModal({ type: 'failure', title: t('servers.add_server_name_empty_title'), message: t('servers.add_server_name_empty_message') });
            return;
        }
        if (!moduleName) {
            setModal({ type: 'failure', title: t('servers.add_module_empty_title'), message: t('servers.add_module_empty_message') });
            return;
        }

        try {
            const selectedModuleData = modules.find(m => m.name === moduleName);

            const instanceData = {
                name: serverName.trim(),
                module_name: moduleName,
                executable_path: selectedModuleData?.executable_path || null
            };

            console.log('Adding instance:', instanceData);
            const result = await window.api.instanceCreate(instanceData);

            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: errorMsg });
            } else {
                setModal({ type: 'success', title: t('command_modal.success'), message: t('server_actions.server_added', { name: serverName }) });
                setShowModuleManager(false);
                fetchServers();
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.add_error_title'), message: errorMsg });
        }
    };

    // ── handleDeleteServer ──────────────────────────────────
    const handleDeleteServer = async (server) => {
        setModal({
            type: 'question',
            title: t('server_actions.delete_confirm_title'),
            message: t('server_actions.delete_confirm_message', { name: server.name }),
            onConfirm: () => performDeleteServer(server),
        });
    };

    const performDeleteServer = async (server) => {
        setModal(null);
        try {
            const result = await window.api.instanceDelete(server.id);
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.delete_failed_title'), message: errorMsg });
            } else {
                console.log(`Instance "${server.name}" (ID: ${server.id}) deleted`);
                setModal({ type: 'success', title: t('command_modal.success'), message: t('server_actions.server_deleted', { name: server.name }) });
                fetchServers();
            }
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.delete_error_title'), message: errorMsg });
        }
    };

    return {
        fetchServers,
        handleStart,
        handleStop,
        handleStatus,
        handleAddServer,
        handleDeleteServer,
    };
}
