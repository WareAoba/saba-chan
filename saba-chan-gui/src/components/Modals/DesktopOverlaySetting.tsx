import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaToggle } from '../ui/SabaUI';

interface OverlayState {
    enabled: boolean;
    available: boolean;
    supported: boolean;
}

/**
 * 설정 > 일반: 바탕화면 오버레이(익스텐션 slot `Desktop.overlay`) 켜기. 그 slot을 쓰는 익스텐션이 켜져 있을 때만 보인다.
 * 셸이 설정(`desktopOverlay.enabled`)에 저장하고 창을 띄우거나 내린다 (overlay.rs). 오버레이 우클릭 메뉴의 숨기기도 이 스위치를 끈다
 */
export function DesktopOverlaySetting() {
    const { t } = useTranslation('gui');
    const [state, setState] = useState<OverlayState | null>(null);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let alive = true;
        Promise.resolve(window.api?.overlayState?.())
            .then((res) => {
                if (alive && res && typeof res.enabled === 'boolean') setState(res as OverlayState);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    if (!state?.available) return null;

    const change = async (enabled: boolean) => {
        setBusy(true);
        setState((s) => (s ? { ...s, enabled } : s));
        try {
            const res = await window.api.overlaySetEnabled(enabled);
            if (res?.ok === false) setState((s) => (s ? { ...s, enabled: !enabled } : s));
        } catch {
            setState((s) => (s ? { ...s, enabled: !enabled } : s));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="setting-item">
            <label className="setting-label">
                <span className="setting-title">
                    <Icon name="monitor" size="sm" /> {t('desktop_overlay.setting_label')}
                </span>
                <span className="setting-description">
                    {state.supported
                        ? t('desktop_overlay.setting_description')
                        : t('desktop_overlay.setting_unsupported')}
                </span>
            </label>
            <SabaToggle
                checked={state.enabled && state.supported}
                disabled={busy || !state.supported}
                onChange={change}
            />
        </div>
    );
}
