import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';

/** 결과가 실패인지 (GUI 브리지는 실패를 {error} 또는 {success:false}·{ok:false}로 돌려준다) */
export const failed = (res) => !res || !!res.error || res.success === false || res.ok === false;

/** 실패 결과 → 사람이 읽을 한 줄. 문지기·중계가 막은 것은 이유를 번역해 보인다 */
export function errorText(t, res) {
    const code = res?.error_code;
    if (typeof code === 'string' && code.startsWith('remote_')) {
        const fields = Array.isArray(res.fields) ? res.fields.join(', ') : '';
        return t(`errors.${code}`, { fields, defaultValue: t('errors.generic') });
    }
    return res?.error || res?.message || t('errors.generic');
}

export function Button({ tone = 'plain', busy = false, children, ...rest }) {
    return (
        <button type="button" className={`btn btn-${tone}`} disabled={busy || rest.disabled} {...rest}>
            {busy ? <span className="spinner" aria-hidden="true" /> : null}
            <span>{children}</span>
        </button>
    );
}

export function StatusDot({ status }) {
    const { t } = useTranslation();
    return (
        <span className={`status status-${status}`}>
            <span className="status-dot" aria-hidden="true" />
            {t(`status.${status}`)}
        </span>
    );
}

export function Empty({ children }) {
    return <p className="empty">{children}</p>;
}

export function Loading() {
    const { t } = useTranslation();
    return (
        <p className="empty">
            <span className="spinner" aria-hidden="true" /> {t('common_loading')}
        </p>
    );
}

export function Section({ title, action, children }) {
    return (
        <section className="section">
            {(title || action) && (
                <header className="section-head">
                    <h2>{title}</h2>
                    {action}
                </header>
            )}
            {children}
        </section>
    );
}

export function Segments({ value, options, onChange }) {
    return (
        <div className="segments" role="tablist">
            {options.map((o) => (
                <button
                    key={o.value}
                    type="button"
                    role="tab"
                    aria-selected={value === o.value}
                    className={value === o.value ? 'on' : ''}
                    onClick={() => onChange(o.value)}
                >
                    {o.label}
                </button>
            ))}
        </div>
    );
}

/** 열린 창을 Esc 로 닫는다 */
function useEscape(onClose) {
    useEffect(() => {
        if (!onClose) return;
        const onKey = (e) => e.key === 'Escape' && onClose();
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
    }, [onClose]);
}

export function ConfirmHost() {
    const { t } = useTranslation();
    const confirm = useApp((s) => s.confirm);
    useEscape(confirm ? () => confirm.done(false) : null);
    if (!confirm) return null;
    return (
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 눌러 닫는 보조 동작 (키보드는 Esc)
        <div className="scrim" role="presentation" onClick={(e) => e.target === e.currentTarget && confirm.done(false)}>
            <div className="dialog" role="dialog" aria-modal="true">
                <h3>{confirm.title}</h3>
                {confirm.message && <p className="dialog-text">{confirm.message}</p>}
                <div className="dialog-actions">
                    <Button onClick={() => confirm.done(false)}>{t('cancel')}</Button>
                    <Button tone={confirm.danger ? 'danger' : 'accent'} onClick={() => confirm.done(true)}>
                        {confirm.confirmLabel || t('confirm')}
                    </Button>
                </div>
            </div>
        </div>
    );
}

export function ToastHost() {
    const toasts = useApp((s) => s.toasts);
    return (
        <div className="toasts" aria-live="polite">
            {toasts.map((t) => (
                <div key={t.id} className={`toast toast-${t.tone}`}>
                    {t.message}
                </div>
            ))}
        </div>
    );
}

export function Sheet({ title, onClose, children }) {
    const { t } = useTranslation();
    useEscape(onClose);
    return (
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 눌러 닫는 보조 동작 (키보드는 Esc)
        <div className="scrim" role="presentation" onClick={(e) => e.target === e.currentTarget && onClose()}>
            <div className="sheet" role="dialog" aria-modal="true">
                <header className="sheet-head">
                    <h3>{title}</h3>
                    <button type="button" className="icon-btn" onClick={onClose} aria-label={t('close')}>
                        ✕
                    </button>
                </header>
                <div className="sheet-body">{children}</div>
            </div>
        </div>
    );
}
