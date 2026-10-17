import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';

export const unisolatedAvailable = () =>
    Boolean(window.api?.sandboxListUnisolated && window.api?.sandboxRevokeUnisolated);

const failed = (res) => !res || res.success === false || Boolean(res.error || res.error_code);

/**
 * 설정 > 고급: 격리 없이 돌리기로 동의한 구성요소 (데몬 config.json sandbox.unisolated).
 * current=false면 파일이 바뀌어 그 동의는 쓰이지 않는다(다음 실행 때 다시 묻는다).
 */
export function UnisolatedSection() {
    const { t } = useTranslation('gui');
    const [items, setItems] = useState(null); // null = 불러오는 중
    const [error, setError] = useState(null);
    const [busy, setBusy] = useState(null); // 지우는 중인 component

    const load = useCallback(async () => {
        try {
            const res = await window.api.sandboxListUnisolated();
            if (failed(res)) setError(res?.error || t('unisolated.failed'));
            else {
                setItems(Array.isArray(res.components) ? res.components : []);
                setError(null);
            }
        } catch (e) {
            setError(e.message);
        }
    }, [t]);

    useEffect(() => {
        load();
    }, [load]);

    const revoke = async (component) => {
        setBusy(component);
        try {
            const res = await window.api.sandboxRevokeUnisolated(component);
            if (failed(res)) safeShowToast(res?.error || t('unisolated.failed'), 'error', 5000);
            else safeShowToast(t('unisolated.revoked', { component }), 'info', 4000);
        } catch (e) {
            safeShowToast(e.message, 'error', 5000);
        }
        setBusy(null);
        load();
    };

    return (
        <div className="data-transfer">
            <h4 className="data-transfer-title">
                <Icon name="key" size="sm" /> {t('unisolated.title')}
            </h4>
            <p className="setting-description">{t('unisolated.description')}</p>
            {error && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{error}</span>
                </div>
            )}
            {!error && items === null && <SabaSpinner size="sm" />}
            {items?.length === 0 && <p className="setting-description">{t('unisolated.empty')}</p>}
            {items?.map((it) => (
                <div key={it.component} className="setting-item">
                    <span className="setting-label">
                        <span className="setting-title">{it.component}</span>
                        <span className="setting-description">
                            {it.current === false ? t('unisolated.stale') : t('unisolated.active')}
                        </span>
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={busy !== null}
                        onClick={() => revoke(it.component)}
                        aria-label={t('unisolated.revoke_named', { component: it.component })}
                    >
                        {busy === it.component ? <SabaSpinner size="sm" /> : t('unisolated.revoke')}
                    </button>
                </div>
            ))}
        </div>
    );
}
