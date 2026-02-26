import { useCallback, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createTranslateError, safeShowToast } from '../utils/helpers';
import { useModalClose } from './useModalClose';

/**
 * Manages server settings modal: open, save, install version, reset,
 * and alias editing for Discord bot integration.
 *
 * @param {Object} params
 * @param {Array} params.servers - Current server list
 * @param {Array} params.modules - Current module list
 * @param {Function} params.setModal - Modal state setter
 * @param {Function} params.setProgressBar - Progress bar state setter
 * @param {Object} params.moduleAliasesPerModule - Per-module alias definitions from module.toml
 * @param {Object} params.discordModuleAliases - Saved user module aliases
 * @param {Object} params.discordCommandAliases - Saved user command aliases
 * @param {Function} params.setDiscordModuleAliases - Module aliases state setter
 * @param {Function} params.setDiscordCommandAliases - Command aliases state setter
 * @param {string} params.discordPrefix - Bot command prefix
 * @param {Function} params.fetchServers - Server list refresh function
 * @returns {Object} Settings modal state and handlers
 */
export function useServerSettings({
    servers,
    modules,
    setModal,
    setProgressBar,
    moduleAliasesPerModule,
    discordModuleAliases,
    discordCommandAliases,
    setDiscordModuleAliases,
    setDiscordCommandAliases,
    discordPrefix,
    fetchServers,
}) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // ── Settings modal state ────────────────────────────────
    const [showSettingsModal, setShowSettingsModal] = useState(false);
    const [settingsServer, setSettingsServer] = useState(null);
    const [settingsValues, setSettingsValues] = useState({});
    const [settingsActiveTab, setSettingsActiveTab] = useState('general');
    const [advancedExpanded, setAdvancedExpanded] = useState(false);
    const [availableVersions, setAvailableVersions] = useState([]);
    const [versionsLoading, setVersionsLoading] = useState(false);
    const [versionInstalling, setVersionInstalling] = useState(false);
    const [resettingServer, setResettingServer] = useState(false);

    // ── Alias editing state ─────────────────────────────────
    const [editingModuleAliases, setEditingModuleAliases] = useState({});
    const [editingCommandAliases, setEditingCommandAliases] = useState({});

    // ── Close animation ─────────────────────────────────────
    const closeSettingsModal = useCallback(() => setShowSettingsModal(false), []);
    const { isClosing: isSettingsClosing, requestClose: requestSettingsClose } = useModalClose(closeSettingsModal);

    // ── handleOpenSettings ──────────────────────────────────
    const handleOpenSettings = async (server) => {
        // Fetch latest server data from API
        let latestServer = server;
        try {
            const data = await window.api.serverList();
            if (data && data.servers) {
                const found = data.servers.find((s) => s.id === server.id);
                if (found) {
                    latestServer = found;
                    console.log('Loaded latest server data:', latestServer);
                }
            }
        } catch (error) {
            console.warn('Failed to fetch latest server data:', error);
        }

        setSettingsServer(latestServer);

        // Initialize settings values from module schema
        const module = modules.find((m) => m.name === latestServer.module);
        if (module && module.settings && module.settings.fields) {
            const initial = {};
            module.settings.fields.forEach((field) => {
                let value = '';
                if (latestServer[field.name] !== undefined && latestServer[field.name] !== null) {
                    value = String(latestServer[field.name]);
                } else if (
                    latestServer.module_settings &&
                    latestServer.module_settings[field.name] !== undefined &&
                    latestServer.module_settings[field.name] !== null
                ) {
                    value = String(latestServer.module_settings[field.name]);
                } else if (field.default !== undefined && field.default !== null) {
                    value = String(field.default);
                }
                initial[field.name] = value;
            });

            // protocol_mode initialization
            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];
            if (
                (latestServer.protocol_mode &&
                    latestServer.protocol_mode !== 'auto' &&
                    latestServer.protocol_mode !== 'rest') ||
                (latestServer.protocol_mode === 'rest' && supportedProtocols.includes('rest'))
            ) {
                initial.protocol_mode = latestServer.protocol_mode;
            } else if (protocols.default) {
                initial.protocol_mode = protocols.default;
            } else if (supportedProtocols.length > 0) {
                initial.protocol_mode = supportedProtocols[0];
            } else {
                initial.protocol_mode = latestServer.protocol_mode || 'auto';
            }

            // 익스텐션 데이터 패스스루 — 익스텐션 탭 컴포넌트가 자체 관리
            if (latestServer.extension_data) {
                initial._extension_data = { ...latestServer.extension_data };
            }

            setSettingsValues(initial);
        } else {
            const protocols = module?.protocols || {};
            const defaultProto = protocols.default || (protocols.supported?.length > 0 ? protocols.supported[0] : null);
            setSettingsValues({
                protocol_mode:
                    latestServer.protocol_mode &&
                    latestServer.protocol_mode !== 'auto' &&
                    latestServer.protocol_mode !== 'rest'
                        ? latestServer.protocol_mode
                        : defaultProto || latestServer.protocol_mode || 'auto',
                ...(latestServer.extension_data
                    ? {
                          _extension_data: { ...latestServer.extension_data },
                      }
                    : {}),
            });
        }

        // Load aliases
        const moduleName = latestServer.module;
        if (moduleAliasesPerModule[moduleName]) {
            const aliases = moduleAliasesPerModule[moduleName];

            if (moduleName in discordModuleAliases) {
                const saved = discordModuleAliases[moduleName] || '';
                const parsed = saved
                    .split(',')
                    .map((a) => a.trim())
                    .filter((a) => a.length > 0);
                setEditingModuleAliases(parsed);
            } else {
                setEditingModuleAliases(aliases.module_aliases || []);
            }

            const cmdAliases = aliases.commands || {};
            const normalized = {};
            for (const [cmd, data] of Object.entries(cmdAliases)) {
                let baseAliases = [];
                if (Array.isArray(data)) {
                    baseAliases = data;
                } else if (data.aliases) {
                    baseAliases = data.aliases;
                }

                const hasSavedCmd = discordCommandAliases[moduleName] && cmd in discordCommandAliases[moduleName];
                const merged = hasSavedCmd
                    ? (discordCommandAliases[moduleName][cmd] || '')
                          .split(',')
                          .map((a) => a.trim())
                          .filter((a) => a.length > 0)
                    : baseAliases;

                normalized[cmd] = {
                    aliases: merged,
                    description: (data && data.description) || '',
                    label: (data && data.label) || cmd,
                };
            }
            setEditingCommandAliases(normalized);
        }

        setSettingsActiveTab('general');
        setAdvancedExpanded(false);
        setShowSettingsModal(true);

        // Async load versions — only for download-based modules (skip SteamCMD modules)
        const mod = modules.find((m) => m.name === latestServer.module);
        const installMethod = mod?.install?.method;
        setAvailableVersions([]);
        if (installMethod === 'download') {
            setVersionsLoading(true);
            try {
                const versions = await window.api.moduleListVersions(latestServer.module, { per_page: 30 });
                if (versions && versions.versions) {
                    setAvailableVersions(versions.versions);
                }
            } catch (err) {
                console.warn('Failed to load versions:', err);
            } finally {
                setVersionsLoading(false);
            }
        }
    };

    // ── handleSettingChange ─────────────────────────────────
    const handleSettingChange = (fieldName, value) => {
        console.log(`Setting ${fieldName} changed to:`, value);
        setSettingsValues((prev) => {
            const updated = { ...prev, [fieldName]: String(value) };
            console.log('Updated settings values:', updated);
            return updated;
        });
    };

    // ── handleInstallVersion ────────────────────────────────
    const handleInstallVersion = async () => {
        if (!settingsServer || !settingsValues.server_version) return;
        const version = settingsValues.server_version;
        const serverName = settingsServer.name;
        const module = settingsServer.module;

        setModal({
            type: 'question',
            title: t('server_settings.install_version_confirm_title', { defaultValue: 'Install Server Version' }),
            message: t('server_settings.install_version_confirm', {
                version,
                name: serverName,
                defaultValue: `Install Minecraft ${version} for server '${serverName}'?\n\nThis will download and replace the server JAR file.`,
            }),
            buttons: [
                {
                    label: t('server_settings.install_version_button', { defaultValue: 'Install' }),
                    action: async () => {
                        setModal(null);
                        setVersionInstalling(true);
                        setProgressBar({ message: t('servers.progress_downloading', { version }), percent: 0 });
                        try {
                            const srv = servers.find((s) => s.id === settingsServer.id);
                            const workDir =
                                srv?.module_settings?.working_dir ||
                                (srv?.executable_path ? srv.executable_path.replace(/[/\\][^/\\]+$/, '') : null);

                            let targetDir;
                            if (!workDir) {
                                const installDir = await window.api.openFolderDialog();
                                if (!installDir) {
                                    setProgressBar(null);
                                    setVersionInstalling(false);
                                    return;
                                }
                                targetDir = installDir;
                            } else {
                                targetDir = workDir;
                            }

                            const installResult = await window.api.moduleInstallServer(module, {
                                version,
                                install_dir: targetDir,
                                accept_eula: true,
                            });

                            if (installResult.error || installResult.success === false) {
                                setProgressBar(null);
                                safeShowToast(installResult.error || installResult.message, 'error', 4000);
                                setVersionInstalling(false);
                                return;
                            }

                            if (installResult.jar_path) {
                                await window.api.instanceUpdateSettings(settingsServer.id, {
                                    executable_path: installResult.jar_path,
                                    server_version: version,
                                });
                                handleSettingChange('server_version', version);
                            }

                            setProgressBar({ message: t('servers.progress_complete'), percent: 100 });
                            setTimeout(() => setProgressBar(null), 2000);

                            const msg = installResult.java_warning
                                ? `${t('servers.install_completed', { version })}\n⚠️ ${installResult.java_warning}`
                                : t('servers.install_completed', { version });
                            safeShowToast(msg, 'success', 5000);
                            await fetchServers();
                        } catch (err) {
                            setProgressBar(null);
                            safeShowToast(translateError(err.message), 'error', 4000);
                        } finally {
                            setVersionInstalling(false);
                        }
                    },
                },
                {
                    label: t('modals.cancel'),
                    action: () => setModal(null),
                },
            ],
        });
    };

    // ── handleResetServer ───────────────────────────────────
    const handleResetServer = async () => {
        if (!settingsServer) return;

        setModal({
            type: 'question',
            title: t('server_settings.reset_confirm_title', { defaultValue: 'Reset Server' }),
            message: t('server_settings.reset_confirm', {
                name: settingsServer.name,
                defaultValue: `Completely reset server '${settingsServer.name}'?\n\nThis will delete all worlds, settings, logs, and other data.\nOnly the server JAR and eula.txt will be kept.\n\nThis action cannot be undone.`,
            }),
            buttons: [
                {
                    label: t('modals.cancel', { defaultValue: 'Cancel' }),
                    action: () => setModal(null),
                },
                {
                    label: t('server_settings.reset_button', { defaultValue: 'Reset' }),
                    action: async () => {
                        setModal(null);
                        setResettingServer(true);
                        try {
                            const result = await window.api.instanceResetServer(settingsServer.id);
                            if (result?.error) {
                                safeShowToast(
                                    t('server_settings.reset_failed', {
                                        error: result.error,
                                        defaultValue: `Reset failed: ${result.error}`,
                                    }),
                                    'error',
                                );
                            } else {
                                const deletedCount = result?.deleted?.length || 0;
                                safeShowToast(
                                    t('server_settings.reset_success', {
                                        name: settingsServer.name,
                                        count: deletedCount,
                                        defaultValue: `Server '${settingsServer.name}' has been reset (${deletedCount} items deleted)`,
                                    }),
                                    'success',
                                );
                                // Refresh settings display
                                if (settingsServer?.id) {
                                    try {
                                        const serverList = await window.api.serverList();
                                        if (serverList && serverList.servers && !serverList.error) {
                                            const updated = serverList.servers.find((s) => s.id === settingsServer.id);
                                            if (updated) {
                                                setSettingsServer(updated);
                                                setSettingsValues(updated.module_settings || {});
                                            }
                                        }
                                    } catch (_e) {
                                        /* silent */
                                    }
                                }
                            }
                        } catch (err) {
                            safeShowToast(
                                t('server_settings.reset_error', {
                                    error: err.message,
                                    defaultValue: `Reset error: ${err.message}`,
                                }),
                                'error',
                            );
                        } finally {
                            setResettingServer(false);
                        }
                    },
                },
            ],
        });
    };

    // ── handleSaveSettings ──────────────────────────────────
    const handleSaveSettings = async () => {
        if (!settingsServer) return;

        try {
            console.log('Saving settings for', settingsServer.name, settingsValues);

            const module = modules.find((m) => m.name === settingsServer.module);
            const convertedSettings = {};

            if (module && module.settings && module.settings.fields) {
                module.settings.fields.forEach((field) => {
                    const value = settingsValues[field.name];

                    if (field.field_type === 'boolean') {
                        convertedSettings[field.name] = value === true || value === 'true';
                        return;
                    }

                    if (value === '' || value === null || value === undefined) {
                        return;
                    }

                    if (field.field_type === 'number') {
                        convertedSettings[field.name] = Number(value);
                    } else {
                        convertedSettings[field.name] = value;
                    }
                });
            }

            if (settingsValues.server_version) {
                convertedSettings.server_version = settingsValues.server_version;
            }

            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];

            if (supportedProtocols.length > 0) {
                if (supportedProtocols.includes('rest') && supportedProtocols.includes('rcon')) {
                    convertedSettings.protocol_mode =
                        settingsValues.protocol_mode || protocols.default || supportedProtocols[0];
                } else {
                    convertedSettings.protocol_mode = protocols.default || supportedProtocols[0];
                }
            } else {
                convertedSettings.protocol_mode = settingsValues.protocol_mode || 'auto';
            }

            // 익스텐션 데이터 패스스루 — 익스텐션 탭 컴포넌트가 변경한 값을 그대로 전달
            if (settingsValues._extension_data) {
                convertedSettings.extension_data = { ...settingsValues._extension_data };
            }

            console.log('Converted settings:', convertedSettings);
            const result = await window.api.instanceUpdateSettings(settingsServer.id, convertedSettings);
            console.log('API Response:', result);

            if (result.error) {
                // validation_failed: 상세 에러 목록 포함
                if (result.error_code === 'validation_failed' && result.details) {
                    const detailStr = result.details.join('\n');
                    setModal({ type: 'failure', title: t('settings.save_failed_title'), message: detailStr });
                } else {
                    setModal({
                        type: 'failure',
                        title: t('settings.save_failed_title'),
                        message: translateError(result.error),
                    });
                }
                console.error('Error response:', result.error);
            } else {
                setModal({
                    type: 'success',
                    title: t('command_modal.success'),
                    message: t('server_actions.settings_saved', { name: settingsServer.name }),
                });
                setShowSettingsModal(false);
                fetchServers();
            }
        } catch (error) {
            console.error('Exception in handleSaveSettings:', error);
            setModal({
                type: 'failure',
                title: t('settings.save_error_title'),
                message: translateError(error.message),
            });
        }
    };

    // ── Alias save/reset for settings modal ─────────────────
    const handleSaveAliasesForModule = async (moduleName) => {
        try {
            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };

            moduleAliases[moduleName] = (editingModuleAliases || []).join(',');

            const cmdMap = {};
            Object.entries(editingCommandAliases || {}).forEach(([cmd, data]) => {
                cmdMap[cmd] = (data.aliases || []).join(',');
            });
            commandAliases[moduleName] = cmdMap;

            const payload = {
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
            };

            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                setModal({
                    type: 'failure',
                    title: t('settings.aliases_save_failed_title'),
                    message: translateError(res.error),
                });
            } else {
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({
                    type: 'success',
                    title: t('server_actions.aliases_saved'),
                    message: t('server_actions.aliases_saved'),
                });
            }
        } catch (error) {
            console.error('Failed to save aliases:', error);
            setModal({
                type: 'failure',
                title: t('settings.aliases_save_error_title'),
                message: translateError(error.message),
            });
        }
    };

    const handleResetAliasesForModule = async (moduleName) => {
        try {
            setEditingModuleAliases([]);
            const clearedCmds = {};
            const defaults = moduleAliasesPerModule[moduleName];
            if (defaults && defaults.commands) {
                for (const [cmd, data] of Object.entries(defaults.commands)) {
                    clearedCmds[cmd] = { aliases: [], description: data.description || '', label: data.label || cmd };
                }
            }
            setEditingCommandAliases(clearedCmds);

            const current = await window.api.botConfigLoad();
            const moduleAliases = { ...(current.moduleAliases || {}) };
            const commandAliases = { ...(current.commandAliases || {}) };
            delete moduleAliases[moduleName];
            delete commandAliases[moduleName];

            const payload = {
                prefix: current.prefix || discordPrefix || '!saba',
                moduleAliases,
                commandAliases,
            };

            const res = await window.api.botConfigSave(payload);
            if (res.error) {
                setModal({
                    type: 'failure',
                    title: t('settings.aliases_reset_failed_title'),
                    message: translateError(res.error),
                });
            } else {
                const saved = await window.api.botConfigLoad();
                setDiscordModuleAliases(saved.moduleAliases || {});
                setDiscordCommandAliases(saved.commandAliases || {});
                setModal({
                    type: 'success',
                    title: t('settings.aliases_reset_completed_title'),
                    message: t('settings.aliases_reset_message'),
                });
            }
        } catch (error) {
            console.error('Failed to reset aliases:', error);
            setModal({
                type: 'failure',
                title: t('settings.aliases_reset_failed_title'),
                message: translateError(error.message),
            });
        }
    };

    return {
        // Settings modal state
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
        // Alias editing state
        editingModuleAliases,
        setEditingModuleAliases,
        editingCommandAliases,
        setEditingCommandAliases,
        // Close animation
        isSettingsClosing,
        requestSettingsClose,
        // Handlers
        handleOpenSettings,
        handleSettingChange,
        handleInstallVersion,
        handleResetServer,
        handleSaveSettings,
        handleSaveAliasesForModule,
        handleResetAliasesForModule,
    };
}
