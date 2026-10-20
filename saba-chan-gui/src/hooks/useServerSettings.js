import { useCallback, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { launchDraft } from '../components/servers/LaunchPreview';
import { busyMessage, createTranslateError, moduleSupports, safeShowToast } from '../utils/helpers';
import { gameNameOf } from '../utils/serverView';
import { listItems } from '../utils/validation';
import { blockedText } from '../utils/trustNotice';
import { useModalClose } from './useModalClose';

/**
 * Manages server settings modal: open, save, install version, reset.
 * (Discord command words moved to the Discord page — components/discord/InstanceWordsTab.js)
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

/** 램 상한 칸 값 (MiB, 비면 제한 없음): 데몬 인스턴스 최상위 memory_limit_mb */
const memoryLimitOf = (server) => (server?.memory_limit_mb == null ? '' : String(server.memory_limit_mb));

/** 정지 대기 칸 값 (초, 비면 모듈 값): 데몬 MK14 module_settings.stop_timeout */
const stopTimeoutOf = (server) =>
    server?.module_settings?.stop_timeout == null ? '' : String(server.module_settings.stop_timeout);

/** 칸 값으로 (없으면 null). list 설정의 문자열 배열은 배열 그대로 */
const fieldValue = (v) => (v === undefined || v === null ? null : Array.isArray(v) ? v.map(String) : String(v));

/** 데몬이 인스턴스 최상위에 저장하는 공통 필드 (src/ipc/handlers/instance.rs known_fields). 이 값들은 module_settings가 낡았을 수 있다 */
const TOP_LEVEL_FIELDS = new Set([
    'port',
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

export function useServerSettings({ servers, modules, setModal, setProgressBar, fetchServers, onRestart }) {
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
    const [savingSettings, setSavingSettings] = useState(false);
    // 저장 응답을 기다리는 동안 다시 누른 저장은 버린다 (G-U5)
    const savingRef = useRef(false);
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
        // 첫 await 전에 번호를 매긴다: 그사이 다른 서버를 열면 이 호출의 늦은 결과는 모두 버린다
        const request = ++openRequest.current;
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
        if (request !== openRequest.current) return;

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
            initial.memory_limit_mb = memoryLimitOf(latestServer);
            initial.stop_timeout = stopTimeoutOf(latestServer);
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
                memory_limit_mb: memoryLimitOf(latestServer),
                stop_timeout: stopTimeoutOf(latestServer),
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
                    // 켜진 익스텐션 파일이 바뀌어 확인되지 않아 모듈이 읽지 못했다
                    const blocked = blockedText(live, t);
                    if (blocked) safeShowToast(blocked, 'warning', 8000);
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
        // 버전 목록·설치를 구현한 모듈만 (MK1)
        if (
            installMethod === 'download' &&
            moduleSupports(mod, 'list_versions') &&
            moduleSupports(mod, 'install_server')
        ) {
            setVersionsLoading(true);
            try {
                const versions = await window.api.moduleListVersions(latestServer.module, { per_page: 30 });
                if (request !== openRequest.current) return;
                if (versions && versions.versions) {
                    setAvailableVersions(versions.versions);
                }
            } catch (err) {
                console.warn('Failed to load versions:', err);
            } finally {
                if (request === openRequest.current) setVersionsLoading(false);
            }
            if (request !== openRequest.current) return;

            // Detect installed version from server binary if not already saved
            const hasSavedVersion = latestServer.module_settings?.server_version || latestServer.server_version;
            if (!hasSavedVersion && latestServer.id && moduleSupports(mod, 'get_installed_version')) {
                try {
                    const detected = await window.api.instanceGetInstalledVersion(latestServer.id);
                    if (request !== openRequest.current) return;
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
        setSettingsValues((prev) => ({ ...prev, [fieldName]: Array.isArray(value) ? value : String(value) }));
    };

    // ── handleInstallVersion ────────────────────────────────
    const handleInstallVersion = async () => {
        if (!settingsServer || !settingsValues.server_version) return;
        const version = settingsValues.server_version;
        const serverName = settingsServer.name;
        const module = settingsServer.module;
        const game = gameNameOf(
            t,
            module,
            modules.find((m) => m.name === module),
        );

        setModal({
            type: 'question',
            title: t('server_settings.install_version_confirm_title', { defaultValue: 'Install Server Version' }),
            message: t('server_settings.install_version_confirm', {
                game,
                version,
                name: serverName,
                defaultValue: `Install ${game} ${version} for server '${serverName}'?\n\nThis will download and replace the server files.`,
            }),
            buttons: [
                {
                    label: t('server_settings.install_version_button', { defaultValue: 'Install' }),
                    action: async () => {
                        // 설치하는 사이 다른 서버 설정을 열면 그 창의 값은 건드리지 않는다
                        const request = openRequest.current;
                        setModal(null);
                        setVersionInstalling(true);
                        setProgressBar({ message: t('servers.progress_downloading', { game, version }), percent: 0 });
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

                            // instance_id: 데몬이 이 인스턴스 설정(서버 종류 등)으로 설치하고 작업 잠금을 잡는다
                            const installResult = await window.api.moduleInstallServer(module, {
                                version,
                                install_dir: targetDir,
                                instance_id: settingsServer.id,
                            });

                            const blocked = blockedText(installResult, t);
                            if (blocked) {
                                setProgressBar(null);
                                setVersionInstalling(false);
                                safeShowToast(blocked, 'error', 8000);
                                return;
                            }

                            if (installResult.error_code === 'extension_required') {
                                setProgressBar(null);
                                setVersionInstalling(false);
                                const extList = (installResult.missing_extensions || []).join(', ');
                                safeShowToast(
                                    installResult.error ||
                                        t('server_settings.extension_required_for_install', { extensions: extList }),
                                    'error',
                                    6000,
                                );
                                return;
                            }

                            if (installResult.error || installResult.success === false) {
                                setProgressBar(null);
                                safeShowToast(
                                    busyMessage(t, installResult.error_code, 'install_server_running') ||
                                        translateError(installResult.error || installResult.message),
                                    'error',
                                    4000,
                                );
                                setVersionInstalling(false);
                                return;
                            }

                            // 설치한 버전만 기록한다 (설정 저장은 버전을 보내지 않는다)
                            const update = { server_version: version };
                            if (installResult.jar_path) {
                                // working_dir 설정 — 실행 파일 경로는 모듈에서 자동 해석됨
                                update.working_dir =
                                    installResult.install_path || installResult.jar_path.replace(/[/\\][^/\\]+$/, '');
                            }
                            const saved = await window.api.instanceUpdateSettings(settingsServer.id, update);
                            if (!saved || saved.error || saved.success === false) {
                                setProgressBar(null);
                                safeShowToast(
                                    t('server_settings.install_settings_failed', {
                                        error: translateError(saved?.error || saved?.message || ''),
                                    }),
                                    'error',
                                    6000,
                                );
                                await fetchServers();
                                return;
                            }
                            if (request === openRequest.current) handleSettingChange('server_version', version);

                            setProgressBar({ message: t('servers.progress_complete'), percent: 100 });
                            setTimeout(() => setProgressBar(null), 2000);

                            const msg = installResult.java_warning
                                ? `${t('servers.install_completed', { game, version })}\n⚠️ ${installResult.java_warning}`
                                : t('servers.install_completed', { game, version });
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
            // 무엇이 지워지는지는 게임마다 달라 모듈 locales의 reset.description을 덧붙인다
            message: [
                t('server_settings.reset_confirm', { name: settingsServer.name }),
                t(`mod_${settingsServer.module}:reset.description`, { defaultValue: '' }),
            ]
                .filter(Boolean)
                .join('\n\n'),
            buttons: [
                {
                    label: t('modals.cancel', { defaultValue: 'Cancel' }),
                    action: () => setModal(null),
                },
                {
                    label: t('server_settings.reset_button', { defaultValue: 'Reset' }),
                    action: async () => {
                        const request = openRequest.current;
                        setModal(null);
                        setResettingServer(true);
                        try {
                            const result = await window.api.instanceResetServer(settingsServer.id);
                            // 모듈은 200으로 {success:false}를 돌려줄 수 있다
                            if (!result || result.error || result.success === false) {
                                safeShowToast(
                                    t('server_settings.reset_failed', {
                                        error:
                                            busyMessage(t, result?.error_code, 'reset_server_running') ||
                                            translateError(result?.error || result?.message || ''),
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
                                            if (updated && request === openRequest.current) {
                                                setSettingsServer(updated);
                                                // 데몬이 따로 두는 칸은 module_settings 밖이거나 표시 형식이 달라 같이 채운다 (G-U3)
                                                setSettingsValues((prev) => ({
                                                    ...prev,
                                                    ...updated.module_settings,
                                                    ...restartFlagsOf(updated),
                                                    memory_limit_mb: memoryLimitOf(updated),
                                                    stop_timeout: stopTimeoutOf(updated),
                                                }));
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
        if (!settingsServer || savingRef.current) return;
        savingRef.current = true;
        setSavingSettings(true);
        // 첫 await 전에 잡는다: 저장하는 사이 다른 서버 설정을 열었다면 그 창을 닫지 않는다
        const request = openRequest.current;

        try {
            const module = modules.find((m) => m.name === settingsServer.module);
            const convertedSettings = {};
            // 저장 직전의 추가 인자로 한 번 더 물어 남는 인자를 정한다 (미리보기 응답은 400ms 늦을 수 있다)
            const launch = await settleLaunchArgs(settingsServer.id, module?.settings?.fields, settingsValues);
            if (launch?.failed) {
                safeShowToast(
                    t(
                        launch.error_code === 'invalid_format'
                            ? 'launch_preview.invalid_format'
                            : 'launch_preview.unchecked',
                    ),
                    'warning',
                    4000,
                );
                return;
            }
            const values = launch ? { ...settingsValues, ...launch.settings } : settingsValues;

            if (module && module.settings && module.settings.fields) {
                module.settings.fields.forEach((field) => {
                    const value = values[field.name];
                    if (unconfirmedFields.current.has(field.name) && !touchedFields.current.has(field.name)) return;

                    if (field.field_type === 'boolean') {
                        convertedSettings[field.name] = value === true || value === 'true';
                        return;
                    }
                    // 목록은 빈 줄을 뺀 문자열 배열로 (데몬은 빈 항목을 invalid_format으로 거절한다)
                    if (field.field_type === 'list') {
                        convertedSettings[field.name] = listItems(value);
                        return;
                    }

                    if (value === '' || value === null || value === undefined) {
                        // 사용자가 지운 선택 문자열 칸은 빈 값으로 보내야 이전 값이 지워진다
                        if (
                            value === '' &&
                            touchedFields.current.has(field.name) &&
                            !field.required &&
                            ['text', 'password', 'multiline'].includes(field.field_type)
                        ) {
                            convertedSettings[field.name] = '';
                        }
                        return;
                    }

                    if (field.field_type === 'number') {
                        convertedSettings[field.name] = Number(value);
                    } else {
                        convertedSettings[field.name] = value;
                    }
                });
            }

            // 램 상한: 바꿨을 때만 보낸다. 비우면 제한 없음(null), 형식 검사는 데몬이 한다 (invalid_format)
            const memoryLimit = String(settingsValues.memory_limit_mb ?? '').trim();
            if (settingsValues.memory_limit_mb !== undefined && memoryLimit !== memoryLimitOf(settingsServer)) {
                convertedSettings.memory_limit_mb = memoryLimit === '' ? null : memoryLimit;
            }

            // 정지 대기 시간: 바꿨을 때만 보낸다. 비우면 모듈 값(null), 범위 검사(1~3600)는 데몬이 한다
            const stopTimeout = String(settingsValues.stop_timeout ?? '').trim();
            if (settingsValues.stop_timeout !== undefined && stopTimeout !== stopTimeoutOf(settingsServer)) {
                convertedSettings.stop_timeout = stopTimeout === '' ? null : stopTimeout;
            }

            // 크래시 자동 재시작 (데몬이 module_settings 에서 읽는다)
            // 토글은 handleSettingChange를 거쳐 'true'·'false' 글자로 올 수 있다
            for (const key of ['auto_restart', 'restart_on_unknown_exit']) {
                const flag = asFlag(settingsValues[key], null);
                if (flag !== null) convertedSettings[key] = flag;
            }

            const protocols = module?.protocols || {};
            const supportedProtocols = protocols.supported || [];

            // 모듈이 정한 프로토콜만 쓴다 (RCON은 데몬에서 빠졌다: 예전에 저장된 'rcon'도 여기서 바뀐다)
            if (supportedProtocols.length > 0) {
                convertedSettings.protocol_mode = protocols.default || supportedProtocols[0];
            } else {
                convertedSettings.protocol_mode = settingsValues.protocol_mode || 'auto';
            }

            if (launch?.extra_args) convertedSettings.extra_args = launch.extra_args;

            // 익스텐션 데이터 패스스루 — 익스텐션 탭 컴포넌트가 변경한 값을 그대로 전달
            if (settingsValues._extension_data) {
                convertedSettings.extension_data = { ...settingsValues._extension_data };
            }

            const result = await window.api.instanceUpdateSettings(settingsServer.id, convertedSettings);
            const stillOpen = request === openRequest.current;

            if (result.error && result.error_code === 'invalid_format') {
                // 형식이 틀린 값: 칸마다 따로 알리지 않고 토스트 하나로 (미리보기와 같은 문구)
                safeShowToast(t('launch_preview.invalid_format'), 'warning', 4000);
            } else if (result.error && result.error_code === 'unknown_setting') {
                // 모듈이 선언하지 않은 키
                const keys = (result.fields || []).map((f) => f.field).join(', ');
                setModal({
                    type: 'failure',
                    title: t('settings.save_failed_title'),
                    message: t('settings.unknown_setting', { fields: keys }),
                });
            } else if (result.error) {
                // validation_failed: 상세 에러 목록 포함
                if (result.error_code === 'validation_failed' && result.details) {
                    const detailStr = result.details.join('\n');
                    setModal({ type: 'failure', title: t('settings.save_failed_title'), message: detailStr });
                } else {
                    setModal({
                        type: 'failure',
                        title: t('settings.save_failed_title'),
                        message:
                            (result.error_code === 'working_dir_in_use' &&
                                t('servers.working_dir_in_use', { instance: result.instance || '?' })) ||
                            (result.error_code === 'instance_busy' && t('server_settings.instance_busy')) ||
                            blockedText(result, t) ||
                            translateError(result.error),
                    });
                }
                console.error('Error response:', result.error);
            } else {
                // 켜져 있는 서버에서 다시 켜야 적용되는 설정을 바꿨다: 데몬은 스스로 다시 켜지 않는다
                const pending = (result.restart_required || []).map((name) => {
                    const field = module?.settings?.fields?.find((f) => f.name === name);
                    return t(`mod_${settingsServer.module}:settings.${name}.label`, {
                        defaultValue: field?.label || name,
                    });
                });
                const serverName = settingsServer.name;
                setModal(
                    pending.length && onRestart
                        ? {
                              type: 'question',
                              title: t('command_modal.success'),
                              message: t('server_settings.restart_required', {
                                  name: serverName,
                                  fields: pending.join(', '),
                              }),
                              buttons: [
                                  {
                                      label: t('server_settings.restart_now'),
                                      action: () => {
                                          setModal(null);
                                          return onRestart(serverName, { confirmed: true });
                                      },
                                  },
                                  { label: t('server_settings.restart_later'), action: () => setModal(null) },
                              ],
                          }
                        : {
                              type: 'success',
                              title: t('command_modal.success'),
                              message: t('server_actions.settings_saved', { name: serverName }),
                          },
                );
                // 저장은 됐지만 모듈이 게임 설정 파일에 쓰지 못했다
                if (result.synced === false) {
                    safeShowToast(t('settings.sync_failed', { error: result.sync_error || '' }), 'warning', 6000);
                }
                // 데몬이 받았지만 쓰지 않는 키 (RCON처럼 빠진 기능)
                if (result.ignored?.length) {
                    safeShowToast(t('settings.ignored', { keys: result.ignored.join(', ') }), 'warning', 6000);
                }
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
        } finally {
            savingRef.current = false;
            setSavingSettings(false);
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
        savingSettings,
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

/**
 * 저장할 때 추가 인자를 데몬으로 한 번 더 나눈다: 설정에 해당하는 값과 남는 인자.
 * 미리보기 IPC가 없거나 실행 인자를 쓰지 않거나 실패하면 null (추가 인자를 보내지 않는다).
 */
export async function settleLaunchArgs(id, fields, values) {
    if (!window.api?.instanceLaunchPreview) return null;
    try {
        // 입력칸을 아직 받지 못했으면(undefined) 데몬이 저장된 추가 인자로 나눈다
        const res = await window.api.instanceLaunchPreview(id, launchDraft(fields, values), values._launch_extra_args);
        // 사용자가 고친 추가 인자를 나누지 못했으면 저장을 멈춘다 (그냥 저장하면 고친 인자가 빠진다).
        // 실행 명령만 못 만든 경우(아직 설치 전 등)는 extra_args가 오므로 저장한다
        if (res?.error && !Array.isArray(res.extra_args) && values._launch_extra_args !== undefined)
            return { failed: true, error_code: res.error_code };
        // 나눴어도 설정 형식에 맞지 않은 인자는 extra_args와 설정 양쪽에서 빠진다
        if (values._launch_extra_args !== undefined && res?.errors?.some((e) => e.arg))
            return { failed: true, error_code: 'invalid_format' };
        if (!res?.has_launch_args || !Array.isArray(res.extra_args)) return null;
        const settings = {};
        for (const [k, v] of Object.entries(res.settings_from_args || {})) settings[k] = String(v);
        return { extra_args: res.extra_args, settings };
    } catch {
        return values._launch_extra_args !== undefined ? { failed: true } : null;
    }
}
