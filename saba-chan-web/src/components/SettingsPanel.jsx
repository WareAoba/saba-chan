import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Section } from './ui';

/** 데몬 문지기(src/remote/gate.rs SETTINGS_PC_ONLY)가 원격으로 받지 않는 칸. 모듈의 비밀번호·file·path 칸도 같다 (비밀번호는 읽어 오지도 않는다) */
const PC_ONLY = new Set([
    'executable_path',
    'working_dir',
    'extra_args',
    'process_name',
    'auto_detect',
    'rest_host',
    'rest_port',
    'rest_password',
    'extension_data',
]);

/** 데몬이 인스턴스 최상위에 두는 공통 칸 (GUI useServerSettings 와 같다): 최상위가 진짜다 */
const TOP_LEVEL = new Set([
    'port',
    'rest_host',
    'rest_port',
    'rest_username',
    'rest_password',
    'executable_path',
    'working_dir',
    'protocol_mode',
    'server_version',
    'extra_args',
]);

const typeOf = (f) => f.field_type || f.type || 'text';
const pcOnly = (f) => PC_ONLY.has(f.name) || ['file', 'path', 'password'].includes(typeOf(f)) || f.format === 'path';

export function savedValue(server, field) {
    const top = server[field.name];
    const cached = server.module_settings?.[field.name];
    const v = TOP_LEVEL.has(field.name) ? (top ?? cached) : (cached ?? top);
    return v ?? field.default ?? '';
}

/** 스키마 2 visible_when: {field, equals} | {field, in:[...]} (값 비교는 글자로) */
export function visible(field, values, fields) {
    const rule = field.visible_when;
    if (!rule?.field) return true;
    const other = fields.find((f) => f.name === rule.field);
    const v = String(values[rule.field] ?? other?.default ?? '');
    if (Array.isArray(rule.in)) return rule.in.map(String).includes(v);
    return String(rule.equals) === v;
}

/** 보낼 값: 고친 칸만, 칸 종류에 맞게 */
export function patchOf(fields, values, touched) {
    const patch = {};
    for (const f of fields) {
        if (!touched.has(f.name) || pcOnly(f)) continue;
        const v = values[f.name];
        const type = typeOf(f);
        if (type === 'boolean') patch[f.name] = v === true || v === 'true';
        else if (type === 'number') patch[f.name] = v === '' ? null : Number(v);
        else if (type === 'list')
            patch[f.name] = String(v ?? '')
                .split('\n')
                .map((s) => s.trim())
                .filter(Boolean);
        else patch[f.name] = v ?? '';
    }
    return patch;
}

export function FieldInput({ field, value, onChange, t, modNs }) {
    const type = typeOf(field);
    const label = t(`${modNs}:settings.${field.name}.label`, { defaultValue: field.label || field.name });
    const desc = t(`${modNs}:settings.${field.name}.description`, { defaultValue: field.description || '' });
    if (pcOnly(field)) {
        return (
            <div className="field field-locked">
                <span>{label}</span>
                <p className="locked">
                    <Icon name="lock" size={14} /> {t('settings.pc_only')}
                </p>
            </div>
        );
    }
    let input;
    if (type === 'boolean') {
        input = (
            <input
                type="checkbox"
                className="switch"
                checked={value === true || value === 'true'}
                onChange={(e) => onChange(e.target.checked)}
            />
        );
    } else if (type === 'select' && Array.isArray(field.options)) {
        input = (
            <Dropdown
                value={String(value)}
                onChange={onChange}
                options={field.options.map((o) => ({
                    value: o,
                    label: t(`${modNs}:settings.${field.name}.options.${o}`, { defaultValue: o }),
                }))}
            />
        );
    } else if (type === 'multiline' || type === 'list') {
        const text = Array.isArray(value) ? value.join('\n') : String(value ?? '');
        input = <textarea rows={4} value={text} onChange={(e) => onChange(e.target.value)} />;
    } else {
        input = (
            <input
                type={type === 'number' ? 'number' : type === 'password' ? 'password' : 'text'}
                inputMode={type === 'number' ? 'numeric' : undefined}
                value={String(value ?? '')}
                onChange={(e) => onChange(e.target.value)}
                autoComplete="off"
            />
        );
    }
    return (
        // biome-ignore lint/a11y/noLabelWithoutControl: 입력칸은 위에서 고른 {input} 이다
        <label className={`field${type === 'boolean' ? ' field-row' : ''}`}>
            <span>
                {label}
                {field.restart_required ? <em className="tag">{t('settings.restart_required')}</em> : null}
            </span>
            {input}
            {desc && <small className="hint">{desc}</small>}
        </label>
    );
}

export function SettingsPanel({ api, server, moduleData, running }) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const ask = useApp((s) => s.ask);
    const fields = useMemo(() => moduleData?.settings?.fields ?? [], [moduleData]);
    const initial = useMemo(
        () => Object.fromEntries(fields.map((f) => [f.name, savedValue(server, f)])),
        [fields, server],
    );
    const [values, setValues] = useState(initial);
    const [touched, setTouched] = useState(() => new Set());
    const [saving, setSaving] = useState(false);
    const modNs = `mod_${server.module}`;
    // 저장에 실을 revision: 고치기 시작한 때(또는 마지막 저장)의 값. 폴링의 최신 값을 실으면 데몬의 settings_changed 검사를 지난다
    const revision = useRef(server.revision || null);
    const seen = useRef(server.revision);

    // 다른 곳에서 바뀐 뒤 다시 불러오기: 마지막 폴링이 아니라 데몬의 지금 값과 revision을 읽는다
    // (폴링 값은 그보다 오래됐을 수 있다). 읽지 못하면 고친 값을 그대로 둔다
    const reload = async (conflictRevision) => {
        const fresh = await api.instanceGet(server.id);
        if (!fresh?.id || failed(fresh)) return toast(errorText(t, fresh), 'error');
        setValues(Object.fromEntries(fields.map((f) => [f.name, savedValue(fresh, f)])));
        setTouched(new Set());
        revision.current = fresh.revision || conflictRevision || null;
    };

    // 고치는 칸이 없는 동안에는 다른 곳에서 바뀐 값을 따라간다
    useEffect(() => {
        if (seen.current === server.revision) return;
        seen.current = server.revision;
        if (touched.size > 0) return;
        setValues(initial);
        revision.current = server.revision || null;
    }, [server.revision, initial, touched]);

    const change = (key, v) => {
        setValues((prev) => ({ ...prev, [key]: v }));
        setTouched((prev) => new Set(prev).add(key));
    };

    const save = async () => {
        const patch = patchOf(fields, values, touched);
        if (Object.keys(patch).length === 0) return;
        if (revision.current) patch.revision = revision.current;
        setSaving(true);
        const res = await api.instanceUpdateSettings(server.id, patch);
        setSaving(false);
        if (res?.error_code === 'settings_changed') {
            // GUI처럼 창으로 알린다. 고르면 바뀐 값을 다시 불러오고, 고친 값은 버린다
            const ok = await ask({
                title: t('settings.save_failed'),
                message: t('settings.changed_elsewhere'),
                confirmLabel: t('settings.reload'),
            });
            if (ok) await reload(res.revision);
            return;
        }
        if (failed(res)) return toast(errorText(t, res), 'error');
        if (res?.revision) revision.current = res.revision;
        setTouched(new Set());
        toast(running ? t('settings.saved_restart') : t('settings.saved'), 'ok');
    };

    const shown = fields.filter((f) => visible(f, values, fields));
    return (
        <div className="panel">
            <Section title={t('settings.game')}>
                {shown.length === 0 ? (
                    <Empty>{t('settings.none')}</Empty>
                ) : (
                    <form
                        className="form"
                        onSubmit={(e) => {
                            e.preventDefault();
                            save();
                        }}
                    >
                        {shown.map((f) => (
                            <FieldInput
                                key={f.name}
                                field={f}
                                value={values[f.name]}
                                onChange={(v) => change(f.name, v)}
                                t={t}
                                modNs={modNs}
                            />
                        ))}
                        <Button tone="accent" type="submit" busy={saving} disabled={touched.size === 0}>
                            {t('settings.save')}
                        </Button>
                    </form>
                )}
            </Section>
        </div>
    );
}
