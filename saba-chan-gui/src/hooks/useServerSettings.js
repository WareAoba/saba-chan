import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createTranslateError, safeShowToast } from '../utils/helpers';
import { useModalClose } from './useModalClose';

/**
 * Manages server settings modal: open, save, install version, reset.
 *
 * 디스코드 별명 편집은 인스턴스 설정에서 제거되었다 — 디스코드 모달의
 * "서버별 설정" 팝업(InstanceSettingsModal)이 인스턴스 단위로 관리한다.
 *
 * @param {Object} params
 * @param {Array} params.servers - Current server list
 * @param {Array} params.modules - Current module list
 * @param {Function} params.setModal - Modal state setter
 * @param {Function} params.setProgressBar - Progress bar state setter
 * @param {Function} params.fetchServers - Server list refresh function
 * @returns {Object} Settings modal state and handlers
 */
export function useServerSettings({ servers, modules, setModal, setProgressBar, fetchServers }) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // ── Settings modal state ────────────────────────────────
    const [showSettingsModal, setShowSettingsModal] = useState(false);
    const [settingsServer, setSettingsServer] = useState(null);
    const [settingsValues, setSettingsValues] = useState({});
    const [settingsActiveTab, setSettingsActiveTab] = useState('general');
    const [advancedExpanded, setAdvancedExpanded] = useState(false);
    const [communicationExpanded, setCommunicationExpanded] = useState(false);
    const [availableVersions, setAvailableVersions] = useState([]);
    const [versionsLoading, setVersionsLoading] = useState(false);
    const [versionInstalling, setVersionInstalling] = useState(false);
    const [resettingServer, setResettingServer] = useState(false);

    // ── 라이브 설정 읽기 상태 ───────────────────────────────
    // 'idle' | 'loading' | 'ok' | 'empty' | 'failed'
    // 'ok'가 아니면 화면값이 실제 설정 파일과 다를 수 있으므로 저장을 제한한다.
    // (과거: 읽기 실패를 무음 폴백해 스키마 기본값으로 실제 파일을 덮어쓰는 사고 발생)
    const [liveSettingsStatus, setLiveSettingsStatus] = useState('idle');
    // 디스크에서 실제로 읽어온 필드명 집합 — 이 값들만 "전체 저장"에 포함해도 안전
    const liveKeysRef = useRef(new Set());
    // 사용자가 모달에서 직접 변경한 필드명 집합 — diff 저장의 기준
    const touchedFieldsRef = useRef(new Set());
    const settingsRequestRef = useRef(0);

    useEffect(() => () => { settingsRequestRef.current += 1; }, []);

    // ── Close animation ─────────────────────────────────────
    const closeSettingsModal = useCallback(() => {
        settingsRequestRef.current += 1;
        setShowSettingsModal(false);
    }, []);
    const { isClosing: isSettingsClosing, requestClose: requestSettingsClose } = useModalClose(closeSettingsModal);

    // ── handleOpenSettings ──────────────────────────────────
    const handleOpenSettings = async (server) => {
        const requestId = ++settingsRequestRef.current;
        const isCurrentRequest = () => settingsRequestRef.current === requestId;
        // 모달을 새로 열 때마다 라이브/변경 추적 상태 초기화
        liveKeysRef.current = new Set();
        touchedFieldsRef.current = new Set();
        setLiveSettingsStatus('loading');
        setVersionsLoading(false);
        setVersionInstalling(false);
        setResettingServer(false);

        // Fetch latest server data from API
        let latestServer = server;
        try {
            const data = await window.api.serverList();
            if (data?.servers) {
                const found = data.servers.find((s) => s.id === server.id);
                if (found) {
                    latestServer = found;
                }
            }
        } catch (error) {
            console.warn('Failed to fetch latest server data:', error);
        }
        if (!isCurrentRequest()) return;

        setSettingsServer(latestServer);

        // Initialize settings values from module schema
        const module = modules.find((m) => m.name === latestServer.module);
        if (module?.settings?.fields) {
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

            // server_version initialization — read from saved module_settings
            const savedVersion = latestServer.module_settings?.server_version || latestServer.server_version;
            if (savedVersion) {
                initial.server_version = String(savedVersion);
            }

            setSettingsValues(initial);

            // ── 실제 설정 파일 값으로 새로고침 ──
            // `initial`은 module_settings 캐시(마지막 저장/마이그레이션 임포트 시점의 스냅샷)로
            // 채워져 있어, 사용자가 게임 내에서 또는 설정 파일을 직접 수정하면 실제 값과
            // 어긋난다. 모듈의 import_settings로 디스크의 실제 값을 다시 읽어 캐시 위에
            // 덮어써서 화면에 항상 "진짜" 현재 값을 보여준다. 이렇게 해야 저장(전체 필드
            // 전송) 시 사용자가 건드리지 않은 필드까지 오래된 캐시 값으로 실제 파일을
            // 덮어쓰는 사고도 함께 방지된다.
            if (latestServer?.id) {
                try {
                    if (typeof window.api.instanceReadLiveSettings !== 'function') {
                        throw new Error('instanceReadLiveSettings API unavailable (outdated preload)');
                    }
                    const live = await window.api.instanceReadLiveSettings(latestServer.id);
                    if (!isCurrentRequest()) return;
                    if (live?.success && live.settings && Object.keys(live.settings).length > 0) {
                        liveKeysRef.current = new Set(Object.keys(live.settings));
                        setSettingsValues((prev) => {
                            const merged = { ...prev };
                            for (const [key, value] of Object.entries(live.settings)) {
                                if (value !== undefined && value !== null) {
                                    merged[key] = String(value);
                                }
                            }
                            return merged;
                        });
                        setLiveSettingsStatus('ok');
                    } else if (live?.success) {
                        // 파일이 없거나 파싱 결과가 비어있음 — 화면값은 캐시/기본값이므로
                        // 전체 저장 시 실제 파일을 덮어쓸 위험이 있다.
                        setLiveSettingsStatus('empty');
                    } else {
                        console.warn('Live settings read reported failure:', live?.error || live?.message);
                        setLiveSettingsStatus('failed');
                    }
                } catch (err) {
                    if (!isCurrentRequest()) return;
                    console.warn('Failed to load live settings from disk:', err);
                    setLiveSettingsStatus('failed');
                }
            } else {
                setLiveSettingsStatus('failed');
            }
        } else {
            // 스키마 필드가 없는 모듈 — INI 동기화 대상 필드가 없으므로 라이브 읽기 불필요.
            // 'ok'로 두어 protocol_mode 등 메타 설정 저장이 차단되지 않게 한다.
            setLiveSettingsStatus('ok');
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
                if (!isCurrentRequest()) return;
                if (versions?.versions) {
                    setAvailableVersions(versions.versions);
                }
            } catch (err) {
                console.warn('Failed to load versions:', err);
            } finally {
                if (isCurrentRequest()) setVersionsLoading(false);
            }
            if (!isCurrentRequest()) return;

            // Detect installed version from server binary if not already saved
            const hasSavedVersion = latestServer.module_settings?.server_version || latestServer.server_version;
            if (!hasSavedVersion && latestServer.id) {
                try {
                    const detected = await window.api.instanceGetInstalledVersion(latestServer.id);
                    if (!isCurrentRequest()) return;
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
        touchedFieldsRef.current.add(fieldName);
        setSettingsValues((prev) => ({ ...prev, [fieldName]: String(value) }));
    };

    // ── handleInstallVersion ────────────────────────────────
    const handleInstallVersion = async () => {
        if (!settingsServer || !settingsValues.server_version) return;
        const requestId = settingsRequestRef.current;
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
                        if (requestId !== settingsRequestRef.current) return;
                        setModal(null);
                        setVersionInstalling(true);
                        setProgressBar({ message: t('servers.progress_downloading', { version }), percent: 0 });
                        try {
                            const srv = servers.find((s) => s.id === settingsServer.id);
                            const workDir = srv?.module_settings?.working_dir || srv?.working_dir || null;

                            let targetDir;
                            if (!workDir) {
                                const installDir = await window.api.openFolderDialog();
                                if (requestId !== settingsRequestRef.current) return;
                                if (!installDir) {
                                    setProgressBar(null);
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
                                if (requestId === settingsRequestRef.current) handleSettingChange('server_version', version);
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
                            if (requestId === settingsRequestRef.current) setVersionInstalling(false);
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
        const requestId = settingsRequestRef.current;

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
                        if (requestId !== settingsRequestRef.current) return;
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
                                        if (requestId !== settingsRequestRef.current) return;
                                        if (serverList?.servers && !serverList.error) {
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
                            if (requestId === settingsRequestRef.current) setResettingServer(false);
                        }
                    },
                },
            ],
        });
    };

    // ── handleSaveSettings ──────────────────────────────────
    // diff 기반 저장: 디스크에서 실제로 읽어온(liveKeys) 필드 + 사용자가 직접
    // 변경한(touched) 필드만 전송한다. 화면에만 존재하는 값(스키마 기본값,
    // 오래된 캐시)은 전송하지 않는다 — 전송된 필드는 데몬이 실제 게임 설정
    // 파일에 그대로 쓰기 때문에, 무분별한 전체 전송은 파일 덮어쓰기 사고가 된다.
    const handleSaveSettings = async () => {
        if (!settingsServer) return;
        const requestId = settingsRequestRef.current;

        try {

            const module = modules.find((m) => m.name === settingsServer.module);
            const convertedSettings = {};
            const touched = touchedFieldsRef.current;
            const liveKeys = liveKeysRef.current;

            if (module?.settings?.fields) {
                module.settings.fields.forEach((field) => {
                    if (!touched.has(field.name) && !liveKeys.has(field.name)) {
                        return;
                    }
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

            if (settingsValues.server_version && touched.has('server_version')) {
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

            // ── 라이브 읽기 실패 가드 ──
            // 실제 파일을 읽지 못한 상태에서는 화면값을 신뢰할 수 없다.
            // 변경분이 없으면 저장 자체를 차단하고, 변경분이 있으면 "변경한
            // 필드만 저장됨"을 사용자에게 확인받은 뒤 진행한다.
            if (liveSettingsStatus !== 'ok') {
                const changedCount = touched.size;
                if (changedCount === 0) {
                    setModal({
                        type: 'failure',
                        title: t('server_settings.live_read_failed_title', {
                            defaultValue: 'Server Settings File Not Loaded',
                        }),
                        message: t('server_settings.live_read_failed_block', {
                            defaultValue:
                                'The actual server configuration file could not be read, so the displayed values may not match the real settings.\nSaving is blocked because it could overwrite the file with wrong values.\nClose this window and try again after resolving the issue.',
                        }),
                    });
                    return;
                }
                setModal({
                    type: 'question',
                    title: t('server_settings.live_read_failed_confirm_title', {
                        defaultValue: 'Save Changed Fields Only?',
                    }),
                    message: t('server_settings.live_read_failed_confirm', {
                        count: changedCount,
                        defaultValue:
                            'The actual server configuration file could not be read, so the displayed values may not match the real settings.\nOnly the {{count}} field(s) you changed in this window will be saved. All other settings will be left untouched.',
                    }),
                    buttons: [
                        {
                            label: t('server_settings.save_changed_only', { defaultValue: 'Save changes' }),
                            action: async () => {
                                setModal(null);
                                await submitSettings(convertedSettings, requestId);
                            },
                        },
                        {
                            label: t('modals.cancel', { defaultValue: 'Cancel' }),
                            action: () => setModal(null),
                        },
                    ],
                });
                return;
            }

            await submitSettings(convertedSettings, requestId);
        } catch (error) {
            console.error('Exception in handleSaveSettings:', error);
            setModal({
                type: 'failure',
                title: t('settings.save_error_title'),
                message: translateError(error.message),
            });
        }
    };

    // 실제 저장 수행 — handleSaveSettings에서 payload 확정 후 호출
    const submitSettings = async (convertedSettings, requestId) => {
        if (requestId !== settingsRequestRef.current) return;
        try {
            const result = await window.api.instanceUpdateSettings(settingsServer.id, convertedSettings);
            if (requestId !== settingsRequestRef.current) return;

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
            } else if (result.properties_warning) {
                // 인스턴스 설정 자체는 저장됐지만, 실제 게임 서버 설정 파일 쓰기는 실패한 경우.
                // 조용히 success로 넘어가면 사용자는 적용됐다고 믿고 넘어가므로 반드시 알려야 한다.
                setModal({
                    type: 'failure',
                    title: t('settings.save_partial_title', { defaultValue: 'Partially Saved' }),
                    message: t('settings.save_partial_message', {
                        error: translateError(result.properties_warning),
                        defaultValue: `Settings were saved in saba-chan, but writing to the actual server config file failed:\n${result.properties_warning}`,
                    }),
                });
                touchedFieldsRef.current = new Set();
                fetchServers();
            } else {
                setModal({
                    type: 'success',
                    title: t('command_modal.success'),
                    message: t('server_actions.settings_saved', { name: settingsServer.name }),
                });
                touchedFieldsRef.current = new Set();
                setShowSettingsModal(false);
                fetchServers();
            }
        } catch (error) {
            if (requestId !== settingsRequestRef.current) return;
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
        settingsActiveTab,
        setSettingsActiveTab,
        advancedExpanded,
        setAdvancedExpanded,
        communicationExpanded,
        setCommunicationExpanded,
        availableVersions,
        versionsLoading,
        versionInstalling,
        resettingServer,
        // 라이브 읽기 상태 — 'ok'가 아니면 모달이 경고 배너를 표시한다
        liveSettingsStatus,
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
