import type { StoreApi } from 'zustand';
import { holdSavingOrder } from '../hooks/useDragReorder';
import { claimServerNotice, eventsLive } from '../utils/daemonEvents';
import { retryWithBackoff, safeShowToast } from '../utils/helpers';
import { reuseUnchanged } from '../utils/serverView';
import { internal } from './serverActionsState';
import { t, translateError } from './serverI18n';
import type { Server, ServerState } from './useServerStore';

/** 서버 목록 받기: 상태가 밖에서 바뀌면(멈춤·바깥 켜기·끄기) 알린다 */
export function serverListActions(set: StoreApi<ServerState>['setState'], get: () => ServerState) {
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
                        // 데몬이 요청을 받기 전에도 보이게 요청이 끝날 때까지만 지킨다
                        let mergedStatus = newServer.transition || newServer.status;
                        const opt = internal.optimistic.get(newServer.id);
                        if (opt) {
                            const transitioned =
                                (opt.status === 'starting' && newServer.status === 'running') ||
                                (opt.status === 'stopping' && newServer.status === 'stopped');
                            if (transitioned || !opt.pending) {
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
    return { fetchServers };
}
