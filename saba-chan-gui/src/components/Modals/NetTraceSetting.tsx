import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import type { ApiBody } from '../../types/daemon';
import { Icon } from '../Icon';
import { SabaToggle } from '../ui/SabaUI';
import { failed } from '../../utils/apiResult';

export const netTraceAvailable = () => typeof window.api?.systemNetTrace === 'function';

/** 데몬 `net_trace::Access` 중 문구가 있는 것 (unsupported면 설정을 숨긴다) */
const ACCESS = ['ok', 'needs_grant', 'needs_relogin'];

/**
 * 설정 > 게임 서버: 포트·서버별 네트워크 (기본 꺼짐). 켜면 시스템 모니터를 펼쳐 둔 동안 데몬이 Windows
 * 네트워크 이벤트로 서버·포트별 트래픽을 센다. 권한은 데몬이 관리자 확인 창 한 번으로 이 계정에 준다
 * (GET /api/system/net-trace, POST …/grant·…/revoke).
 */
export function NetTraceSetting() {
    const { t } = useTranslation('gui');
    const enabled = useSettingsStore((s) => s.systemMonitorNetTrace);
    const [access, setAccess] = useState<string | null>(null);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        let alive = true;
        window.api
            .systemNetTrace()
            .then((res: ApiBody) => {
                if (alive && !failed(res)) setAccess(String(res.access));
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    if (!access || !ACCESS.includes(access)) return null;

    const change = async (grant: boolean) => {
        setBusy(true);
        setError(null);
        try {
            const res = grant ? await window.api.systemNetTraceGrant() : await window.api.systemNetTraceRevoke();
            if (failed(res)) {
                setError(
                    res?.error_code === 'net_trace_elevation_failed'
                        ? t('net_trace.err_elevation')
                        : t('net_trace.err_generic'),
                );
                return;
            }
            setAccess(String(res.access));
        } catch {
            setError(t('net_trace.err_generic'));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="setting-item setting-item-group">
            <div className="setting-item-row">
                <label className="setting-label">
                    <span className="setting-title">
                        {t('net_trace.label')}
                    </span>
                    <span className="setting-description">{t('net_trace.description')}</span>
                </label>
                <SabaToggle
                    checked={enabled}
                    aria-label={t('net_trace.label')}
                    onChange={(checked) => useSettingsStore.getState().update({ systemMonitorNetTrace: checked })}
                />
            </div>
            {enabled && (
                <div className="setting-sublist">
                    <div className="setting-subitem">
                        <span>{t(`net_trace.status_${access}`)}</span>
                        {access === 'needs_grant' ? (
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={busy}
                                onClick={() => change(true)}
                            >
                                {t('net_trace.grant')}
                            </button>
                        ) : (
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={busy}
                                onClick={() => change(false)}
                            >
                                {t('net_trace.revoke')}
                            </button>
                        )}
                    </div>
                    {error && (
                        <div className="setting-validation-error" role="alert">
                            <Icon name="alertCircle" size="sm" /> {error}
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
