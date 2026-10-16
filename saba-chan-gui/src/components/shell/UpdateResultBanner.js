import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

const nameOf = (item) => (typeof item === 'string' ? item : item?.component || item?.name || '');

/**
 * 재시작 뒤 첫 화면에 "업데이트 결과"를 한 번 보여준다.
 * 데몬 /api/updates/last-result 가 원천이고, 닫으면 clear 해서 다시 뜨지 않는다.
 * daemonReady: 데몬이 응답할 때가 되면 true (그 전에는 묻지 않는다).
 */
export function UpdateResultBanner({ daemonReady }) {
    const { t } = useTranslation('gui');
    const [result, setResult] = useState(null);

    useEffect(() => {
        if (!daemonReady || !window.api?.updaterLastResult) return;
        let alive = true;
        window.api
            .updaterLastResult({ clear: false })
            .then((r) => {
                if (alive && r && r.timestamp && r.ok !== false) setResult(r);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, [daemonReady]);

    if (!result) return null;

    const failed = (result.failed || []).map(nameOf).filter(Boolean);
    const updated = (result.updated_components || []).map(nameOf).filter(Boolean);
    const ok = result.success && failed.length === 0;

    const dismiss = () => {
        setResult(null);
        window.api?.updaterLastResult?.({ clear: true }).catch(() => {});
    };

    return (
        <div className={`update-banner notice ${ok ? 'tone-ok' : 'tone-stop'}`} role="status">
            <Icon name={ok ? 'checkCircle' : 'alertTriangle'} size="sm" />
            <div className="update-banner-text">
                <strong>{ok ? t('shell.update_done') : t('shell.update_failed')}</strong>
                {updated.length > 0 && <span>{t('shell.update_components', { list: updated.join(', ') })}</span>}
                {failed.length > 0 && <span>{t('shell.update_failed_components', { list: failed.join(', ') })}</span>}
                {!ok && result.message && <span className="update-banner-msg">{result.message}</span>}
            </div>
            <button type="button" className="icon-btn" onClick={dismiss} aria-label={t('shell.dismiss')}>
                <Icon name="close" size="sm" />
            </button>
        </div>
    );
}
