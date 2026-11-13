import type { Agreement, ApiBody } from '../types/daemon';
import { failed } from '../utils/apiResult';
import { showPortMoved, watchInstance } from '../utils/daemonEvents';
import { moduleFailureText, moduleSupports, safeShowToast } from '../utils/helpers';
import { controlOutcome, installFailure } from '../utils/serverControl';
import { gameNameOf } from '../utils/serverView';
import { blockedText, canAskUnisolated } from '../utils/trustNotice';
import { updateRequiredText } from '../utils/updateRequired';
import { internal, openConsole, ui } from './serverActionsState';
import { t, translateError } from './serverI18n';
import type { Server, ServerState } from './useServerStore';
import { setModal, setProgressBar, useUIStore } from './useUIStore';

/** 켜기·끄기·다시 시작: 약관·격리 동의, 실행 파일 찾기, 관리 모드 콘솔 열기를 함께 한다 */
export function serverPowerActions(get: () => ServerState, fetchServers: () => Promise<void>) {
    /** Optimistic 상태 해제 후 fetchServers로 실제 상태 복원 */
    const revertOptimistic = (id: string) => {
        internal.optimistic.delete(id);
        fetchServers();
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
                            if (failed(res)) {
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
    const askAgreements = (srv: Server, title: string, message: string, agreements: Agreement[]) => {
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
                    if (failed(res)) {
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
            internal.optimistic.set(id, { status: 'starting', pending: true });
            get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, status: 'starting' } : s)));

            // 시작 방식(managed 여부)과 포트 충돌 검사는 데몬이 인스턴스·전역 설정으로 정한다
            const result = await window.api.instanceStart(id);
            const outcome = controlOutcome(result);
            const opt = internal.optimistic.get(id);
            if (opt?.pending) internal.optimistic.set(id, { status: 'starting' });
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
                                        if (failed(saved)) {
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

                                    const failure = installFailure(t, installResult, translateError);
                                    if (failure) {
                                        setProgressBar(null);
                                        safeShowToast(failure, 'error', 8000);
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
                                    // 모듈이 붙인 주의 한 줄 (`warning`, 데몬의 설치 진행 알림과 같은 키)
                                    const msg = installResult.warning ? `${done}\n${installResult.warning}` : done;
                                    safeShowToast(msg, 'success', 5000);
                                    await fetchServers();

                                    if (!installResult.warning) {
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
            if (outcome.kind === 'agreements') {
                revertOptimistic(id);
                if (outcome.agreements.length && !!window.api.instanceAgreements) {
                    askAgreements(srv, t('agreements.start_title'), t('agreements.start_message', { name }), outcome.agreements);
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
            if (outcome.kind === 'alreadyRunning') {
                revertOptimistic(id);
                safeShowToast(t('servers.already_running_toast', { name }), 'info', 3000);
                return;
            }

            // ── error_code: start_cancelled — 켜는 동안 사용자가 정지했다: 실패가 아니므로 알리지 않는다 ──
            if (outcome.kind === 'cancelled') {
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

    // ── stopServer ──────────────────────────────────────────
    const stopServer = async (id: string) => {
        const srv = get().servers.find((s) => s.id === id);
        const name = srv?.name ?? id;
        const stopNow = async () => {
            const undo = () => {
                internal.guiOps.delete(id);
                revertOptimistic(id);
            };
            try {
                // Optimistic update: 즉시 'stopping' 상태 표시
                // 데몬은 stop_timeout까지 기다린 뒤 답하므로, 요청이 끝날 때까지는 목록 새로고침이 이 표시를 지우지 않는다.
                // 그사이 꺼진 것도 GUI가 한 정지로 본다
                internal.optimistic.set(id, { status: 'stopping', pending: true });
                internal.guiOps.add(id);
                get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, status: 'stopping' } : s)));

                const useGraceful = srv?.module_settings?.graceful_stop;
                const result = await window.api.instanceStop(id, { force: useGraceful === false });
                const opt = internal.optimistic.get(id);
                if (opt?.pending) internal.optimistic.set(id, { status: 'stopping' });

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
                await stopNow();
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
        const restartNow = async () => {
            internal.optimistic.set(id, { status: 'stopping' });
            get().setServers((prev) => prev.map((s) => (s.id === id ? { ...s, status: 'stopping' } : s)));
            internal.guiOps.add(id);
            setProgressBar({ message: t('servers.restarting_toast', { name }), indeterminate: true });
            try {
                const force = srv.module_settings?.graceful_stop === false;
                const result = await window.api.instanceRestart(id, { force });
                notifyPortMoved(result, srv);
                const outcome = controlOutcome(result);
                if (outcome.kind === 'cancelled') {
                    revertOptimistic(id);
                    safeShowToast(t(`errors.${outcome.code}`), 'info', 4000);
                } else if (outcome.kind === 'stillStopping') {
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
        if (confirmed) return restartNow();
        setModal({
            type: 'question',
            title: t('servers.restart_confirm_title'),
            message: t('servers.restart_confirm_message', { name }),
            onConfirm: async () => {
                setModal(null);
                await restartNow();
            },
            onCancel: () => setModal(null),
        });
    };

    return { startServer, stopServer, restartServer };
}
