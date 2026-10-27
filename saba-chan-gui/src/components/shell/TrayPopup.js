import clsx from 'clsx';
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { playersText, primaryAction, STATUS_TONE, viewStatus } from '../../utils/serverView';
import { Icon } from '../Icon';
import { StatusChip } from '../ui/StatusChip';
import './TrayPopup.css';

/** 팝업이 열려 있는 동안 데몬·서버 상태를 다시 읽는 간격 */
const REFRESH_MS = 3000;

/**
 * 트레이 아이콘을 누르면 아이콘 옆에 뜨는 작은 창 (셸 window.rs `toggle_tray_popup`).
 * 서버 시작·정지는 데몬 API로, 창 열기·CLI로 전환·데몬 켜고 끄기·완전히 종료는 셸(`tray_action`)로 보낸다.
 * 내용 높이를 재서 셸에 알리면 셸이 크기와 자리를 정하고 보인다. 포커스를 잃거나 Esc면 닫힌다.
 * 상단 바에서 원격 데몬을 골랐으면 서버 요청이 그 데몬으로 가므로, 상태 칩은 그 데몬의 이름과 닿는지를 보이고
 * 그 데몬의 서버 정지는 한 번 더 눌러 확정한다. 데몬 켜기·끄기는 늘 이 PC의 데몬이다.
 */
export function TrayPopup() {
    const { t } = useTranslation('gui');
    const rootRef = useRef(null);
    const [daemonUp, setDaemonUp] = useState(null);
    const [servers, setServers] = useState([]);
    const [canCli, setCanCli] = useState(false);
    const [busy, setBusy] = useState({});
    const [failed, setFailed] = useState({});
    // 고른 원격 데몬: {name, reachable}, 이 PC면 null
    const [remote, setRemote] = useState(null);
    const [armed, setArmed] = useState(null); // 두 번 눌러 확정할 정지 (원격 서버 id)

    const refresh = useCallback(async () => {
        const [status, current] = await Promise.all([
            Promise.resolve(window.api?.daemonStatus?.()).catch(() => null),
            Promise.resolve(window.api?.daemonTarget?.()).catch(() => null),
        ]);
        const up = status?.running === true;
        const target = current?.target || null;
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
            const saved = Array.isArray(daemons?.daemons) ? daemons.daemons.find((d) => d.hostId === target) : null;
            setRemote({ name: saved?.name || target, reachable });
            // 닿지 않는 원격 데몬의 지난 목록을 남기지 않는다
            if (!reachable) setServers([]);
        } else {
            setRemote(null);
        }
        if (reachable) setServers(list.servers);
    }, []);

    useEffect(() => {
        refresh();
        Promise.resolve(window.api?.clientsLaunchable?.())
            .then((r) => setCanCli(r?.cli === true))
            .catch(() => {});
        const timer = setInterval(refresh, REFRESH_MS);
        const onKey = (e) => {
            if (e.key === 'Escape') window.api?.trayAction?.('close');
        };
        window.addEventListener('keydown', onKey);
        return () => {
            clearInterval(timer);
            window.removeEventListener('keydown', onKey);
        };
    }, [refresh]);

    // 내용이 바뀔 때마다 높이를 셸에 알린다 (첫 알림에 창이 보인다)
    useLayoutEffect(() => {
        const el = rootRef.current;
        if (!el) return undefined;
        const report = () => window.api?.trayPopupFit?.(Math.ceil(el.getBoundingClientRect().height));
        report();
        if (typeof ResizeObserver === 'undefined') return undefined;
        const observer = new ResizeObserver(report);
        observer.observe(el);
        return () => observer.disconnect();
    }, []);

    const act = (action) => window.api?.trayAction?.(action);

    const toggleServer = async (server, action) => {
        if (action === 'stop' && remote && armed !== server.id) {
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
        <div className="tray-popup" ref={rootRef}>
            <header className="tray-popup-head">
                <img className="tray-popup-logo" src="favicon.png" alt="" />
                <span className="tray-popup-title">{t('tray_popup.title')}</span>
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

            {daemonUp &&
                remote?.reachable !== false &&
                (servers.length > 0 ? (
                    <ul className="tray-popup-servers" aria-label={t('tray_popup.servers')}>
                        {servers.map((server) => {
                            // 팝업은 모듈 목록을 읽지 않는다: "모듈 없음"으로 판정하지 않는다
                            const status = viewStatus(server, true);
                            const action = busy[server.id] ? 'busy' : primaryAction(status);
                            const players = playersText(server);
                            const confirming = armed === server.id && action === 'stop';
                            return (
                                <li key={server.id} className="tray-popup-server" data-status={status}>
                                    <span
                                        className={clsx('tray-popup-dot', `tone-${STATUS_TONE[status]}`)}
                                        aria-hidden="true"
                                    />
                                    <span className="tray-popup-server-name">{server.name}</span>
                                    <span className={clsx('tray-popup-server-meta', { failed: failed[server.id] })}>
                                        {failed[server.id]
                                            ? t('tray_popup.action_failed')
                                            : players || t(`shell.status_${status}`)}
                                    </span>
                                    {confirming ? (
                                        <button
                                            type="button"
                                            className="btn btn-danger btn-sm tray-popup-server-confirm"
                                            aria-label={t('tray_popup.stop_confirm_aria', { name: server.name })}
                                            onClick={() => toggleServer(server, action)}
                                        >
                                            <Icon name="stop" size={12} />
                                            {t('tray_popup.stop_confirm')}
                                        </button>
                                    ) : (
                                        <button
                                            type="button"
                                            className="icon-btn tray-popup-server-action"
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
                ) : (
                    <p className="tray-popup-empty">{t('tray_popup.no_servers')}</p>
                ))}

            <div className="tray-popup-actions">
                <button type="button" className="btn-primary btn-sm" onClick={() => act('open_window')}>
                    <Icon name="monitor" size={14} />
                    {t('tray_popup.open_window')}
                </button>
                {canCli && (
                    <button type="button" className="btn-secondary btn-sm" onClick={() => act('switch_to_cli')}>
                        <Icon name="terminal" size={14} />
                        {t('tray_popup.switch_to_cli')}
                    </button>
                )}
            </div>

            <footer className="tray-popup-foot">
                {/* 상태를 읽기 전(데몬이 꺼져 있으면 1~2초)에는 켜기·끄기를 그리지 않는다 */}
                {daemonUp === false && (
                    <button type="button" className="tray-popup-link" onClick={() => act('start_daemon')}>
                        <Icon name="play" size={12} />
                        {t('tray_popup.start_daemon')}
                    </button>
                )}
                {daemonUp === true && (
                    <button type="button" className="tray-popup-link" onClick={() => act('stop_daemon')}>
                        <Icon name="stop" size={12} />
                        {t('tray_popup.stop_daemon')}
                    </button>
                )}
                <button type="button" className="tray-popup-link danger" onClick={() => act('quit_all')}>
                    <Icon name="power" size={12} />
                    {t('tray_popup.quit_all')}
                </button>
            </footer>
        </div>
    );
}
