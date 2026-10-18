import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useExtensions } from '../../contexts/ExtensionContext';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';

export const extensionGuiAvailable = () =>
    Boolean(window.api?.sandboxListExtensionGui && window.api?.sandboxRevokeExtensionGui);

const failed = (res) => !res || res.success === false || Boolean(res.error || res.error_code);

/**
 * 설정 > 고급: 확인되지 않은 익스텐션의 화면(GUI 번들)을 이 PC에서 허락한 목록 (데몬 config.json sandbox.extension_gui).
 * current=false면 허락한 뒤 파일이 바뀌어 그 허락은 쓰이지 않는다. 묻고 거절한 익스텐션은 여기서 허락한다.
 */
export function ExtensionGuiSection() {
    const { t } = useTranslation('gui');
    const { guiPending, approveExtensionGui } = useExtensions();
    const [items, setItems] = useState(null); // null = 불러오는 중
    const [error, setError] = useState(null);
    const [busy, setBusy] = useState(null); // 바꾸는 중인 항목

    const load = useCallback(async () => {
        try {
            const res = await window.api.sandboxListExtensionGui();
            if (failed(res)) setError(res?.error || t('extension_gui_consent.failed'));
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
            const res = await window.api.sandboxRevokeExtensionGui(component);
            if (failed(res)) safeShowToast(res?.error || t('extension_gui_consent.failed'), 'error', 5000);
            else safeShowToast(t('extension_gui_consent.revoked', { component }), 'info', 4000);
        } catch (e) {
            safeShowToast(e.message, 'error', 5000);
        }
        setBusy(null);
        load();
    };

    const allow = async (id) => {
        setBusy(id);
        await approveExtensionGui(id);
        setBusy(null);
        load();
    };

    return (
        <div className="data-transfer">
            <h4 className="data-transfer-title">
                <Icon name="key" size="sm" /> {t('extension_gui_consent.title')}
            </h4>
            <p className="setting-description">{t('extension_gui_consent.description')}</p>
            {error && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{error}</span>
                </div>
            )}
            {guiPending.map((ext) => (
                <div key={ext.id} className="setting-item">
                    <span className="setting-label">
                        <span className="setting-title">{ext.id}</span>
                        <span className="setting-description">{t('extension_gui_consent.pending')}</span>
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={busy !== null}
                        onClick={() => allow(ext.id)}
                        aria-label={t('extension_gui_consent.allow_named', { id: ext.id })}
                    >
                        {busy === ext.id ? <SabaSpinner size="sm" /> : t('extension_gui_consent.allow')}
                    </button>
                </div>
            ))}
            {!error && items === null && <SabaSpinner size="sm" />}
            {items?.length === 0 && guiPending.length === 0 && (
                <p className="setting-description">{t('extension_gui_consent.empty')}</p>
            )}
            {items?.map((it) => (
                <div key={it.component} className="setting-item">
                    <span className="setting-label">
                        <span className="setting-title">{it.component}</span>
                        <span className="setting-description">
                            {it.current === false
                                ? t('extension_gui_consent.stale')
                                : t('extension_gui_consent.active')}
                        </span>
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={busy !== null}
                        onClick={() => revoke(it.component)}
                        aria-label={t('extension_gui_consent.revoke_named', { component: it.component })}
                    >
                        {busy === it.component ? <SabaSpinner size="sm" /> : t('extension_gui_consent.revoke')}
                    </button>
                </div>
            ))}
        </div>
    );
}
