import { useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useUIStore } from '../stores/useUIStore';
import {
    busyMessage,
    createTranslateError,
    installFailureText,
    moduleSupports,
    retryWithBackoff,
    safeShowToast,
} from '../utils/helpers';
import { claimStopNotice, showPortMoved } from '../utils/daemonEvents';
import { gameNameOf, reuseUnchanged } from '../utils/serverView';
import { blockedText, canAskUnisolated, showTrustWarnings } from '../utils/trustNotice';
import { updateRequiredText } from '../utils/updateRequired';

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
    openConsole,
    closeConsole,
    setShowModuleManager,
    formatUptime,
    openSettingsToExtensions,
}) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // -- Internal refs for change detection --
    const guiInitiatedOpsRef = useRef(new Set());
    const optimisticStatusRef = useRef(new Map()); // name → { status, timestamp, pending }
    const lastErrorToastRef = useRef(0);
    const firstFetchDoneRef = useRef(false);
    // 목록 요청 번호. 먼저 보낸 요청이 늦게 오면 그 목록은 버린다 (G-S4)
    const fetchSeqRef = useRef({ sent: 0, applied: 0 });
    const bootGraceRef = useRef(true); // suppress notifications during app boot
    const bootGraceTimerRef = useRef(null);

    /** Optimistic 상태 해제 후 fetchServers로 실제 상태 복원 */
    const revertOptimistic = (name) => {
        optimisticStatusRef.current.delete(name);
        fetchServers();
    };

    // ── fetchServers ────────────────────────────────────────
    const fetchServers = async () => {
        const seq = ++fetchSeqRef.current.sent;
        try {
            const data = await retryWithBackoff(() => window.api.serverList(), 3, 800);
            if (data && data.servers) {
                const stale = seq < fetchSeqRef.current.applied;
                if (!stale) fetchSeqRef.current.applied = seq;
                setServers((prev) => {
                    if (stale) return prev;
                    if (!firstFetchDoneRef.current) {
                        firstFetchDoneRef.current = true;
                        // Start boot grace period — suppress state-change
                        // notifications for 30s to avoid false alarms from
                        // daemon auto-start, initial status catch-up, etc.
                        if (!bootGraceTimerRef.current) {
                            bootGraceTimerRef.current = setTimeout(() => {
                                bootGraceRef.current = false;
                            }, 30_000);
                        }
                        return data.servers.map((newServer) => {
                            const existing = prev.find((s) => s.name === newServer.name);
                            return { ...newServer, expanded: existing?.expanded || false };
                        });
                    }

                    // Detect state changes (crash / external start·stop)
                    for (const newServer of data.servers) {
                        const existing = prev.find((s) => s.name === newServer.name);
                        if (!existing) continue;

                        const wasRunning = existing.status === 'running';
                        const nowStopped = newServer.status === 'stopped';
                        const nowRunning = newServer.status === 'running';
                        const wasStopped = existing.status === 'stopped';
                        const isGuiOp = guiInitiatedOpsRef.current.has(newServer.name);

                        const apiAction = Number(newServer.last_api_action || 0);
                        const prevApiAction = Number(existing.last_api_action || 0);
                        const apiActionUpdated = apiAction > prevApiAction;
                        const isRecentApiOp = apiAction > 0 && Date.now() - apiAction < 10 * 60 * 1000;
                        const isApiOp = apiActionUpdated || isRecentApiOp;

                        // During boot grace period, skip all state-change
                        // notifications to avoid false alarms from daemon
                        // auto-start and initial status catch-up.
                        const inBootGrace = bootGraceRef.current;

                        // Also treat servers with pending optimistic status as
                        // "known ops" — we initiated something but the guard
                        // hasn't cleared yet.
                        const hasPendingOptimistic = optimisticStatusRef.current.has(newServer.name);

                        const isKnownOp = isGuiOp || isApiOp || inBootGrace || hasPendingOptimistic;

                        if (wasRunning && nowStopped && !isKnownOp) {
                            // Crash / unexpected stop — genuinely important.
                            // This is the ONLY state transition that warrants
                            // a persistent notice. instance.exit 이벤트가 이미 알렸으면 건너뛴다 (G-S6)
                            if (claimStopNotice(newServer.id, 'poll'))
                                safeShowToast(
                                    t('servers.unexpected_stop_toast', { name: newServer.name }),
                                    'error',
                                    5000,
                                    {
                                        isNotice: true,
                                        source: newServer.name,
                                    },
                                );
                        } else if (wasRunning && nowStopped && !isGuiOp && isApiOp && !inBootGrace) {
                            // API-initiated stop (Discord bot command, etc.)
                            // Informational toast only — not notice-worthy.
                            safeShowToast(t('servers.unexpected_stop_toast', { name: newServer.name }), 'info', 3000);
                        } else if (wasStopped && nowRunning && !isKnownOp) {
                            // External start — informational only, not alarming.
                            // Toast is enough; no persistent notice.
                            safeShowToast(t('servers.external_start_toast', { name: newServer.name }), 'info', 3000);
                        } else if (wasStopped && nowRunning && !isGuiOp && isApiOp && !inBootGrace) {
                            // API-initiated start (Discord bot command, etc.)
                            safeShowToast(t('servers.external_start_toast', { name: newServer.name }), 'info', 3000);
                        }

                        if (isGuiOp && (nowStopped || nowRunning) && existing.status !== newServer.status) {
                            guiInitiatedOpsRef.current.delete(newServer.name);
                        }
                    }

                    const merged = data.servers.map((newServer) => {
                        const existing = prev.find((s) => s.name === newServer.name);

                        // Optimistic status 보호: starting/stopping 상태를
                        // 서버가 아직 전환 완료 전이면 유지 (요청이 끝난 뒤 최대 60초, 요청 중에는 계속)
                        let mergedStatus = newServer.status;
                        const opt = optimisticStatusRef.current.get(newServer.name);
                        if (opt) {
                            const elapsed = Date.now() - opt.timestamp;
                            const GUARD_MS = 60_000;
                            const transitioned =
                                (opt.status === 'starting' && newServer.status === 'running') ||
                                (opt.status === 'stopping' && newServer.status === 'stopped');
                            if (transitioned || (elapsed > GUARD_MS && !opt.pending)) {
                                optimisticStatusRef.current.delete(newServer.name);
                            } else {
                                mergedStatus = opt.status;
                            }
                        }

                        return {
                            ...newServer,
                            status: mergedStatus,
                            expanded: existing?.expanded || false,
                        };
                    });
                    return reuseUnchanged(prev, merged);
                });

                // 포트 충돌로 강제 정지된 서버가 있으면 앱 내 토스트 표시
                if (data.port_conflict_stops && data.port_conflict_stops.length > 0) {
                    for (const evt of data.port_conflict_stops) {
                        safeShowToast(
                            t('errors.port_conflict_force_stop_toast', {
                                stopped: evt.stopped_name,
                                port: evt.port,
                                existing: evt.existing_name,
                            }),
                            'error',
                            8000,
                            { isNotice: true, source: evt.stopped_name },
                        );
                    }
                }
            } else if (data && data.error) {
                console.error('Server list error:', data.error);
                const translatedErr = translateError(data.error);
                const now = Date.now();
                if (translatedErr && !loading && now - lastErrorToastRef.current > 5000) {
                    safeShowToast(t('servers.fetch_failed_toast', { error: translatedErr }), 'warning', 3000);
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
            if (errorMsg && !loading && now - lastErrorToastRef.current > 5000) {
                safeShowToast(t('servers.fetch_update_failed_toast', { error: errorMsg }), 'warning', 3000);
                lastErrorToastRef.current = now;
            }
        } finally {
            setLoading(false);
        }
    };

    // 격리 없이 돌리기 동의 (파일이 바뀌면 데몬이 다시 묻는다). 동의하면 retry를 다시 부른다
    const askUnisolated = (result, retry) => {
        const component = result.component;
        setModal({
            type: 'question',
            title: t('trust.sandbox.title'),
            message: t('trust.sandbox.message', { component, detail: result.detail || result.message || '' }),
            buttons: [
                {
                    label: t('trust.sandbox.allow'),
                    action: async () => {
                        setModal(null);
                        try {
                            const res = await window.api.sandboxAllowUnisolated(component);
                            if (res?.error_code === 'canceled') return;
                            if (!res || res.success === false || res.error) {
                                safeShowToast(
                                    t('trust.sandbox.allow_failed', { error: translateError(res?.error || '') }),
                                    'error',
                                    5000,
                                );
                                return;
                            }
                            safeShowToast(t('trust.sandbox.allowed', { component }), 'info', 5000);
                            retry();
                        } catch (err) {
                            safeShowToast(t('trust.sandbox.allow_failed', { error: err.message }), 'error', 5000);
                        }
                    },
                },
                { label: t('modals.cancel'), action: () => setModal(null) },
            ],
        });
    };

    // 약관 동의 창 (module.toml [[agreements]]). 동의하면 이 PC에서 기록하고 다시 켠다
    const askAgreements = (srv, name, module, title, message, agreements) => {
        setModal({
            type: 'agreements',
            title,
            message,
            module: srv?.module || module,
            agreements,
            onAccept: async (ids) => {
                setModal(null);
                try {
                    const res = await window.api.instanceAgreements(srv.id, ids);
                    if (!res || res.success === false || res.error) {
                        safeShowToast(translateError(res?.error || ''), 'error', 5000);
                        return;
                    }
                    handleStart(name, module);
                } catch (err) {
                    safeShowToast(translateError(err.message), 'error', 5000);
                }
            },
        });
    };

    // ── handleStart ─────────────────────────────────────────
    // 데몬이 Windows 예약 범위를 피해 포트를 옮겨 저장했다 (시작·재시작, 성공·실패 모두): 조용히 바뀌지 않게 알린다
    const notifyPortMoved = (result, srv) => {
        for (const m of Array.isArray(result?.port_moved) ? result.port_moved : []) {
            showPortMoved({ instanceId: srv.id, name: srv.name, from: m.from, to: m.to }, t);
        }
    };

    const handleStart = async (name, module) => {
        try {
            const srv = servers.find((s) => s.name === name);
            if (!srv) {
                safeShowToast(t('servers.start_failed_toast', { error: 'Instance not found' }), 'error', 4000);
                return;
            }

            // Optimistic update: 즉시 'starting' 상태 표시
            // 시작 요청이 끝날 때까지는(첫 시작은 몇 분) 목록 새로고침이 이 표시를 지우지 않는다
            optimisticStatusRef.current.set(name, { status: 'starting', timestamp: Date.now(), pending: true });
            setServers((prev) => prev.map((s) => (s.name === name ? { ...s, status: 'starting' } : s)));

            // 시작 방식(managed 여부)과 포트 충돌 검사는 데몬이 인스턴스·전역 설정으로 정한다
            const result = await window.api.instanceStart(srv.id);
            const opt = optimisticStatusRef.current.get(name);
            if (opt?.pending) optimisticStatusRef.current.set(name, { status: 'starting', timestamp: Date.now() });
            notifyPortMoved(result, srv);

            // ── action_required: 서버 실행 파일 없음 (범용 코드, 예전 모듈은 server_jar_not_found) ──
            if (
                result.action_required === 'executable_not_found' ||
                result.action_required === 'server_jar_not_found'
            ) {
                // 파일 종류는 설정된 경로의 확장자로, 없으면 모듈이 밝힌 실행 파일 이름으로 (둘 다 없으면 모든 파일)
                const declared = modules.find((m) => m.name === (srv?.module || module))?.server_executable;
                const configuredName = (result.configured_path || declared || '').split(/[/\\]/).pop();
                const ext = configuredName.includes('.') ? configuredName.split('.').pop() : '';
                revertOptimistic(name);
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
                                        filters: ext ? [{ name: configuredName, extensions: [ext] }] : undefined,
                                        title: t('servers.select_server_jar'),
                                    });
                                    if (filePath) {
                                        const s = servers.find((s) => s.name === name);
                                        if (s) {
                                            // 고른 파일과 그 폴더: 이름이 모듈 기본값과 달라도, 예전 경로가 남아 있어도 이 파일로 켠다
                                            const dir = filePath.replace(/[/\\][^/\\]+$/, '');
                                            const saved = await window.api.instanceUpdateSettings(s.id, {
                                                working_dir: dir,
                                                executable_path: filePath,
                                            });
                                            if (!saved || saved.error || saved.success === false) {
                                                safeShowToast(
                                                    translateError(saved?.error || saved?.message || ''),
                                                    'error',
                                                    4000,
                                                );
                                                return;
                                            }
                                            safeShowToast(t('servers.jar_path_updated'), 'success', 3000);
                                            await fetchServers();
                                            handleStart(name, module);
                                        }
                                    }
                                } catch (err) {
                                    safeShowToast(translateError(err.message), 'error', 4000);
                                }
                            },
                        },
                        moduleSupports(
                            modules.find((m) => m.name === module),
                            'install_server',
                        ) && {
                            label: t('servers.jar_action_install_new'),
                            action: async () => {
                                setModal(null);
                                try {
                                    const installDir = await window.api.openFolderDialog();
                                    if (!installDir) return;
                                    const s = servers.find((s) => s.name === name);

                                    setProgressBar({ message: t('servers.progress_installing'), indeterminate: true });

                                    // 최신 버전은 데몬이 이 인스턴스 설정(서버 종류 등)으로 고른다 (L23)
                                    const installResult = await window.api.moduleInstallServer(module, {
                                        instance_id: s?.id,
                                        install_dir: installDir,
                                    });

                                    if (installResult.error || installResult.success === false) {
                                        setProgressBar(null);
                                        const code = installResult.error_code;
                                        safeShowToast(
                                            busyMessage(t, code, 'install_server_running') ||
                                                (code === 'version_unknown'
                                                    ? t('servers.version_unknown', { error: installResult.error })
                                                    : installFailureText(t, installResult, translateError)),
                                            'error',
                                            8000,
                                        );
                                        return;
                                    }

                                    // 받은 실행 파일·작업 폴더는 데몬이 그 인스턴스에 기록했다
                                    setProgressBar({ message: t('servers.progress_complete'), percent: 100 });
                                    setTimeout(() => setProgressBar(null), 2000);

                                    const done = t('servers.install_completed', {
                                        game: gameNameOf(
                                            t,
                                            module,
                                            modules.find((m) => m.name === module),
                                        ),
                                        version: installResult.version || '',
                                    });
                                    const msg = installResult.java_warning
                                        ? `${done}\n⚠️ ${installResult.java_warning}`
                                        : done;
                                    safeShowToast(msg, 'success', 5000);
                                    await fetchServers();

                                    if (!installResult.java_warning) {
                                        handleStart(name, module);
                                    }
                                } catch (err) {
                                    setProgressBar(null);
                                    safeShowToast(translateError(err.message), 'error', 4000);
                                }
                            },
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => setModal(null),
                        },
                    ].filter(Boolean),
                });
                return;
            }

            // ── action_required: extension_required ──
            if (result.action_required === 'extension_required') {
                revertOptimistic(name);
                setModal({
                    type: 'question',
                    title: t('servers.extension_required_title', { defaultValue: 'Extension Required' }),
                    message:
                        result.message ||
                        t('servers.extension_required_message', {
                            name,
                            defaultValue: `Server '${name}' requires an extension that is not enabled.`,
                        }),
                    buttons: [
                        {
                            label: t('servers.extension_open_settings', { defaultValue: 'Open Extension Settings' }),
                            action: () => {
                                setModal(null);
                                if (openSettingsToExtensions) {
                                    openSettingsToExtensions();
                                }
                            },
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => setModal(null),
                        },
                    ],
                });
                return;
            }

            // ── 모듈이 더 새 사바쨩을 요구함 (saba-core 의존성, 더 새 module.toml 스키마) — 업데이트 화면으로 ──
            const updateText = updateRequiredText(t, result, srv?.module || module);
            if (updateText) {
                revertOptimistic(name);
                setModal({
                    type: 'question',
                    title: t('update_required.title'),
                    message: updateText,
                    buttons: [
                        {
                            label: t('update_required.check'),
                            action: () => {
                                setModal(null);
                                useUIStore.getState().openSettings('update');
                            },
                        },
                        { label: t('modals.close'), action: () => setModal(null) },
                    ],
                });
                return;
            }

            // ── error_code: module_dependency_unsatisfied — 모듈이 요구하는 익스텐션 버전이 안 맞음 ──
            if (result.error_code === 'module_dependency_unsatisfied') {
                revertOptimistic(name);
                const missing = result.missing_dependencies || [];
                const needsExtension = missing.some((d) => d.startsWith('ext-'));
                const title = t('servers.dependency_unmet_title');
                const message = `${t('servers.dependency_unmet_message', { name })}\n\n${missing.map((d) => `• ${d}`).join('\n')}`;
                if (needsExtension && openSettingsToExtensions) {
                    setModal({
                        type: 'question',
                        title,
                        message,
                        buttons: [
                            {
                                label: t('servers.extension_open_settings'),
                                action: () => {
                                    setModal(null);
                                    openSettingsToExtensions();
                                },
                            },
                            { label: t('modals.close'), action: () => setModal(null) },
                        ],
                    });
                } else {
                    setModal({ type: 'failure', title, message });
                }
                return;
            }

            // ── error_code: agreement_required — 모듈이 선언한 약관(EULA 등)에 아직 동의하지 않음 ──
            if (result.error_code === 'agreement_required') {
                revertOptimistic(name);
                const pending = Array.isArray(result.agreements) ? result.agreements : [];
                if (pending.length && window.api.instanceAgreements) {
                    askAgreements(
                        srv,
                        name,
                        module,
                        t('agreements.start_title'),
                        t('agreements.start_message', { name }),
                        pending,
                    );
                } else {
                    // 데몬 문구는 이미 현지화돼 있다
                    setModal({ type: 'failure', title: t('agreements.start_title'), message: result.error || '' });
                }
                return;
            }

            // ── error_code: preflight_failed — 켜기 전 점검(모듈 validate)이 critical 문제를 냈다 ──
            if (result.error_code === 'preflight_failed') {
                revertOptimistic(name);
                const issues = Array.isArray(result.issues) ? result.issues : [];
                const others = issues.slice(1).map((i) => `• ${i.message || i.code}`);
                const message = [result.error, result.solution, others.length ? others.join('\n') : null]
                    .filter(Boolean)
                    .join('\n\n');
                const title = t('servers.preflight_failed_title', { name });
                // 동의 기록은 있는데 파일(eula.txt 등)이 지워진 경우: 다시 동의하면 모듈이 파일을 다시 쓴다
                // 서버 목록에는 agreements가 없어 모듈 목록의 선언을 쓴다
                const srvAgreements = modules.find((m) => m.name === (srv?.module || module))?.agreements || [];
                if (
                    result.issue_code === 'EULA_NOT_ACCEPTED' &&
                    srvAgreements.length &&
                    window.api.instanceAgreements
                ) {
                    askAgreements(srv, name, module, title, message, srvAgreements);
                } else {
                    setModal({ type: 'failure', title, message });
                }
                return;
            }

            // ── error_code: start_in_progress — 같은 서버를 이미 켜는 중 (다른 창·봇·더블클릭) ──
            if (result.error_code === 'start_in_progress') {
                // 실제로 켜지는 중이므로 '켜는 중' 표시는 그대로 두고 알려만 준다
                safeShowToast(t('servers.start_in_progress', { name }), 'info', 3000);
                return;
            }

            // ── error_code: start_cancelled — 켜는 동안 사용자가 정지했다: 실패가 아니므로 알리지 않는다 ──
            if (result.error_code === 'start_cancelled') {
                revertOptimistic(name);
                return;
            }

            // ── error_code: port_conflict — 데몬이 포트 충돌 감지 ──
            if (result.error_code === 'port_conflict' || result.error === 'port_conflict') {
                revertOptimistic(name);
                const conflictDetails = (result.conflicts || []).join('\n');
                setModal({
                    type: 'failure',
                    title: t('errors.port_conflict', { defaultValue: 'Port Conflict' }),
                    message: translateError(result.message || '') + (conflictDetails ? '\n\n' + conflictDetails : ''),
                });
                return;
            }

            // ── action_required: 격리를 준비하지 못함 → 이 구성요소만 격리 없이 돌릴지 한 번 묻는다 ──
            if (canAskUnisolated(result)) {
                revertOptimistic(name);
                askUnisolated(result, () => handleStart(name, module));
                return;
            }

            // ── action_required: 확인되지 않은 구성요소라 데몬이 막음 (악성코드 검사·서버 폴더 밖 실행) ──
            const blocked = blockedText(result, t);
            if (blocked) {
                revertOptimistic(name);
                safeShowToast(t('servers.start_failed_toast', { error: blocked }), 'error', 8000);
                return;
            }

            // ── success=false without specific action_required ──
            if (result.success === false) {
                revertOptimistic(name);
                const msg = result.message || result.error || 'Unknown error';
                safeShowToast(translateError(msg), 'error', 5000);
                return;
            }

            if (result.error) {
                revertOptimistic(name);
                const errorMsg = translateError(result.error);
                safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
            } else {
                guiInitiatedOpsRef.current.add(name);
                setProgressBar({ message: t('servers.starting_toast', { name }), indeterminate: true });
                if (result.managed) {
                    openConsole(srv.id, name);
                }

                // Poll until running (max 30s)
                let attempts = 0;
                const maxAttempts = 60;
                const delay = 500;
                let resolved = false;
                let consecutiveStopped = 0;

                const checkStatus = async () => {
                    if (resolved) return;
                    attempts++;
                    try {
                        const statusResult = await window.api.serverStatus(name);
                        if (statusResult.status === 'running') {
                            resolved = true;
                            setProgressBar(null);
                            safeShowToast(t('servers.start_completed_toast', { name }), 'success', 3000);
                            fetchServers();
                            return;
                        }
                        // 프로세스가 즉시 죽은 경우 조기 감지 (5회 연속 stopped → 크래시 판정)
                        if (statusResult.status === 'stopped') {
                            consecutiveStopped++;
                            if (consecutiveStopped >= 5) {
                                resolved = true;
                                setProgressBar(null);
                                revertOptimistic(name);
                                safeShowToast(
                                    t('servers.start_failed_toast', { error: 'Process exited immediately' }),
                                    'error',
                                    4000,
                                );
                                fetchServers();
                                return;
                            }
                        } else {
                            consecutiveStopped = 0;
                        }
                    } catch (_error) {
                        /* ignore */
                    }
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
            revertOptimistic(name);
            const errorMsg = translateError(error.message);
            safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
        }
    };

    // 콘솔도 REST도 없어 저장 후 종료를 요청할 수 없는 서버는 데몬이 그대로 둔다 (MK10). 저장 없이 끈다고 알리고 고를 때만 force로
    const askForceStop = (name, retry) =>
        setModal({
            type: 'question',
            title: t('servers.force_stop_title'),
            message: t('servers.force_stop_message', { name }),
            buttons: [
                {
                    label: t('servers.force_stop_confirm'),
                    action: () => {
                        setModal(null);
                        retry();
                    },
                },
                { label: t('modals.cancel'), action: () => setModal(null) },
            ],
        });

    // ── handleStop ──────────────────────────────────────────
    const handleStop = async (name) => {
        const stopNow = async (forced) => {
            const undo = () => {
                guiInitiatedOpsRef.current.delete(name);
                revertOptimistic(name);
            };
            try {
                // Optimistic update: 즉시 'stopping' 상태 표시
                // 데몬은 stop_timeout까지 기다린 뒤 답하므로, 요청이 끝날 때까지는 목록 새로고침이 이 표시를 지우지 않는다.
                // 그사이 꺼진 것도 GUI가 한 정지로 본다
                optimisticStatusRef.current.set(name, { status: 'stopping', timestamp: Date.now(), pending: true });
                guiInitiatedOpsRef.current.add(name);
                setServers((prev) => prev.map((s) => (s.name === name ? { ...s, status: 'stopping' } : s)));

                const srv = servers.find((s) => s.name === name);
                const useGraceful = srv?.module_settings?.graceful_stop;
                const forceStop = forced || useGraceful === false;

                // id로 정지 (이름이 바뀌어도 안전). 옛 데몬·IPC면 이름으로
                const result =
                    srv?.id && window.api.instanceStop
                        ? await window.api.instanceStop(srv.id, { force: forceStop })
                        : await window.api.serverStop(name, { force: forceStop });
                const opt = optimisticStatusRef.current.get(name);
                if (opt?.pending) optimisticStatusRef.current.set(name, { status: 'stopping', timestamp: Date.now() });

                // ── extension_required 처리 ──
                if (result.action_required === 'extension_required') {
                    undo();
                    setModal({
                        type: 'question',
                        title: t('servers.extension_required_title', { defaultValue: 'Extension Required' }),
                        message:
                            result.message ||
                            t('servers.extension_required_message', {
                                name,
                                defaultValue: `Server '${name}' requires an extension that is not enabled.`,
                            }),
                        buttons: [
                            {
                                label: t('servers.extension_open_settings', {
                                    defaultValue: 'Open Extension Settings',
                                }),
                                action: () => {
                                    setModal(null);
                                    if (openSettingsToExtensions) {
                                        openSettingsToExtensions();
                                    }
                                },
                            },
                            {
                                label: t('modals.cancel'),
                                action: () => setModal(null),
                            },
                        ],
                    });
                    return;
                }

                if (result.error_code === 'graceful_stop_unavailable') {
                    undo();
                    askForceStop(name, () => stopNow(true));
                    return;
                }

                const blocked = blockedText(result, t);
                if (result.success === false && (result.message || blocked)) {
                    undo();
                    safeShowToast(
                        blocked ? t('servers.stop_failed_toast', { error: blocked }) : result.message,
                        'error',
                        blocked ? 8000 : 5000,
                    );
                    return;
                }

                if (result.error) {
                    undo();
                    const errorMsg = translateError(result.error);
                    safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
                } else {
                    if (srv) {
                        closeConsole(srv.id);
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
                                safeShowToast(t('servers.stop_completed_toast', { name }), 'success', 3000);
                                fetchServers();
                                return;
                            }
                        } catch (_error) {
                            /* ignore */
                        }
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
                undo();
                const errorMsg = translateError(error.message);
                safeShowToast(t('servers.stop_failed_toast', { error: errorMsg }), 'error', 4000);
            }
        };
        setModal({
            type: 'question',
            title: t('servers.stop_confirm_title'),
            message: t('servers.stop_confirm_message', { name }),
            onConfirm: async () => {
                setModal(null);
                await stopNow(false);
            },
            onCancel: () => setModal(null),
        });
    };

    // ── handleRestart: 데몬이 프로세스가 끝나길 기다렸다가 다시 켠다 ──
    // confirmed: 부른 곳에서 이미 재시작을 골랐다 (설정 저장 뒤 '지금 다시 시작') → 다시 묻지 않는다
    const handleRestart = async (name, { confirmed = false } = {}) => {
        const srv = servers.find((s) => s.name === name);
        if (!srv?.id || !window.api.instanceRestart) return;
        const restartNow = async (forced) => {
            optimisticStatusRef.current.set(name, { status: 'stopping', timestamp: Date.now() });
            setServers((prev) => prev.map((s) => (s.name === name ? { ...s, status: 'stopping' } : s)));
            guiInitiatedOpsRef.current.add(name);
            setProgressBar({ message: t('servers.restarting_toast', { name }), indeterminate: true });
            try {
                const force = forced || srv.module_settings?.graceful_stop === false;
                const result = await window.api.instanceRestart(srv.id, { force });
                notifyPortMoved(result, srv);
                const code = result?.error_code;
                if (code === 'graceful_stop_unavailable') {
                    revertOptimistic(name);
                    askForceStop(name, () => restartNow(true));
                } else if (code === 'start_cancelled' || code === 'restart_cancelled') {
                    // 그사이 다른 쪽에서 정지·시작·삭제했다: 실패가 아니라 그만둔 것
                    revertOptimistic(name);
                    safeShowToast(t(`errors.${code}`), 'info', 4000);
                } else if (result?.error || result?.success === false) {
                    revertOptimistic(name);
                    const msg = translateError(result.error || result.message || '');
                    safeShowToast(t('servers.restart_failed_toast', { error: msg }), 'error', 4000);
                } else {
                    // 데몬이 다시 켰다: 폴링이 짧은 정지 구간을 놓쳐도 '끄는 중'으로 남기지 않는다
                    optimisticStatusRef.current.delete(name);
                    safeShowToast(t('servers.restart_completed_toast', { name }), 'success', 3000);
                }
            } catch (error) {
                revertOptimistic(name);
                safeShowToast(
                    t('servers.restart_failed_toast', { error: translateError(error.message) }),
                    'error',
                    4000,
                );
            } finally {
                setProgressBar(null);
                fetchServers();
            }
        };
        if (confirmed) return restartNow(false);
        setModal({
            type: 'question',
            title: t('servers.restart_confirm_title'),
            message: t('servers.restart_confirm_message', { name }),
            onConfirm: async () => {
                setModal(null);
                await restartNow(false);
            },
            onCancel: () => setModal(null),
        });
    };

    // ── handleRename: 꺼져 있을 때만 (데몬이 server_running으로 거절) ──
    const handleRename = async (id, newName) => {
        const name = (newName || '').trim();
        const srv = servers.find((s) => s.id === id);
        if (!srv || !name || name === srv.name || !window.api.instanceRename) return false;
        try {
            const result = await window.api.instanceRename(id, name);
            if (result?.success) {
                setServers((prev) => prev.map((s) => (s.id === id ? { ...s, name: result.name || name } : s)));
                fetchServers();
                return true;
            }
            const code = result?.error_code;
            const msg =
                code === 'name_taken' || code === 'server_running' || code === 'instance_busy'
                    ? t(`servers.rename_${code}`, { name })
                    : translateError(result?.error || '');
            safeShowToast(t('servers.rename_failed_toast', { error: msg }), 'error', 4000);
        } catch (error) {
            safeShowToast(t('servers.rename_failed_toast', { error: translateError(error.message) }), 'error', 4000);
        }
        return false;
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
    const handleAddServer = async (payload, legacyModuleName) => {
        // payload = { name, module_name, accept_agreements?, preset?, extension_data?, migration_source?, module_settings? }
        const serverName = typeof payload === 'string' ? payload : payload?.name;
        const moduleName = typeof payload === 'string' ? legacyModuleName : payload?.module_name;

        if (!serverName || !serverName.trim()) {
            setModal({
                type: 'failure',
                title: t('servers.add_server_name_empty_title'),
                message: t('servers.add_server_name_empty_message'),
            });
            return false;
        }
        if (!moduleName) {
            setModal({
                type: 'failure',
                title: t('servers.add_module_empty_title'),
                message: t('servers.add_module_empty_message'),
            });
            return false;
        }

        try {
            const selectedModuleData = modules.find((m) => m.name === moduleName);

            const instanceData = {
                name: serverName.trim(),
                module_name: moduleName,
                // executable_path는 모듈의 server_executable + working_dir에서 자동 해석됨
            };

            // 만들 때 고른 모듈 설정 (예: 서버 종류)
            if (payload?.module_settings && Object.keys(payload.module_settings).length > 0) {
                instanceData.module_settings = payload.module_settings;
            }

            // 만들 때 동의한 약관 id (module.toml [[agreements]])
            if (payload?.accept_agreements?.length) {
                instanceData.accept_agreements = payload.accept_agreements;
            }

            // 익스텐션이 물은 칸 (AddServer.options → extension_data)
            if (payload?.extension_data) {
                instanceData.extension_data = payload.extension_data;
            }

            // 모듈 추천 프리셋 id (값은 데몬이 푼다)
            if (payload?.preset) {
                instanceData.preset = payload.preset;
            }

            // 마이그레이션: 기존 서버 디렉토리 직접 연결
            if (payload?.migration_source) {
                instanceData.migration_source = payload.migration_source;
            }

            console.log('Adding instance:', instanceData);
            const result = await window.api.instanceCreate(instanceData);

            // ── 켜진 익스텐션 파일이 바뀌어 확인되지 않음 (다시 설치·되돌리기 안내) ──
            const blocked = blockedText(result, t);
            if (blocked) {
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: blocked });
                return false;
            }

            // ── extension_required 처리 ──
            if (result.error_code === 'extension_required') {
                setModal({
                    type: 'question',
                    title: t('servers.extension_required_title', { defaultValue: 'Extension Required' }),
                    message: result.missing_extensions
                        ? t('servers.extension_required_message_detail', {
                              name: serverName,
                              extensions: result.missing_extensions.join(', '),
                              defaultValue: `Server '${serverName}' requires the following extension(s): ${result.missing_extensions.join(', ')}. Enable them in Settings → Extensions first.`,
                          })
                        : t('servers.extension_required_message', {
                              name: serverName,
                              defaultValue: `Server '${serverName}' requires an extension that is not enabled.`,
                          }),
                    buttons: [
                        {
                            label: t('servers.extension_open_settings', { defaultValue: 'Open Extension Settings' }),
                            action: () => {
                                setModal(null);
                                if (openSettingsToExtensions) {
                                    openSettingsToExtensions();
                                }
                            },
                        },
                        {
                            label: t('modals.cancel'),
                            action: () => setModal(null),
                        },
                    ],
                });
                return false;
            }

            // 데몬은 같은 이름의 서버를 만들지 않는다 (이름으로 서버를 찾는 화면이 섞이지 않게)
            if (result.error_code === 'name_taken') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('servers.rename_name_taken', { name: serverName }),
                });
                return false;
            }

            if (result.error_code === 'working_dir_in_use') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('servers.working_dir_in_use', { instance: result.instance || '?' }),
                });
                return false;
            }

            if (result.error_code === 'preset_not_found') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('add_server_modal.preset_not_found'),
                });
                return false;
            }

            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: errorMsg });
            } else {
                showTrustWarnings(result, t);
                if (result.provisioning) {
                    // 프로비저닝 시작 (컨테이너 또는 네이티브 설치) — 모달 닫고 서버 리스트에서 진행률 표시
                    setShowModuleManager(false);
                    fetchServers();
                } else {
                    // 마이그레이션: 모듈이 판정한 서버 종류와 플러그인·모드
                    const detected = result.detected;
                    const components = (detected?.components || []).filter((c) => c.enabled !== false);
                    const detectedLine = detected?.server_type
                        ? '\n' +
                          t('server_actions.server_detected', {
                              type: detected.server_type,
                              version: detected.version || '',
                              count: components.length,
                              names: components.length ? ` (${components.map((c) => c.name).join(', ')})` : '',
                          })
                        : '';
                    setModal({
                        type: 'success',
                        title: t('command_modal.success'),
                        message: t('server_actions.server_added', { name: serverName }) + detectedLine,
                    });
                    setShowModuleManager(false);
                    fetchServers();
                }
            }
            return !result.error;
        } catch (error) {
            const errorMsg = translateError(error.message);
            setModal({ type: 'failure', title: t('servers.add_error_title'), message: errorMsg });
            return false;
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
            if (result.error_code === 'ETIMEDOUT') {
                // 답이 늦었을 뿐 데몬은 계속 지우고 있을 수 있다: 목록에 남았는지로 판단한다
                const list = await window.api.serverList().catch(() => null);
                const checked = Array.isArray(list?.servers);
                const gone = checked && !list.servers.some((s) => s.id === server.id);
                setModal(
                    !checked
                        ? {
                              type: 'failure',
                              title: t('server_actions.delete_confirm_title'),
                              message: t('server_actions.delete_unconfirmed', { name: server.name }),
                          }
                        : gone
                          ? {
                                type: 'success',
                                title: t('command_modal.success'),
                                message: t('server_actions.server_deleted', { name: server.name }),
                            }
                          : {
                                type: 'notification',
                                title: t('server_actions.delete_confirm_title'),
                                message: t('server_actions.delete_still_running', { name: server.name }),
                            },
                );
                fetchServers();
                return;
            }
            if (result.error) {
                const errorMsg = translateError(result.error);
                setModal({ type: 'failure', title: t('servers.delete_failed_title'), message: errorMsg });
            } else {
                console.log(`Instance "${server.name}" (ID: ${server.id}) deleted`);
                setModal({
                    type: 'success',
                    title: t('command_modal.success'),
                    message: t('server_actions.server_deleted', { name: server.name }),
                });
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
        handleRestart,
        handleRename,
        handleStatus,
        handleAddServer,
        handleDeleteServer,
    };
}
