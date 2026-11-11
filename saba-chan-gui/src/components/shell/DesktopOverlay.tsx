import type { TFunction } from 'i18next';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { OverlayMenuItem } from '../../api/createApi';
import { type HostWindow, HostWindowContext, isAppRoute, type ServerAction } from '../../extensions/host';
import type { ServerInfo } from '../../types/daemon';
import { SERVER_CHANGING, subscribeDaemonEvents } from '../../utils/daemonEvents';
import { primaryAction, viewStatus } from '../../utils/serverView';
import ExtensionSlot from '../ExtensionSlot';
import './DesktopOverlay.css';

/** 서버 목록을 다시 읽는 간격 (이벤트로도 다시 읽는다) */
const REFRESH_MS = 10_000;
/** 누른 채 이만큼 움직이면 클릭이 아니라 창 끌기 (px) */
const DRAG_THRESHOLD = 4;
/** 메뉴에 넣는 서버 수 */
const MENU_SERVERS = 12;
const ACTIONS: ServerAction[] = ['start', 'stop', 'restart'];
/** 메뉴에 쓰는 서버 이름 글자 수: 셸은 80자가 넘는 항목이 있으면 메뉴를 통째로 거절한다 */
const MENU_NAME_MAX = 40;

const shortName = (name: string) => {
    const chars = Array.from(name);
    return chars.length > MENU_NAME_MAX ? `${chars.slice(0, MENU_NAME_MAX - 1).join('')}…` : name;
};

/**
 * 우클릭 메뉴 항목 (셸이 OS 메뉴로 띄운다). 서버마다 하위 메뉴: 꺼져 있으면 시작, 켜져 있으면 정지·재시작을
 * 한 번 더 골라야 한다 (작은 창에서 잘못 누르기 쉽다). 켜고 끄는 중이면 고를 것이 없다
 */
export function overlayMenuItems(servers: ServerInfo[], t: TFunction): OverlayMenuItem[] {
    const items: OverlayMenuItem[] = servers.slice(0, MENU_SERVERS).map((server) => {
        // 이 창은 모듈 목록을 읽지 않는다: "모듈 없음"으로 판정하지 않는다
        const status = viewStatus(server, true);
        const action = primaryAction(status);
        const confirm = (what: 'stop' | 'restart') => ({
            label: t(`desktop_overlay.menu_${what}`),
            enabled: action === 'stop',
            items: [{ id: `server:${server.id}:${what}`, label: t(`desktop_overlay.menu_${what}_confirm`) }],
        });
        return {
            label: t('desktop_overlay.menu_server', {
                name: shortName(server.name),
                status: t(`shell.status_${status}`),
            }),
            items: [
                {
                    id: `server:${server.id}:start`,
                    label: t('desktop_overlay.menu_start'),
                    enabled: action === 'start',
                },
                confirm('stop'),
                confirm('restart'),
            ],
        };
    });
    if (items.length === 0) items.push({ label: t('desktop_overlay.menu_no_servers'), enabled: false });
    items.push(
        { separator: true },
        { id: 'open_app', label: t('desktop_overlay.menu_open_app') },
        { id: 'hide', label: t('desktop_overlay.menu_hide') },
    );
    return items;
}

/**
 * 바탕화면 오버레이 창 (`?desktop-overlay=1`, 셸 overlay.rs). 익스텐션 slot `Desktop.overlay`를 그리고,
 * 창 일은 여기서 한다: 누르면 앱 열기, 누른 채 움직이면 창 끌기, 우클릭은 서버 메뉴.
 * 내용 크기를 재서 셸에 알리면 셸이 아래 가운데 점을 지키며 크기를 맞추고 보인다.
 * 서버를 켜고 끄는 일은 데몬 API로 한다. 원격 데몬의 정지·재시작은 메뉴에서 한 번 더 고른 것만 하고,
 * 익스텐션(host.run)은 트레이 팝업처럼 이 PC 데몬의 서버만 정지·재시작할 수 있다
 */
export function DesktopOverlay() {
    const { t } = useTranslation('gui');
    const rootRef = useRef<HTMLDivElement>(null);
    const [servers, setServers] = useState<ServerInfo[]>([]);
    const serversRef = useRef(servers);
    serversRef.current = servers;
    /** 마지막으로 확인한 고른 데몬: 이 PC면 null, 아직 모르면 undefined */
    const targetRef = useRef<string | null | undefined>(undefined);

    const refresh = useCallback(async () => {
        const [status, current] = await Promise.all([
            Promise.resolve(window.api?.daemonStatus?.()).catch(() => null),
            Promise.resolve(window.api?.daemonTarget?.()).catch(() => null),
        ]);
        targetRef.current =
            typeof window.api?.daemonTarget !== 'function'
                ? null
                : current && 'target' in current
                  ? current.target || null
                  : undefined;
        if (status?.running !== true) {
            setServers([]);
            return;
        }
        const list = await Promise.resolve(window.api?.serverList?.()).catch(() => null);
        setServers(Array.isArray(list?.servers) ? (list.servers as ServerInfo[]) : []);
    }, []);

    useEffect(() => {
        refresh();
        const timer = setInterval(refresh, REFRESH_MS);
        const off = subscribeDaemonEvents((ev) => {
            if (SERVER_CHANGING.includes(ev.type)) refresh();
        });
        return () => {
            clearInterval(timer);
            off();
        };
    }, [refresh]);

    const runServer = useCallback(
        async (instanceId: string, action: ServerAction) => {
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
        [refresh],
    );

    const hostWindow = useMemo<HostWindow>(
        () => ({
            servers: () => serversRef.current,
            run: async (instanceId, action) => {
                if (action !== 'start' && targetRef.current !== null) return false;
                return runServer(instanceId, action);
            },
            // 고른 화면(서버·탭)은 셸이 메인 창에 바로 연다. 모르는 화면은 앱만 연다
            openApp: (route) =>
                isAppRoute(route)
                    ? window.api?.overlayAction?.('open_app', route)
                    : window.api?.overlayAction?.('open_app'),
        }),
        [runServer],
    );

    // 셸 메뉴에서 고른 항목
    useEffect(() => {
        if (typeof window.api?.onOverlayMenu !== 'function') return undefined;
        const off = window.api.onOverlayMenu((choice: unknown) => {
            if (choice === 'open_app' || choice === 'hide') {
                window.api?.overlayAction?.(choice);
                return;
            }
            const m = typeof choice === 'string' ? /^server:(.+):(start|stop|restart)$/.exec(choice) : null;
            if (!m || !ACTIONS.includes(m[2] as ServerAction)) return;
            if (!serversRef.current.some((s) => s.id === m[1])) return;
            runServer(m[1], m[2] as ServerAction);
        });
        return () => {
            if (typeof off === 'function') off();
        };
    }, [runServer]);

    // 내용 크기가 바뀔 때마다 셸에 알린다 (첫 알림에 창이 보인다)
    useLayoutEffect(() => {
        const el = rootRef.current;
        if (!el) return undefined;
        const report = () => {
            const box = el.getBoundingClientRect();
            window.api?.overlayFit?.(Math.ceil(box.width), Math.ceil(box.height));
        };
        report();
        if (typeof ResizeObserver === 'undefined') return undefined;
        const observer = new ResizeObserver(report);
        observer.observe(el);
        return () => observer.disconnect();
    }, []);

    // 누르기: 움직이지 않고 떼면 앱 열기, 누른 채 움직이면 창 끌기
    const press = useRef<{ x: number; y: number; dragging: boolean } | null>(null);
    const onPointerDown = (e: React.PointerEvent) => {
        if (e.button !== 0) return;
        press.current = { x: e.screenX, y: e.screenY, dragging: false };
    };
    const onPointerMove = (e: React.PointerEvent) => {
        const p = press.current;
        if (!p || p.dragging || !(e.buttons & 1)) return;
        if (Math.hypot(e.screenX - p.x, e.screenY - p.y) < DRAG_THRESHOLD) return;
        p.dragging = true;
        // 끌기는 OS가 맡는다: 끝난 뒤 떼기가 오지 않을 수 있어 여기서 누르기를 끝낸다
        press.current = null;
        window.api?.overlayStartDrag?.();
    };
    const onPointerUp = (e: React.PointerEvent) => {
        const p = press.current;
        press.current = null;
        if (!p || p.dragging || e.button !== 0) return;
        window.api?.overlayAction?.('open_app');
    };
    const onContextMenu = (e: React.MouseEvent) => {
        e.preventDefault();
        window.api?.overlayMenu?.(overlayMenuItems(serversRef.current, t));
    };

    return (
        <HostWindowContext.Provider value={hostWindow}>
            <div
                ref={rootRef}
                className="desktop-overlay"
                onPointerDown={onPointerDown}
                onPointerMove={onPointerMove}
                onPointerUp={onPointerUp}
                onPointerCancel={() => {
                    press.current = null;
                }}
                onContextMenu={onContextMenu}
            >
                <ExtensionSlot slotId="Desktop.overlay" />
            </div>
        </HostWindowContext.Provider>
    );
}
