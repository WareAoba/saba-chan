import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import './QuickPanel.css';
import { QuickPanelHead, ServerQuickList, usePanelFit, useQuickServers } from './ServerQuickList';

/** 판이 열려 있는 동안 데몬·서버 상태를 다시 읽는 간격 */
const REFRESH_MS = 3000;
/** 펫 크기 (%, 셸 overlay.rs `scale_of`와 같다) */
const SCALE_MIN = 50;
const SCALE_MAX = 200;
const SCALE_STEP = 10;

/** 주소(`?overlay-menu=1&scale=`)로 받은 지금 펫 크기 */
function initialScale() {
    const value = Number(new URLSearchParams(window.location.search).get('scale'));
    return Number.isFinite(value) && value >= SCALE_MIN && value <= SCALE_MAX ? value : 100;
}

/**
 * 펫을 우클릭하면 펫 옆에 뜨는 작은 판 (셸 overlay.rs `toggle_menu`, 트레이 팝업과 같은 판).
 * 서버 켜고 끄기(정지는 늘 한 번 더 눌러 확정한다: 작은 판에서 잘못 누르기 쉽다), 펫 크기 막대,
 * 사바쨩 열기, 바탕화면에서 숨기기. 셸 동작은 `overlay_action` 하나이고, 포커스를 잃거나 Esc면 닫힌다
 */
export function OverlayMenu() {
    const { t } = useTranslation('gui');
    const rootRef = useRef<HTMLDivElement>(null);
    const { daemonUp, servers, remote, refresh } = useQuickServers(REFRESH_MS);
    const [scale, setScale] = useState(initialScale);

    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') window.api?.overlayAction?.('close');
        };
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
    }, []);

    usePanelFit(rootRef, (height) => window.api?.overlayMenuFit?.(height));

    const resize = (value: number) => {
        setScale(value);
        window.api?.overlaySetScale?.(value);
    };
    const fill = ((scale - SCALE_MIN) / (SCALE_MAX - SCALE_MIN)) * 100;

    return (
        <div className="quick-panel overlay-menu" ref={rootRef}>
            <QuickPanelHead daemonUp={daemonUp} remote={remote} />
            {daemonUp && remote?.reachable !== false && (
                <ServerQuickList servers={servers} confirmStop refresh={refresh} />
            )}
            <div className="quick-panel-sep" />
            <label className="quick-panel-size">
                <span className="quick-panel-size-label">{t('desktop_overlay.menu_size')}</span>
                <input
                    className="quick-panel-slider"
                    type="range"
                    min={SCALE_MIN}
                    max={SCALE_MAX}
                    step={SCALE_STEP}
                    value={scale}
                    style={{ '--fill': `${fill}%` } as React.CSSProperties}
                    onChange={(e) => resize(Number(e.target.value))}
                />
                <span className="quick-panel-size-value">{scale}%</span>
            </label>
            <div className="quick-panel-sep" />
            <button type="button" className="quick-panel-row" onClick={() => window.api?.overlayAction?.('open_app')}>
                <Icon name="monitor" size={14} />
                {t('desktop_overlay.menu_open_app')}
            </button>
            <button type="button" className="quick-panel-row" onClick={() => window.api?.overlayAction?.('hide')}>
                <Icon name="eyeOff" size={14} />
                {t('desktop_overlay.menu_hide')}
            </button>
        </div>
    );
}
