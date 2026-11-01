import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { SabaCheckbox } from './ui/SabaUI';

const NUMBER_TYPES = new Set(['number', 'integer', 'int', 'float']);
const BOOL_TYPES = new Set(['boolean', 'bool']);

const coerce = (def, raw) => {
    if (BOOL_TYPES.has(def.type)) return !!raw;
    if (NUMBER_TYPES.has(def.type)) {
        const n = Number(raw);
        return raw === '' || Number.isNaN(n) ? null : n;
    }
    return raw;
};

/**
 * 익스텐션 전역 설정 (manifest `config_fields`, GET/PUT /api/extensions/:id/config, Q-2).
 * 값은 데몬이 config.json에 저장한다. 이름은 익스텐션 locales의 config.<키>가 있으면 그것을 쓴다.
 */
export function ExtensionConfigForm({ ext }) {
    const { t } = useTranslation('gui');
    const fields = Object.entries(ext.config_fields || {});
    const [open, setOpen] = useState(false);
    const [values, setValues] = useState(null);
    const [busy, setBusy] = useState(false);
    const [message, setMessage] = useState(null); // {tone, text}

    if (
        !fields.length ||
        typeof window.api?.extensionConfigGet !== 'function' ||
        typeof window.api?.extensionConfigSet !== 'function'
    )
        return null;

    const toggle = async () => {
        if (open) return setOpen(false);
        setOpen(true);
        setBusy(true);
        setMessage(null);
        try {
            const res = await window.api.extensionConfigGet(ext.id);
            if (res?.error) throw new Error(res.error);
            const saved = res?.config || {};
            setValues(
                Object.fromEntries(
                    fields.map(([k, def]) => [k, saved[k] ?? def.default ?? (BOOL_TYPES.has(def.type) ? false : '')]),
                ),
            );
        } catch (err) {
            setMessage({ tone: 'stop', text: t('extension_config.load_failed', { error: err.message }) });
        } finally {
            setBusy(false);
        }
    };

    const save = async () => {
        setBusy(true);
        setMessage(null);
        try {
            const config = Object.fromEntries(fields.map(([k, def]) => [k, coerce(def, values[k])]));
            const res = await window.api.extensionConfigSet(ext.id, config);
            if (res?.error) throw new Error(res.error);
            setMessage({ tone: 'ok', text: t('extension_config.saved') });
        } catch (err) {
            setMessage({ tone: 'stop', text: t('extension_config.save_failed', { error: err.message }) });
        } finally {
            setBusy(false);
        }
    };

    const label = (key) => t(`ext_${ext.id}:config.${key}`, { defaultValue: key });

    return (
        <div className="extension-config">
            <button type="button" className="btn btn-ghost btn-sm" onClick={toggle} aria-expanded={open}>
                <Icon name={open ? 'chevronUp' : 'settings'} size="sm" /> {t('extension_config.toggle')}
            </button>
            {open && values && (
                <div className="extension-config-form">
                    {fields.map(([key, def]) => {
                        const id = `ext-cfg-${ext.id}-${key}`;
                        return (
                            <div key={key} className="extension-config-field">
                                <label htmlFor={id}>{label(key)}</label>
                                {BOOL_TYPES.has(def.type) ? (
                                    <SabaCheckbox
                                        id={id}
                                        checked={!!values[key]}
                                        onChange={(on) => setValues((v) => ({ ...v, [key]: on }))}
                                    />
                                ) : (
                                    <input
                                        id={id}
                                        className="setting-input-text"
                                        type={NUMBER_TYPES.has(def.type) ? 'number' : 'text'}
                                        value={values[key] ?? ''}
                                        onChange={(e) => setValues((v) => ({ ...v, [key]: e.target.value }))}
                                    />
                                )}
                            </div>
                        );
                    })}
                    <div className="extension-config-actions">
                        <button type="button" className="btn btn-primary btn-sm" onClick={save} disabled={busy}>
                            <Icon name="save" size="sm" /> {t('extension_config.save')}
                        </button>
                    </div>
                </div>
            )}
            {message && (
                <p
                    className={`extension-config-message tone-${message.tone}`}
                    role={message.tone === 'stop' ? 'alert' : 'status'}
                >
                    {message.text}
                </p>
            )}
        </div>
    );
}
