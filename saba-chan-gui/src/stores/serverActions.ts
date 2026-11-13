import type { StoreApi } from 'zustand';
import { holdSavingOrder } from '../hooks/useDragReorder';
import type { ApiBody, Json } from '../types/daemon';
import { claimServerNotice, eventsLive, showPortMoved, watchInstance } from '../utils/daemonEvents';
import {
    busyMessage,
    installFailureText,
    moduleFailureText,
    moduleSupports,
    retryWithBackoff,
    safeShowToast,
} from '../utils/helpers';
import { gameNameOf, reuseUnchanged } from '../utils/serverView';
import { blockedText, canAskUnisolated, showTrustWarnings } from '../utils/trustNotice';
import { updateRequiredText } from '../utils/updateRequired';
import { t, translateError } from './serverI18n';
import type { Server, ServerState } from './useServerStore';
import { setModal, setProgressBar, useUIStore } from './useUIStore';

/** 만들기 요청 본문 (AddServer 모달 → 데몬 POST /api/instances). 예전 호출은 이름 문자열만 넘겼다 */
export interface AddServerPayload {
    name: string;
    module_name: string;
    accept_agreements?: string[];
    preset?: string;
    extension_data?: Record<string, Json>;
    migration_source?: string;
    migration_mode?: string;
    module_settings?: Record<string, unknown>;
    version?: string;
    working_dir?: string;
    managed_start?: boolean;
    port?: number;
    /** 브랜치 서버: 상위 서버 id (모듈·설정은 데몬이 그 서버에서 가져온다) */
    branch_of?: string;
}

/** 서버 동작: 목록 받기, 켜기·끄기·다시 시작, 이름 바꾸기, 만들기, 지우기 (useServerStore의 액션) */
export interface ServerActions {
    fetchServers: () => Promise<void>;
    startServer: (id: string) => Promise<void>;
    stopServer: (id: string) => Promise<void>;
    /** confirmed: 부른 곳에서 이미 재시작을 골랐다 (설정 저장 뒤 '지금 다시 시작') → 다시 묻지 않는다 */
    restartServer: (id: string, options?: { confirmed?: boolean }) => Promise<void>;
    renameServer: (id: string, newName: string) => Promise<boolean>;
    addServer: (payload: AddServerPayload, legacyModuleName?: string) => Promise<boolean>;
    deleteServer: (server: Server) => void;
}

/** 화면이 버튼을 눌러 낙관적으로 바꿔 둔 상태 */
interface OptimisticStatus {
    status: string;
    timestamp: number;
    pending?: boolean;
}

/** 동작이 기억하는 것: 화면이 그리지 않으므로 스토어 상태 밖에 둔다 */
const freshInternal = () => ({
    // 이 창이 시킨 켜기·끄기 (폴링이 밖에서 바뀐 것으로 알리지 않는다)
    guiOps: new Set<string>(),
    optimistic: new Map<string, OptimisticStatus>(), // id → { status, timestamp, pending }
    lastErrorToast: 0,
    firstFetchDone: false,
    // 목록 요청 번호. 먼저 보낸 요청이 늦게 오면 그 목록은 버린다 (G-S4)
    fetchSeq: { sent: 0, applied: 0 },
    bootGrace: true, // suppress notifications during app boot
    bootGraceTimer: null as ReturnType<typeof setTimeout> | null,
});
let internal = freshInternal();

/** 시험용: 기억한 것을 비운다 (useServerStore._resetForTest) */
export function resetServerActions() {
    if (internal.bootGraceTimer) clearTimeout(internal.bootGraceTimer);
    internal = freshInternal();
}

/** 관리 모드로 켠 서버의 콘솔을 여는 곳. 콘솔은 App의 useMultiConsole이 들고 있어 App이 넣는다 */
let openConsole: (instanceId: string, serverName: string) => void = () => {};
export const setServerConsole = (open: typeof openConsole) => {
    openConsole = open;
};

const ui = () => useUIStore.getState();

export function serverActions(set: StoreApi<ServerState>['setState'], get: () => ServerState): ServerActions {
    // 되돌리지 못한 설정 사본: 개수와 함께 데몬이 알려준 보관 위치를 보인다
    const unrestoredNote = (r: ApiBody) =>
        r.settings_kept_at
            ? t('server_actions.unlink_restore_failed_kept', { n: r.settings_restore_failed, path: r.settings_kept_at })
            : t('server_actions.unlink_restore_failed', { n: r.settings_restore_failed });

    /** Optimistic 상태 해제 후 fetchServers로 실제 상태 복원 */
    const revertOptimistic = (id: string) => {
        internal.optimistic.delete(id);
        fetchServers();
    };

    // ── fetchServers ────────────────────────────────────────
    const fetchServers = async () => {
        const seq = ++internal.fetchSeq.sent;
        try {
            const data = await retryWithBackoff(() => window.api.serverList(), 3, 800);
            const list = data?.servers;
            if (list) {
                const stale = seq < internal.fetchSeq.applied;
                if (!stale) internal.fetchSeq.applied = seq;
                get().setServers((prev) => {
                    if (stale) return prev;
                    if (!internal.firstFetchDone) {
                        internal.firstFetchDone = true;
                        // Start boot grace period — suppress state-change
                        // notifications for 30s to avoid false alarms from
                        // daemon auto-start, initial status catch-up, etc.
                        if (!internal.bootGraceTimer) {
                            internal.bootGraceTimer = setTimeout(() => {
                                internal.bootGrace = false;
                            }, 30_000);
                        }
                        return list.map((newServer: Server) => {
                            const existing = prev.find((s) => s.id === newServer.id);
                            return { ...newServer, status: newServer.transition || newServer.status, expanded: existing?.expanded || false };
                        });
                    }

                    // Detect state changes (crash / external start·stop)
                    for (const newServer of list) {
                        const existing = prev.find((s) => s.id === newServer.id);
                        if (!existing) continue;

                        const wasRunning = existing.status === 'running';
                        const nowStopped = newServer.status === 'stopped';
                        const nowRunning = newServer.status === 'running';
                        const wasStopped = existing.status === 'stopped';
                        const isGuiOp = internal.guiOps.has(newServer.id);

                        const apiAction = Number(newServer.last_api_action || 0);
                        const prevApiAction = Number(existing.last_api_action || 0);
                        const apiActionUpdated = apiAction > prevApiAction;
                        const isRecentApiOp = apiAction > 0 && Date.now() - apiAction < 10 * 60 * 1000;
                        const isApiOp = apiActionUpdated || isRecentApiOp;

                        // During boot grace period, skip all state-change
                        // notifications to avoid false alarms from daemon
                        // auto-start and initial status catch-up.
                        const inBootGrace = internal.bootGrace;

                        // Also treat servers with pending optimistic status as
                        // "known ops" — we initiated something but the guard
                        // hasn't cleared yet.
                        const hasPendingOptimistic = internal.optimistic.has(newServer.id);

                        const isKnownOp = isGuiOp || isApiOp || inBootGrace || hasPendingOptimistic;

                        // 데몬 이벤트를 받으면 켜짐·꺼짐은 이벤트가 출처와 함께 알린다 (instance.started·stopped·exit).
                        // 폴링은 이벤트를 못 받을 때(셸이 스트림에 다시 붙지 못하는 동안 포함)만, 사바쨩을 거치지 않은 변화만 알린다.
                        // 익스텐션이 상태를 알려 주는 서버(server.list_enrich)는 데몬이 종료를 판정하지 않아 이벤트가 없다
                        const extensionStatus = Object.keys(newServer.extension_status || {}).length > 0;
                        const pollNotices = (!eventsLive() || extensionStatus) && !isKnownOp;
                        if (pollNotices && wasRunning && nowStopped) {
                            // Crash / unexpected stop — genuinely important.
                            // instance.exit 이벤트가 이미 알렸으면 건너뛴다 (G-S6)
                            if (claimServerNotice(newServer.id, 'stop', 'poll'))
                                safeShowToast(
                                    t('servers.unexpected_stop_toast', { name: newServer.name }),
                                    'error',
                                    5000,
                                    {
                                        isNotice: true,
                                        source: newServer.name,
                                        notice: { message: t('events.n_exit_unknown_exit'), instanceId: newServer.id },
                                    },
                                );
                        } else if (pollNotices && wasStopped && nowRunning && claimServerNotice(newServer.id, 'start', 'poll')) {
                            // External start — informational only, not alarming.
                            safeShowToast(t('servers.external_start_toast', { name: newServer.name }), 'info', 3000);
                        }

                        if (isGuiOp && (nowStopped || nowRunning) && existing.status !== newServer.status) {
                            internal.guiOps.delete(newServer.id);
                        }
                    }

                    const merged = list.map((newServer: Server) => {
                        const existing = prev.find((s) => s.id === newServer.id);

                        // 시작·정지가 진행 중인지는 데몬이 안다(`transition`, 어디서 시켰든). 이 창이 누른 것은
                        // 데몬이 요청을 받기 전에도 보이게 요청이 끝날 때까지만 지킨다. 데몬이 그 값을 주지 않는 옛 데몬이면 끝난 뒤 60초까지
                        let mergedStatus = newServer.transition || newServer.status;
                        const opt = internal.optimistic.get(newServer.id);
                        if (opt) {
                            const elapsed = Date.now() - opt.timestamp;
                            // 0.1.14~ 데몬은 `update_check`를 늘 싣는다 (`transition`은 진행 중일 때만 있어 판별에 못 쓴다)
                            const GUARD_MS = newServer.update_check !== undefined ? 0 : 60_000;
                            const transitioned =
                                (opt.status === 'starting' && newServer.status === 'running') ||
                                (opt.status === 'stopping' && newServer.status === 'stopped');
                            if (transitioned || (elapsed > GUARD_MS && !opt.pending)) {
                                internal.optimistic.delete(newServer.id);
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
                    return reuseUnchanged(prev, holdSavingOrder(merged));
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
                            {
                                isNotice: true,
                                source: evt.stopped_name,
                                notice: {
                                    message: t('events.n_port_conflict', { port: evt.port, existing: evt.existing_name }),
                                    ...(evt.stopped_id ? { instanceId: evt.stopped_id } : {}),
                                },
                            },
                        );
                    }
                }
            } else if (data && data.error) {
                console.error('Server list error:', data.error);
                const translatedErr = translateError(data.error);
                const now = Date.now();
                if (translatedErr && !get().loading && now - internal.lastErrorToast > 5000) {
                    safeShowToast(t('servers.fetch_failed_toast', { error: translatedErr }), 'warning', 3000);
                    internal.lastErrorToast = now;
                }
            } else {
                if (get().loading) {
                    get().setServers([]);
                }
            }
        } catch (error) {
            console.error('Failed to fetch servers:', error);
            const errorMsg = translateError((error as Error).message);
            const now = Date.now();
            if (errorMsg && !get().loading && now - internal.lastErrorToast > 5000) {
                safeShowToast(t('servers.fetch_update_failed_toast', { error: errorMsg }), 'warning', 3000);
                internal.lastErrorToast = now;
            }
        } finally {
            set({ loading: false });
        }
    };

    // 격리 없이 돌리기 동의 (파일이 바뀌면 데몬이 다시 묻는다). 동의하면 retry를 다시 부른다
    const askUnisolated = (result: ApiBody, retry: () => void) => {
        const component = result.component;
        setModal({
            type: 'question',
            title: t('trust.sandbox.title'),
            message: t('trust.sandbox.message', {
                component,
                detail: result.error_code === 'isolation_unsupported' ? blockedText(result, t) : result.detail || result.message || '',
            }),
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
                            safeShowToast(t('trust.sandbox.allow_failed', { error: (err as Error).message }), 'error', 5000);
                        }
                    },
                },
                { label: t('modals.cancel'), action: () => setModal(null) },
            ],
        });
    };

    // 약관 동의 창 (module.toml [[agreements]]). 동의하면 이 PC에서 기록하고 다시 켠다
    const askAgreements = (srv: Server, title: string, message: string, agreements: unknown[]) => {
        setModal({
            type: 'agreements',
            title,
            message,
            module: srv.module,
            agreements,
            onAccept: async (ids: string[]) => {
                setModal(null);
                try {
                    const res = await window.api.instanceAgreements(srv.id, ids);
                    if (!res || res.success === false || res.error) {
                        safeShowToast(translateError(res?.error || ''), 'error', 5000);
                        return;
                    }
                    startServer(srv.id);
                } catch (err) {
                    safeShowToast(translateError((err as Error).message), 'error', 5000);
                }
            },
        });
    };

    // ── startServer ─────────────────────────────────────────
    // 데몬이 Windows 예약 범위를 피해 포트를 옮겨 저장했다 (시작·재시작, 성공·실패 모두): 조용히 바뀌지 않게 알린다
    const notifyPortMoved = (result: ApiBody | undefined, srv: Server) => {
        for (const m of Array.isArray(result?.port_moved) ? result.port_moved : []) {
            showPortMoved({ instanceId: srv.id, name: srv.name, from: m.from, to: m.to }, t);
        }
    };

    const startServer = async (id: string) => {
        try {
            const srv = get().servers.find((s) => s.id === id);
            if (!srv) {
                safeShowToast(t('servers.start_failed_toast', { error: t('servers.start_not_in_list') }), 'error', 4000);
                return;
            }
            const { name, module } = srv;

            // Optimistic update: 즉시 'starting' 상태 표시
            // 시작 요청이 끝날 때까지는(첫 시작은 몇 분) 목록 새로고침이 이 표시를 지우지 않는다
            internal.optimistic.set(id, { status: 'starting', timestamp: Date.now(), pending: true });
            get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, status: 'starting' } : s)));

            // 시작 방식(managed 여부)과 포트 충돌 검사는 데몬이 인스턴스·전역 설정으로 정한다
            const result = await window.api.instanceStart(id);
            const opt = internal.optimistic.get(id);
            if (opt?.pending) internal.optimistic.set(id, { status: 'starting', timestamp: Date.now() });
            notifyPortMoved(result, srv);

            // ── action_required: 서버 실행 파일 없음 (범용 코드, 예전 모듈은 server_jar_not_found) ──
            if (
                result.action_required === 'executable_not_found' ||
                result.action_required === 'server_jar_not_found'
            ) {
                // 파일 종류는 설정된 경로의 확장자로, 없으면 모듈이 밝힌 실행 파일 이름으로 (둘 다 없으면 모든 파일)
                const declared = get().modules.find((m) => m.name === module)?.server_executable;
                const configuredName = (result.configured_path || declared || '').split(/[/\\]/).pop();
                const ext = configuredName.includes('.') ? configuredName.split('.').pop() : '';
                revertOptimistic(id);
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
                                        // 고른 파일과 그 폴더: 이름이 모듈 기본값과 달라도, 예전 경로가 남아 있어도 이 파일로 켠다
                                        const dir = filePath.replace(/[/\\][^/\\]+$/, '');
                                        const saved = await window.api.instanceUpdateSettings(id, {
                                            working_dir: dir,
                                            executable_path: filePath,
                                        });
                                        if (!saved || saved.error || saved.success === false) {
                                            safeShowToast(translateError(saved?.error || saved?.message || ''), 'error', 4000);
                                            return;
                                        }
                                        safeShowToast(t('servers.jar_path_updated'), 'success', 3000);
                                        await fetchServers();
                                        startServer(id);
                                    }
                                } catch (err) {
                                    safeShowToast(translateError((err as Error).message), 'error', 4000);
                                }
                            },
                        },
                        moduleSupports(
                            get().modules.find((m) => m.name === module),
                            'install_server',
                        ) && {
                            label: t('servers.jar_action_install_new'),
                            action: async () => {
                                setModal(null);
                                try {
                                    const installDir = await window.api.openFolderDialog();
                                    if (!installDir) return;

                                    setProgressBar({ message: t('servers.progress_installing'), indeterminate: true });

                                    // 최신 버전은 데몬이 이 인스턴스 설정(서버 종류 등)으로 고른다 (L23)
                                    const installResult = await window.api.moduleInstallServer(module, {
                                        instance_id: id,
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
                                            get().modules.find((m) => m.name === module),
                                        ),
                                        version: installResult.version || '',
                                    });
                                    const msg = installResult.java_warning
                                        ? `${done}\n${installResult.java_warning}`
                                        : done;
                                    safeShowToast(msg, 'success', 5000);
                                    await fetchServers();

                                    if (!installResult.java_warning) {
                                        startServer(id);
                                    }
                                } catch (err) {
                                    setProgressBar(null);
                                    safeShowToast(translateError((err as Error).message), 'error', 4000);
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
                revertOptimistic(id);
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
                                ui().openSettings('extensions');
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
            const updateText = updateRequiredText(t, result, module);
            if (updateText) {
                revertOptimistic(id);
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
                revertOptimistic(id);
                const missing = result.missing_dependencies || [];
                const needsExtension = missing.some((d: string) => d.startsWith('ext-'));
                const title = t('servers.dependency_unmet_title');
                const message = `${t('servers.dependency_unmet_message', { name })}\n\n${missing.map((d: string) => `• ${d}`).join('\n')}`;
                if (needsExtension) {
                    setModal({
                        type: 'question',
                        title,
                        message,
                        buttons: [
                            {
                                label: t('servers.extension_open_settings'),
                                action: () => {
                                    setModal(null);
                                    ui().openSettings('extensions');
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
                revertOptimistic(id);
                const pending = Array.isArray(result.agreements) ? result.agreements : [];
                if (pending.length && !!window.api.instanceAgreements) {
                    askAgreements(srv, t('agreements.start_title'), t('agreements.start_message', { name }), pending);
                } else {
                    // 데몬 문구는 이미 현지화돼 있다
                    setModal({ type: 'failure', title: t('agreements.start_title'), message: result.error || '' });
                }
                return;
            }

            // ── error_code: preflight_failed — 켜기 전 점검(모듈 validate)이 critical 문제를 냈다 ──
            if (result.error_code === 'preflight_failed') {
                revertOptimistic(id);
                const issues = Array.isArray(result.issues) ? result.issues : [];
                const others = issues.slice(1).map((i: { message?: string; code?: string }) => `• ${i.message || i.code}`);
                const message = [result.error, result.solution, others.length ? others.join('\n') : null]
                    .filter(Boolean)
                    .join('\n\n');
                const title = t('servers.preflight_failed_title', { name });
                // 동의 기록은 있는데 파일(eula.txt 등)이 지워진 경우: 다시 동의하면 모듈이 파일을 다시 쓴다
                // 서버 목록에는 agreements가 없어 모듈 목록의 선언을 쓴다
                const srvAgreements = get().modules.find((m) => m.name === module)?.agreements || [];
                if (
                    result.issue_code === 'EULA_NOT_ACCEPTED' &&
                    srvAgreements.length &&
                    !!window.api.instanceAgreements
                ) {
                    askAgreements(srv, title, message, srvAgreements);
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

            // ── already_running — 이미 켜져 있다: "시작 완료" 대신 그렇다고만 알린다 (사용자 11-06 #10) ──
            if (result.success !== false && result.already_running === true) {
                revertOptimistic(id);
                safeShowToast(t('servers.already_running_toast', { name }), 'info', 3000);
                return;
            }

            // ── error_code: start_cancelled — 켜는 동안 사용자가 정지했다: 실패가 아니므로 알리지 않는다 ──
            if (result.error_code === 'start_cancelled') {
                revertOptimistic(id);
                return;
            }

            // ── error_code: port_conflict — 데몬이 포트 충돌 감지 ──
            if (result.error_code === 'port_conflict' || result.error === 'port_conflict') {
                revertOptimistic(id);
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
                revertOptimistic(id);
                askUnisolated(result, () => startServer(id));
                return;
            }

            // ── action_required: 확인되지 않은 구성요소라 데몬이 막음 (악성코드 검사·서버 폴더 밖 실행) ──
            const blocked = blockedText(result, t);
            if (blocked) {
                revertOptimistic(id);
                safeShowToast(t('servers.start_failed_toast', { error: blocked }), 'error', 8000);
                return;
            }

            // ── success=false without specific action_required ──
            if (result.success === false) {
                revertOptimistic(id);
                safeShowToast(translateError(moduleFailureText(t, result)), 'error', 5000);
                return;
            }

            if (result.error) {
                revertOptimistic(id);
                const errorMsg = translateError(result.error);
                safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
            } else {
                internal.guiOps.add(id);
                setProgressBar({ message: t('servers.starting_toast', { name }), indeterminate: true });
                if (result.managed) {
                    openConsole(id, name);
                }

                // 켜질 때까지 확인 (최대 30초). 이벤트가 있으면 그 서버의 이벤트가 올 때 묻는다
                let consecutiveStopped = 0;
                watchInstance({
                    instanceId: id,
                    limitMs: 30000,
                    check: async (ev: { type?: string } | null) => {
                        const statusResult = await window.api.instanceStatus(id, name);
                        if (statusResult.status === 'running') {
                            setProgressBar(null);
                            safeShowToast(t('servers.start_completed_toast', { name }), 'success', 3000);
                            fetchServers();
                            return true;
                        }
                        // 프로세스가 즉시 죽은 경우 조기 감지: 꺼짐 이벤트, 또는 연속 stopped 5회
                        if (statusResult.status === 'stopped') {
                            consecutiveStopped++;
                            if (ev?.type === 'instance.exit' || consecutiveStopped >= 5) {
                                setProgressBar(null);
                                revertOptimistic(id);
                                safeShowToast(
                                    t('servers.start_failed_toast', { error: t('servers.start_exited_immediately') }),
                                    'error',
                                    4000,
                                );
                                fetchServers();
                                return true;
                            }
                        } else {
                            consecutiveStopped = 0;
                        }
                        return false;
                    },
                    onTimeout: () => {
                        setProgressBar(null);
                        safeShowToast(t('servers.start_timeout_toast', { name }), 'warning', 3000);
                        fetchServers();
                    },
                });
            }
        } catch (error) {
            setProgressBar(null);
            revertOptimistic(id);
            const errorMsg = translateError((error as Error).message);
            safeShowToast(t('servers.start_failed_toast', { error: errorMsg }), 'error', 4000);
        }
    };

    // 콘솔도 REST도 없어 저장 후 종료를 요청할 수 없는 서버는 데몬이 그대로 둔다 (MK10). 저장 없이 끈다고 알리고 고를 때만 force로
    const askForceStop = (name: string, retry: () => void) =>
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

    // ── stopServer ──────────────────────────────────────────
    const stopServer = async (id: string) => {
        const srv = get().servers.find((s) => s.id === id);
        const name = srv?.name ?? id;
        const stopNow = async (forced: boolean) => {
            const undo = () => {
                internal.guiOps.delete(id);
                revertOptimistic(id);
            };
            try {
                // Optimistic update: 즉시 'stopping' 상태 표시
                // 데몬은 stop_timeout까지 기다린 뒤 답하므로, 요청이 끝날 때까지는 목록 새로고침이 이 표시를 지우지 않는다.
                // 그사이 꺼진 것도 GUI가 한 정지로 본다
                internal.optimistic.set(id, { status: 'stopping', timestamp: Date.now(), pending: true });
                internal.guiOps.add(id);
                get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, status: 'stopping' } : s)));

                const useGraceful = srv?.module_settings?.graceful_stop;
                const forceStop = forced || useGraceful === false;

                const result = await window.api.instanceStop(id, { force: forceStop });
                const opt = internal.optimistic.get(id);
                if (opt?.pending) internal.optimistic.set(id, { status: 'stopping', timestamp: Date.now() });

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
                                    ui().openSettings('extensions');
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
                        blocked ? t('servers.stop_failed_toast', { error: blocked }) : moduleFailureText(t, result),
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
                    // 콘솔은 닫지 않는다: 꺼진 뒤에도 지난 로그를 보인다 (폴링이 디스크 꼬리에서 못 본 줄만 잇는다)
                    setProgressBar({ message: t('servers.stopping_toast', { name }), indeterminate: true });

                    // 꺼질 때까지 확인 (최대 10초). 이벤트가 있으면 그 서버의 이벤트가 올 때 묻는다
                    watchInstance({
                        instanceId: id,
                        limitMs: 10000,
                        check: async () => {
                            const statusResult = await window.api.instanceStatus(id, name);
                            if (statusResult.status !== 'stopped') return false;
                            setProgressBar(null);
                            safeShowToast(t('servers.stop_completed_toast', { name }), 'success', 3000);
                            fetchServers();
                            return true;
                        },
                        onTimeout: () => {
                            setProgressBar(null);
                            safeShowToast(t('servers.stop_timeout_toast', { name }), 'warning', 3000);
                            fetchServers();
                        },
                    });
                }
            } catch (error) {
                setProgressBar(null);
                undo();
                const errorMsg = translateError((error as Error).message);
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

    // ── restartServer: 데몬이 프로세스가 끝나길 기다렸다가 다시 켠다 ──
    // confirmed: 부른 곳에서 이미 재시작을 골랐다 (설정 저장 뒤 '지금 다시 시작') → 다시 묻지 않는다
    const restartServer = async (id: string, { confirmed = false }: { confirmed?: boolean } = {}) => {
        const srv = get().servers.find((s) => s.id === id);
        if (!srv || !window.api.instanceRestart) return;
        const { name } = srv;
        const restartNow = async (forced: boolean) => {
            internal.optimistic.set(id, { status: 'stopping', timestamp: Date.now() });
            get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, status: 'stopping' } : s)));
            internal.guiOps.add(id);
            setProgressBar({ message: t('servers.restarting_toast', { name }), indeterminate: true });
            try {
                const force = forced || srv.module_settings?.graceful_stop === false;
                const result = await window.api.instanceRestart(id, { force });
                notifyPortMoved(result, srv);
                const code = result?.error_code;
                if (code === 'graceful_stop_unavailable') {
                    revertOptimistic(id);
                    askForceStop(name, () => restartNow(true));
                } else if (code === 'start_cancelled' || code === 'restart_cancelled') {
                    // 그사이 다른 쪽에서 정지·시작·삭제했다: 실패가 아니라 그만둔 것
                    revertOptimistic(id);
                    safeShowToast(t(`errors.${code}`), 'info', 4000);
                } else if (code === 'still_stopping') {
                    // 서버가 제때 꺼지지 않아 데몬이 다시 켜지 않았다
                    revertOptimistic(id);
                    safeShowToast(t('errors.still_stopping'), 'error', 4000);
                } else if (result?.error || result?.success === false) {
                    revertOptimistic(id);
                    const msg = translateError(result.error || result.message || '');
                    safeShowToast(t('servers.restart_failed_toast', { error: msg }), 'error', 4000);
                } else {
                    // 데몬이 다시 켰다: 폴링이 짧은 정지 구간을 놓쳐도 '끄는 중'으로 남기지 않는다
                    internal.optimistic.delete(id);
                    safeShowToast(t('servers.restart_completed_toast', { name }), 'success', 3000);
                }
            } catch (error) {
                revertOptimistic(id);
                safeShowToast(
                    t('servers.restart_failed_toast', { error: translateError((error as Error).message) }),
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

    // ── renameServer: 꺼져 있을 때만 (데몬이 server_running으로 거절) ──
    const renameServer = async (id: string, newName: string) => {
        const name = (newName || '').trim();
        const srv = get().servers.find((s) => s.id === id);
        if (!srv || !name || name === srv.name || !window.api.instanceRename) return false;
        try {
            const result = await window.api.instanceRename(id, name);
            if (result?.success) {
                get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, name: result.name || name } : s)));
                fetchServers();
                return true;
            }
            const code = result?.error_code;
            const msg =
                code === 'name_taken' || code === 'server_running' || code === 'instance_busy'
                    ? t(`servers.rename_${code}`, { name })
                    : code === 'invalid_name'
                      ? t('servers.name_comma')
                      : translateError(result?.error || '');
            safeShowToast(t('servers.rename_failed_toast', { error: msg }), 'error', 4000);
        } catch (error) {
            safeShowToast(t('servers.rename_failed_toast', { error: translateError((error as Error).message) }), 'error', 4000);
        }
        return false;
    };

    // ── addServer ─────────────────────────────────────
    const addServer = async (payload: AddServerPayload, legacyModuleName?: string) => {
        // payload = { name, module_name, accept_agreements?, preset?, extension_data?, migration_source?, migration_mode?, module_settings?, version?, working_dir?, managed_start?, port? }
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
            const instanceData: ApiBody = {
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

            // 마법사에서 고른 버전·폴더·실행 방식·포트 (installFields.createBody가 고른 것만 싣는다)
            for (const key of ['version', 'working_dir', 'managed_start', 'port', 'branch_of'] as const) {
                if (payload?.[key] !== undefined) instanceData[key] = payload[key];
            }

            // 마이그레이션: 기존 서버 디렉토리 직접 연결
            if (payload?.migration_source) {
                instanceData.migration_source = payload.migration_source;
                // 그 자리에 연결(link) / 사바쨩으로 옮기기(move)
                if (payload.migration_mode) {
                    instanceData.migration_mode = payload.migration_mode;
                }
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
                                ui().openSettings('extensions');
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

            // 데몬은 같은 이름의 서버를 만들지 않는다 (디스코드·CLI는 서버를 이름으로 부른다)
            if (result.error_code === 'name_taken') {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t('servers.rename_name_taken', { name: serverName }),
                });
                return false;
            }

            // 쉼표가 든 이름 (디스코드에서 부르는 말을 쉼표로 나눈다)
            if (result.error_code === 'invalid_name') {
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: t('servers.name_comma') });
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

            // 가져오기 "옮기기"가 폴더를 옮기지 못함 (다른 드라이브, 켜진 서버·열린 파일 등): 아무것도 만들지 않았다
            if (['move_cross_volume', 'move_in_use', 'move_failed', 'source_missing', 'destination_exists'].includes(result.error_code as string)) {
                const key = ['move_cross_volume', 'move_in_use'].includes(result.error_code as string) ? result.error_code : 'move_failed';
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: t(`servers.${key}`) });
                return false;
            }

            // 고른 설정 값을 사바쨩이 설치하지 못한다 (모듈 `install_options` 밖)
            // 새 서버를 설치할 폴더: 그 사이 파일이 생겼거나 폴더를 만들지 못함 (만들기 마법사의 파일 위치)
            if (['folder_not_empty', 'folder_unavailable'].includes(result.error_code as string)) {
                setModal({
                    type: 'failure',
                    title: t('servers.add_failed_title'),
                    message: t(`add_server_modal.${result.error_code}`),
                });
                return false;
            }

            if (result.error_code === 'not_installable') {
                setModal({ type: 'failure', title: t('servers.add_failed_title'), message: t('servers.not_installable') });
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
                    ui().setShowModuleManager(false);
                    fetchServers();
                } else {
                    // 마이그레이션: 모듈이 판정한 서버 종류와 플러그인·모드
                    const detected = result.detected;
                    const components = (detected?.components || []).filter((c: { enabled?: boolean }) => c.enabled !== false);
                    const detectedLine = detected?.server_type
                        ? '\n' +
                          t('server_actions.server_detected', {
                              type: detected.server_type,
                              version: detected.version || '',
                              count: components.length,
                              names: components.length ? ` (${components.map((c: { name: string }) => c.name).join(', ')})` : '',
                          })
                        : '';
                    // 가져온 폴더에서 미리 알릴 것 (데몬 `notices`: 쓸 수 없는 폴더, 밖에서 켜 둔 서버)
                    const noticeLines = (result.notices || [])
                        .map((code: string) => '\n' + t(`server_actions.migrate_notice_${code}`, { defaultValue: '' }))
                        .filter((line: string) => line.trim())
                        .join('');
                    setModal({
                        type: 'success',
                        title: t('command_modal.success'),
                        message: t('server_actions.server_added', { name: serverName }) + detectedLine + noticeLines,
                    });
                    ui().setShowModuleManager(false);
                    fetchServers();
                }
            }
            return !result.error;
        } catch (error) {
            const errorMsg = translateError((error as Error).message);
            setModal({ type: 'failure', title: t('servers.add_error_title'), message: errorMsg });
            return false;
        }
    };

    // ── deleteServer ──────────────────────────────────
    const deleteServer = async (server: Server) => {
        // 가져온 서버(인스턴스 폴더 밖의 서버 폴더): 연결만 끊고 폴더는 남긴다. 보관한 원래 설정으로 되돌릴지 고른다
        if (server.linked_folder) {
            const remove = (keepSettings: boolean) => () => performDeleteServer(server, { keepSettings });
            setModal({
                type: 'question',
                title: t('server_actions.unlink_confirm_title'),
                message: t('server_actions.unlink_confirm_message', { name: server.name }),
                detail: t(
                    server.restorable_settings
                        ? 'server_actions.unlink_detail_restorable'
                        : 'server_actions.unlink_detail',
                    { path: server.linked_folder },
                ),
                buttons: server.restorable_settings
                    ? [
                          { label: t('server_actions.unlink_restore'), action: remove(false) },
                          { label: t('server_actions.unlink_keep_settings'), action: remove(true) },
                          { label: t('modals.cancel'), action: () => setModal(null) },
                      ]
                    : undefined,
                onConfirm: remove(false),
            });
            return;
        }
        // 옮겨 온 서버: 서버 파일은 함께 지워지지만, 서버 폴더 밖(데이터 폴더)에 남는 설정은 되돌릴지 고른다
        if (server.restorable_settings) {
            const remove = (keepSettings: boolean) => () => performDeleteServer(server, { keepSettings });
            setModal({
                type: 'question',
                title: t('server_actions.delete_confirm_title'),
                message: t('server_actions.delete_confirm_message', { name: server.name }),
                detail: t('server_actions.delete_detail_restorable'),
                buttons: [
                    { label: t('server_actions.delete_restore'), action: remove(false) },
                    { label: t('server_actions.delete_keep_settings'), action: remove(true) },
                    { label: t('modals.cancel'), action: () => setModal(null) },
                ],
                onConfirm: remove(false),
            });
            return;
        }
        setModal({
            type: 'question',
            title: t('server_actions.delete_confirm_title'),
            message: t('server_actions.delete_confirm_message', { name: server.name }),
            detail: t('server_actions.delete_detail'),
            onConfirm: () => performDeleteServer(server),
        });
    };

    const performDeleteServer = async (server: Server, options?: { keepSettings?: boolean }) => {
        setModal(null);
        try {
            const result = await window.api.instanceDelete(server.id, options);
            if (result.error_code === 'ETIMEDOUT') {
                // 답이 늦었을 뿐 데몬은 계속 지우고 있을 수 있다: 목록에 남았는지로 판단한다
                const list = await window.api.serverList().catch(() => null);
                const checked = Array.isArray(list?.servers);
                const gone = checked && !list?.servers?.some((s) => s.id === server.id);
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
                const message = result.linked_folder
                    ? t('server_actions.server_unlinked', { name: server.name, path: result.linked_folder }) +
                      (result.settings_restore_failed ? '\n' + unrestoredNote(result) : '')
                    : t('server_actions.server_deleted', { name: server.name }) +
                      (result.settings_restore_failed ? '\n' + unrestoredNote(result) : '');
                setModal({ type: 'success', title: t('command_modal.success'), message });
                fetchServers();
            }
        } catch (error) {
            const errorMsg = translateError((error as Error).message);
            setModal({ type: 'failure', title: t('servers.delete_error_title'), message: errorMsg });
        }
    };

    return {
        fetchServers,
        startServer,
        stopServer,
        restartServer,
        renameServer,
        addServer,
        deleteServer,
    };
}
