import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type HostWindow, HostWindowContext } from '../../extensions/host';
import ExtensionSlot from '../ExtensionSlot';
import { Icon } from '../Icon';
import './QuickPanel.css';
import { QuickPanelHead, ServerQuickList, usePanelFit, useQuickServers } from './ServerQuickList';

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
    const rootRef = useRef<HTMLDivElement>(null);
    const [canCli, setCanCli] = useState(false);
    const { daemonUp, servers, remote, targetRef, refresh } = useQuickServers();

    useEffect(() => {
        refresh();
        Promise.resolve(window.api?.clientsLaunchable?.())
            .then((r) => setCanCli(r?.cli === true))
            .catch(() => {});
        const timer = setInterval(refresh, REFRESH_MS);
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') window.api?.trayAction?.('close');
        };
        window.addEventListener('keydown', onKey);
        return () => {
            clearInterval(timer);
            window.removeEventListener('keydown', onKey);
        };
    }, [refresh]);

    // 내용이 바뀔 때마다 높이를 셸에 알린다 (첫 알림에 창이 보인다)
    usePanelFit(rootRef, (height) => window.api?.trayPopupFit?.(height));

    const act = (action: string) => window.api?.trayAction?.(action);

    // 익스텐션 칸(`TrayPopup.section`)의 host: 원격 데몬 서버의 정지·재시작은 이 창에서 두 번 눌러 확정하므로 익스텐션은 하지 못한다.
    // 이 PC 데몬이라고 확인한 때(targetRef null)에만 보낸다. 아직 모르면(undefined) 원격으로 본다
    const serversRef = useRef(servers);
    serversRef.current = servers;
    const hostWindow = useMemo<HostWindow>(
        () => ({
            servers: () => serversRef.current,
            run: async (instanceId, action) => {
                if (action !== 'start' && targetRef.current !== null) return false;
                const call =
                    action === 'start'
                        ? window.api.instanceStart
                        : action === 'stop'
                          ? window.api.instanceStop
                          : window.api.instanceRestart;
                if (typeof call !== 'function') return false;
                try {
                    const res = await call(instanceId);
                    return !(res?.error || res?.success === false);
                } catch {
                    return false;
                } finally {
                    refresh();
                }
            },
            openApp: () => window.api?.trayAction?.('open_window'),
        }),
        [refresh, targetRef],
    );

    return (
        <div className="quick-panel tray-popup" ref={rootRef}>
            <QuickPanelHead daemonUp={daemonUp} remote={remote} />

            {daemonUp && remote?.reachable !== false && (
                <ServerQuickList servers={servers} confirmStop={!!remote} refresh={refresh} />
            )}

            {daemonUp && (
                <HostWindowContext.Provider value={hostWindow}>
                    <div className="quick-panel-ext">
                        <ExtensionSlot slotId="TrayPopup.section" servers={servers} t={t} />
                    </div>
                </HostWindowContext.Provider>
            )}

            <div className="quick-panel-actions">
                <button type="button" className="btn btn-primary btn-sm" onClick={() => act('open_window')}>
                    <Icon name="monitor" size={14} />
                    {t('tray_popup.open_window')}
                </button>
                {canCli && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => act('switch_to_cli')}>
                        <Icon name="terminal" size={14} />
                        {t('tray_popup.switch_to_cli')}
                    </button>
                )}
            </div>

            <footer className="quick-panel-foot">
                {/* 상태를 읽기 전(데몬이 꺼져 있으면 1~2초)에는 켜기·끄기를 그리지 않는다 */}
                {daemonUp === false && (
                    <button type="button" className="quick-panel-link" onClick={() => act('start_daemon')}>
                        <Icon name="play" size={12} />
                        {t('tray_popup.start_daemon')}
                    </button>
                )}
                {daemonUp === true && (
                    <button type="button" className="quick-panel-link" onClick={() => act('stop_daemon')}>
                        <Icon name="stop" size={12} />
                        {t('tray_popup.stop_daemon')}
                    </button>
                )}
                <button type="button" className="quick-panel-link danger" onClick={() => act('quit_all')}>
                    <Icon name="power" size={12} />
                    {t('tray_popup.quit_all')}
                </button>
            </footer>
        </div>
    );
}
