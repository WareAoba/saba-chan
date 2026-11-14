import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { type HostWindow, HostWindowContext, isAppRoute } from '../../extensions/host';
import ExtensionSlot from '../ExtensionSlot';
import './DesktopOverlay.css';
import { useQuickServers } from './ServerQuickList';

/** 서버 목록을 다시 읽는 간격 (이벤트로도 다시 읽는다) */
const REFRESH_MS = 10_000;
/** 누른 채 이만큼 움직이면 클릭이 아니라 창 끌기 (px) */
const DRAG_THRESHOLD = 4;
/** 주소(`?desktop-overlay=1&scale=`)로 받은 처음 펫 크기 (%). 바뀌면 셸이 `overlay:scale`로 알린다 */
function initialScale() {
    const value = Number(new URLSearchParams(window.location.search).get('scale'));
    return Number.isFinite(value) && value > 0 ? value : 100;
}

/**
 * 바탕화면 오버레이 창 (`?desktop-overlay=1`, 셸 overlay.rs). 익스텐션 slot `Desktop.overlay`를 그리고,
 * 창 일은 여기서 한다: 누르면 앱 열기, 누른 채 움직이면 창 끌기, 우클릭은 셸이 펫 옆에 판을 연다(`OverlayMenu`).
 * 펫 크기(판의 크기 막대)는 slot 전체를 CSS zoom으로 키운다.
 * 내용 크기를 재서 셸에 알리면 셸이 아래 가운데 점을 지키며 크기를 맞추고 보인다.
 * 서버를 켜고 끄는 일은 데몬 API로 한다. 익스텐션(host.run)은 트레이 팝업처럼 이 PC 데몬의 서버만 정지·재시작할 수 있다
 */
export function DesktopOverlay() {
    const rootRef = useRef<HTMLDivElement>(null);
    const { servers, run } = useQuickServers(REFRESH_MS);
    const [scale, setScale] = useState(initialScale);
    const serversRef = useRef(servers);
    serversRef.current = servers;

    const hostWindow = useMemo<HostWindow>(
        () => ({
            servers: () => serversRef.current,
            run,
            // 고른 화면(서버·탭)은 셸이 메인 창에 바로 연다. 모르는 화면은 앱만 연다
            openApp: (route) =>
                isAppRoute(route)
                    ? window.api?.overlayAction?.('open_app', route)
                    : window.api?.overlayAction?.('open_app'),
        }),
        [run],
    );

    // 판의 크기 막대
    useEffect(() => {
        if (typeof window.api?.onOverlayScale !== 'function') return undefined;
        const off = window.api.onOverlayScale((value: unknown) => {
            if (typeof value === 'number' && value > 0) setScale(value);
        });
        return () => {
            if (typeof off === 'function') off();
        };
    }, []);

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
        window.api?.overlayMenu?.();
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
                <div className="desktop-overlay-zoom" style={{ zoom: scale / 100 }}>
                    <ExtensionSlot slotId="Desktop.overlay" />
                </div>
            </div>
        </HostWindowContext.Provider>
    );
}
