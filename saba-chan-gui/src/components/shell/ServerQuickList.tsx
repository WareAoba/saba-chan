import clsx from 'clsx';
import { type RefObject, useCallback, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ServerInfo } from '../../types/daemon';
import { isTransitional, playersText, primaryAction, STATUS_TONE, viewStatus } from '../../utils/serverView';
import { Icon } from '../Icon';
import { StatusChip } from '../ui/StatusChip';

/** 고른 원격 데몬의 이름과 닿는지 */
export interface RemoteState {
    name: string;
    reachable: boolean;
}

/**
 * 작은 판 창(트레이 팝업·펫 우클릭 판)이 보이는 데몬 상태와 서버 목록.
 * 상단 바에서 원격 데몬을 골랐으면 목록이 그 데몬의 것이라, 그 데몬의 이름과 닿는지(`remote`)를 함께 본다.
 * 닿지 않는 원격 데몬의 지난 목록은 남기지 않는다
 */
export function useQuickServers() {
    const [daemonUp, setDaemonUp] = useState<boolean | null>(null);
    const [servers, setServers] = useState<ServerInfo[]>([]);
    const [remote, setRemote] = useState<RemoteState | null>(null);
    /** 마지막으로 확인한 고른 데몬: 이 PC면 null, 아직 모르면 undefined (익스텐션 host.run이 본다) */
    const targetRef = useRef<string | null | undefined>(undefined);

    const refresh = useCallback(async () => {
        const [status, current] = await Promise.all([
            Promise.resolve(window.api?.daemonStatus?.()).catch(() => null),
            Promise.resolve(window.api?.daemonTarget?.()).catch(() => null),
        ]);
        const up = status?.running === true;
        const target = current?.target || null;
        // 고를 수 없는 창이면 늘 이 PC 데몬, 물었는데 답을 못 받았으면 모른다
        targetRef.current =
            typeof window.api?.daemonTarget !== 'function' ? null : current && 'target' in current ? target : undefined;
        setDaemonUp(up);
        if (!up) {
            setRemote(null);
            setServers([]);
            return;
        }
        const [list, daemons] = await Promise.all([
            Promise.resolve(window.api?.serverList?.()).catch(() => null),
            target ? Promise.resolve(window.api?.remoteDaemons?.()).catch(() => null) : null,
        ]);
        const reachable = Array.isArray(list?.servers);
        if (target) {
            const saved = Array.isArray(daemons?.daemons)
                ? daemons.daemons.find((d: { hostId: string }) => d.hostId === target)
                : null;
            setRemote({ name: saved?.label || saved?.name || target, reachable });
            if (!reachable) setServers([]);
        } else {
            setRemote(null);
        }
        if (reachable) setServers(list.servers as ServerInfo[]);
    }, []);

    return { daemonUp, servers, remote, targetRef, refresh };
}

/** 판 내용 높이(논리 px)가 바뀔 때마다 셸에 알린다. 셸은 첫 알림에 판을 놓고 보인다 */
export function usePanelFit(rootRef: RefObject<HTMLElement | null>, report: (height: number) => void) {
    const reportRef = useRef(report);
    reportRef.current = report;
    useLayoutEffect(() => {
        const el = rootRef.current;
        if (!el) return undefined;
        const send = () => reportRef.current(Math.ceil(el.getBoundingClientRect().height));
        send();
        if (typeof ResizeObserver === 'undefined') return undefined;
        const observer = new ResizeObserver(send);
        observer.observe(el);
        return () => observer.disconnect();
    }, [rootRef]);
}

/** 판 머리: 사바쨩과 데몬 상태 (원격 데몬을 골랐으면 그 이름과 닿는지) */
export function QuickPanelHead({ daemonUp, remote }: { daemonUp: boolean | null; remote: RemoteState | null }) {
    const { t } = useTranslation('gui');
    return (
        <header className="quick-panel-head">
            <img className="quick-panel-logo" src="favicon.png" alt="" />
            <span className="quick-panel-title">{t('tray_popup.title')}</span>
            {daemonUp && remote && (
                <StatusChip
                    tone={remote.reachable ? 'ok' : 'warn'}
                    label={t(remote.reachable ? 'tray_popup.remote_connected' : 'tray_popup.remote_unreachable', {
                        name: remote.name,
                    })}
                    className="bare"
                />
            )}
            {daemonUp !== null && !(daemonUp && remote) && (
                <StatusChip
                    tone={daemonUp ? 'ok' : 'stop'}
                    label={daemonUp ? t('tray_popup.daemon_running') : t('tray_popup.daemon_stopped')}
                    className="bare"
                />
            )}
        </header>
    );
}

interface ServerQuickListProps {
    servers: ServerInfo[];
    /** 정지를 한 번 더 눌러 확정한다 (원격 데몬의 서버, 펫 판은 늘) */
    confirmStop: boolean;
    /** 시작·정지 뒤 목록을 다시 읽는다 */
    refresh: () => void;
}

/** 서버 줄: 상태 점·이름·접속자 수(켜진 서버)나 상태, 시작·정지 버튼. 판은 모듈 목록을 읽지 않는다 */
export function ServerQuickList({ servers, confirmStop, refresh }: ServerQuickListProps) {
    const { t } = useTranslation('gui');
    const [busy, setBusy] = useState<Record<string, boolean>>({});
    const [failed, setFailed] = useState<Record<string, boolean>>({});
    const [armed, setArmed] = useState<string | null>(null); // 두 번 눌러 확정할 정지 (서버 id)

    if (servers.length === 0) return <p className="quick-panel-empty">{t('tray_popup.no_servers')}</p>;

    const toggleServer = async (server: ServerInfo, action: string) => {
        if (action === 'stop' && confirmStop && armed !== server.id) {
            setArmed(server.id);
            return;
        }
        setArmed(null);
        setBusy((b) => ({ ...b, [server.id]: true }));
        setFailed((f) => ({ ...f, [server.id]: false }));
        try {
            const res =
                action === 'stop'
                    ? await window.api.instanceStop(server.id)
                    : await window.api.instanceStart(server.id);
            if (res?.error || res?.success === false) setFailed((f) => ({ ...f, [server.id]: true }));
        } catch {
            setFailed((f) => ({ ...f, [server.id]: true }));
        }
        setBusy((b) => ({ ...b, [server.id]: false }));
        refresh();
    };

    return (
        <ul className="quick-panel-servers" aria-label={t('tray_popup.servers')}>
            {servers.map((server) => {
                // 모듈 목록을 읽지 않는다: "모듈 없음"으로 판정하지 않는다
                const status = viewStatus(server, true);
                const action = busy[server.id] ? 'busy' : primaryAction(status);
                const players = playersText(server);
                const confirming = armed === server.id && action === 'stop';
                return (
                    <li key={server.id} className="quick-panel-server" data-status={status}>
                        <span
                            className={clsx('quick-panel-dot', `tone-${STATUS_TONE[status]}`, {
                                pulse: isTransitional(status),
                            })}
                            aria-hidden="true"
                        />
                        <span className="quick-panel-server-name">{server.name}</span>
                        <span className={clsx('quick-panel-server-meta', { failed: failed[server.id] })}>
                            {failed[server.id] ? t('tray_popup.action_failed') : players || t(`shell.status_${status}`)}
                        </span>
                        {confirming ? (
                            <button
                                type="button"
                                className="btn btn-danger btn-sm quick-panel-server-confirm"
                                aria-label={t('tray_popup.stop_confirm_aria', { name: server.name })}
                                onClick={() => toggleServer(server, action)}
                            >
                                <Icon name="stop" size={12} />
                                {t('tray_popup.stop_confirm')}
                            </button>
                        ) : (
                            <button
                                type="button"
                                className="icon-btn quick-panel-server-action"
                                disabled={action === 'busy'}
                                aria-label={t(action === 'stop' ? 'tray_popup.stop' : 'tray_popup.start', {
                                    name: server.name,
                                })}
                                onClick={() => action !== 'busy' && toggleServer(server, action)}
                            >
                                <Icon name={action === 'stop' ? 'stop' : 'play'} size={14} />
                            </button>
                        )}
                    </li>
                );
            })}
        </ul>
    );
}
