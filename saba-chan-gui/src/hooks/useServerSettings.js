import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createTranslateError, safeShowToast } from '../utils/helpers';
import { useModalClose } from './useModalClose';

/**
 * Manages server settings modal: open, save, install version, reset.
 * (Discord command words moved to the Discord page — components/discord/WordsView.js)
 *
 * @param {Object} params
 * @param {Array} params.servers - Current server list
 * @param {Array} params.modules - Current module list
 * @param {Function} params.setModal - Modal state setter
 * @param {Function} params.setProgressBar - Progress bar state setter
 * @param {Function} params.fetchServers - Server list refresh function
 * @returns {Object} Settings modal state and handlers
 */

/** 크래시 자동 재시작 설정 (module_settings 에 저장, 기본값은 데몬과 같다: 켜짐 / 원인 모를 종료는 꺼짐) */
const asFlag = (v, dflt) => (v === true || v === 'true' ? true : v === false || v === 'false' ? false : dflt);
const restartFlagsOf = (server) => ({
    auto_restart: asFlag(server?.module_settings?.auto_restart, true),
    restart_on_unknown_exit: asFlag(server?.module_settings?.restart_on_unknown_exit, false),
});

/** 문자열 칸 값으로 (없으면 null) */
const fieldValue = (v) => (v === undefined || v === null ? null : String(v));

/** 데몬이 인스턴스 최상위에 저장하는 공통 필드 (src/ipc/handlers/instance.rs known_fields). 이 값들은 module_settings가 낡았을 수 있다 */
const TOP_LEVEL_FIELDS = new Set([
    'port',
    'rcon_port',
    'rcon_password',
    'rest_host',
    'rest_port',
    'rest_username',
    'rest_password',
    'executable_path',
    'protocol_mode',
    'server_version',
]);

/** 저장된 값: 공통 필드는 최상위가, 나머지는 module_settings가 진짜다 */
const savedValue = (server, name) => {
    const top = fieldValue(server[name]);
    const cached = fieldValue(server.module_settings?.[name]);
    return TOP_LEVEL_FIELDS.has(name) ? (top ?? cached) : (cached ?? top);
};

/**
 * 실제 설정 값을 칸 값으로. 문자열 칸에 숫자가 오면 (모듈이 "000123"을 123으로 바꾼 경우)
 * 저장된 문자열이 같은 수를 뜻할 때 그 문자열을 지킨다.
 */
const liveFieldValue = (field, v, saved) => {
    if (typeof v === 'number' && field.field_type !== 'number' && saved !== null && Number(saved) === v) return saved;
    return fieldValue(v);
};

export function useServerSettings({ servers, modules, setModal, setProgressBar, fetchServers }) {
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
    // 실제 설정으로 확인하지 못한 칸 (저장된 캐시나 module.toml 기본값). 사용자가 고치지 않으면 저장하지 않는다
    // (그대로 저장하면 게임 안이나 파일에서 바꾼 값을 덮어쓴다)
    const unconfirmedFields = useRef(new Set());
    // 'loading' 실제 설정 읽는 중 / 'live' 실제 설정 / 'stored' 못 읽어 저장된 값을 보여 주는 중
    const [settingsSource, setSettingsSource] = useState('loading');
    const touchedFields = useRef(new Set());
    const openRequest = useRef(0);

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
                }
            }
        } catch (error) {
            console.warn('Failed to fetch latest server data:', error);
        }

        const request = ++openRequest.current;
        setSettingsServer(latestServer);
        unconfirmedFields.current = new Set();
        touchedFields.current = new Set();
        setSettingsSource('loading');
        const defaulted = new Set();

        // Initialize settings values from module schema
        // 우선순위: 실제 설정(아래에서 비동기로 덮음) > 저장된 값(savedValue) > 기본값.
        const module = modules.find((m) => m.name === latestServer.module);
        if (module && module.settings && module.settings.fields) {
            const initial = {};
            module.settings.fields.forEach((field) => {
                let value = savedValue(latestServer, field.name);
                if (value === null) {
                    value = fieldValue(field.default) ?? '';
                    defaulted.add(field.name);
                }
                unconfirmedFields.current.add(field.name);
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

            // server_version initialization — read from saved module_settings
            const savedVersion = latestServer.module_settings?.server_version || latestServer.server_version;
            if (savedVersion) {
                initial.server_version = String(savedVersion);
            }

            Object.assign(initial, restartFlagsOf(latestServer));
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
                ...restartFlagsOf(latestServer),
            });
        }

        setSettingsActiveTab('general');
        setAdvancedExpanded(false);
        setShowSettingsModal(true);

        // 디스크의 실제 설정으로 덮는다. 그사이 사용자가 고친 칸과, 다른 서버로 바꿔 연 경우는 건드리지 않는다
        if (!module?.settings?.fields) {
            setSettingsSource('live');
        } else if (!latestServer.id || !window.api.instanceReadLiveSettings) {
            setSettingsSource('stored');
        } else {
            try {
                const live = await window.api.instanceReadLiveSettings(latestServer.id);
                if (request !== openRequest.current) return;
                if (live?.success && live.settings && typeof live.settings === 'object') {
                    const patch = {};
                    for (const field of module.settings.fields) {
                        const v = liveFieldValue(
                            field,
                            live.settings[field.name],
                            savedValue(latestServer, field.name),
                        );
                        // 읽기에 성공하면 데몬이 캐시도 맞춘다: 실제 값이 없는 칸은 캐시가 확인된 값이다 (기본값만 아니면)
                        if (v !== null || !defaulted.has(field.name)) unconfirmedFields.current.delete(field.name);
                        if (v === null || touchedFields.current.has(field.name)) continue;
                        patch[field.name] = v;
                    }
                    setSettingsValues((prev) => ({ ...prev, ...patch }));
                    setSettingsSource('live');
                } else {
                    console.warn('Live settings unavailable; only edited fields will be saved:', live?.error);
                    setSettingsSource('stored');
                }
            } catch (err) {
                if (request !== openRequest.current) return;
                console.warn('Live settings unavailable; only edited fields will be saved:', err);
                setSettingsSource('stored');
            }
        }

        // Async load versions — only for download-based modules (extension install methods have no version list)
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

            // Detect installed version from server binary if not already saved
            const hasSavedVersion = latestServer.module_settings?.server_version || latestServer.server_version;
            if (!hasSavedVersion && latestServer.id) {
                try {
                    const detected = await window.api.instanceGetInstalledVersion(latestServer.id);
                    if (detected?.success && detected.version) {
                        setSettingsValues((prev) => ({
                            ...prev,
                            server_version: detected.version,
                        }));
                        // 데몬 핸들러가 감지한 버전을 자동 저장하므로,
                        // 서버 리스트를 갱신해 카드 배지에도 반영
                        fetchServers?.();
                    }
                } catch (err) {
                    console.warn('Failed to detect installed version:', err);
                }
            }
        }
    };

    // ── handleSettingChange ─────────────────────────────────
    const handleSettingChange = (fieldName, value) => {
        touchedFields.current.add(fieldName);
        setSettingsValues((prev) => ({ ...prev, [fieldName]: String(value) }));
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
                            const workDir = srv?.module_settings?.working_dir || srv?.working_dir || null;

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

                            if (installResult.error_code === 'extension_required') {
                                setProgressBar(null);
                                setVersionInstalling(false);
                                const extList = (installResult.missing_extensions || []).join(', ');
                                safeShowToast(
                                    installResult.error ||
                                        t('server_settings.extension_required_for_install', {
                                            extensions: extList,
                                            defaultValue: `Required extension(s) missing: ${extList}. Install them in Settings → Extensions first.`,
                                        }),
                                    'error',
                                    6000,
                                );
                                return;
                            }

                            if (installResult.error || installResult.success === false) {
                                setProgressBar(null);
                                safeShowToast(
                                    translateError(installResult.error || installResult.message),
                                    'error',
                                    4000,
                                );
                                setVersionInstalling(false);
                                return;
                            }

                            if (installResult.jar_path) {
                                // working_dir 설정 — 실행 파일 경로는 모듈에서 자동 해석됨
                                const installDir =
                                    installResult.install_path || installResult.jar_path.replace(/[/\\][^/\\]+$/, '');
                                await window.api.instanceUpdateSettings(settingsServer.id, {
                                    working_dir: installDir,
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
                                        error: translateError(result.error),
                                        defaultValue: 'Reset failed',
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
                                    error: translateError(err.message),
                                    defaultValue: 'Reset error',
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
            const module = modules.find((m) => m.name === settingsServer.module);
            const convertedSettings = {};

            if (module && module.settings && module.settings.fields) {
                module.settings.fields.forEach((field) => {
                    const value = settingsValues[field.name];
                    if (unconfirmedFields.current.has(field.name) && !touchedFields.current.has(field.name)) return;

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

            // 크래시 자동 재시작 (데몬이 module_settings 에서 읽는다)
            for (const key of ['auto_restart', 'restart_on_unknown_exit']) {
                if (typeof settingsValues[key] === 'boolean') convertedSettings[key] = settingsValues[key];
            }

            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];

            // 모듈이 정한 프로토콜만 쓴다 (RCON은 데몬에서 빠졌다: 예전에 저장된 'rcon'도 여기서 바뀐다)
            if (supportedProtocols.length > 0) {
                convertedSettings.protocol_mode = protocols.default || supportedProtocols[0];
            } else {
                convertedSettings.protocol_mode = settingsValues.protocol_mode || 'auto';
            }

            // 익스텐션 데이터 패스스루 — 익스텐션 탭 컴포넌트가 변경한 값을 그대로 전달
            if (settingsValues._extension_data) {
                convertedSettings.extension_data = { ...settingsValues._extension_data };
            }

            const request = openRequest.current;
            const result = await window.api.instanceUpdateSettings(settingsServer.id, convertedSettings);
            // 저장하는 사이 다른 서버 설정을 열었다면 그 창을 닫지 않는다
            const stillOpen = request === openRequest.current;

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
                if (stillOpen) setShowSettingsModal(false);
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

    return {
        // Settings modal state
        showSettingsModal,
        settingsServer,
        settingsValues,
        settingsSource,
        settingsActiveTab,
        setSettingsActiveTab,
        advancedExpanded,
        setAdvancedExpanded,
        availableVersions,
        versionsLoading,
        versionInstalling,
        resettingServer,
        // Close animation
        isSettingsClosing,
        requestSettingsClose,
        // Handlers
        handleOpenSettings,
        handleSettingChange,
        handleInstallVersion,
        handleResetServer,
        handleSaveSettings,
    };
}
