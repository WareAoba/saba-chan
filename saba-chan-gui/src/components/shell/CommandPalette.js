import clsx from 'clsx';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import { useUIStore } from '../../stores/useUIStore';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';

/**
 * 명령 팔레트 (Ctrl+K): 데몬 액션 카탈로그(GET /api/actions?surface=gui)만 검색한다. 자연어는 받지 않는다.
 * 고르면 plan으로 요약을 받고, 확인이 필요한 액션은 한 번 더 누른 뒤 run한다.
 */
export const paletteAvailable = () =>
    Boolean(window.api?.actionsList && window.api?.actionsPlan && window.api?.actionsRun);

const failed = (res) => !res || res.success === false || Boolean(res.error || res.error_code);

const listOf = (res) => (Array.isArray(res) ? res : Array.isArray(res?.actions) ? res.actions : []);

export const actionTitle = (a) => a.title || a.titleKey || a.id;

/** 계획 요약 값. 설정 변경은 `before`가 붙어 "이전 → 새 값"으로 보인다 (null이면 없음) */
export const summaryValue = (s, t) =>
    'before' in s
        ? `${s.before == null ? t('palette.none') : String(s.before)} → ${String(s.value ?? '')}`
        : String(s.value ?? '');

/** 검색: 제목·id에 검색어 조각이 모두 들어 있는 것 (순서 무관) */
export function filterActions(actions, query) {
    const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!words.length) return actions;
    return actions.filter((a) => {
        const hay = `${actionTitle(a)} ${a.id} ${a.description || ''}`.toLowerCase();
        return words.every((w) => hay.includes(w));
    });
}

/** 입력 스키마의 필드 (JSON Schema object의 properties) */
export function inputFields(schema) {
    const props = schema?.properties || {};
    const required = Array.isArray(schema?.required) ? schema.required : [];
    return Object.entries(props).map(([name, p]) => ({
        name,
        title: p.title || name,
        type: p.type,
        format: p.format,
        enum: Array.isArray(p.enum) ? p.enum : null,
        default: p.default,
        required: required.includes(name),
    }));
}

function initialInput(fields) {
    const out = {};
    for (const f of fields) {
        if (f.default !== undefined) out[f.name] = f.default;
        else if (f.type === 'boolean') out[f.name] = false;
    }
    return out;
}

/** 빈 칸은 보내지 않고, 숫자 칸은 숫자로 */
function cleanInput(fields, values) {
    const out = {};
    for (const f of fields) {
        const v = values[f.name];
        if (v === undefined || v === '') continue;
        out[f.name] = f.type === 'number' || f.type === 'integer' ? Number(v) : v;
    }
    return out;
}

function errorText(res, t) {
    if (res?.error_code === 'invalid_input') {
        return t('palette.invalid_input', { field: res.field || '?', reason: res.reason || '' });
    }
    if (res?.error_code === 'plan_expired') return t('palette.plan_expired');
    return res?.error || res?.message || t('palette.failed');
}

function Field({ field, value, onChange }) {
    const id = `palette-field-${field.name}`;
    let input;
    if (field.enum) {
        input = (
            <CustomDropdown
                id={id}
                value={value ?? ''}
                onChange={(v) => onChange(v)}
                placeholder=" "
                options={[
                    ...(field.required ? [] : [{ value: '', label: ' ' }]),
                    ...field.enum.map((v) => ({ value: v, label: String(v) })),
                ]}
            />
        );
    } else if (field.type === 'boolean') {
        input = <input id={id} type="checkbox" checked={!!value} onChange={(e) => onChange(e.target.checked)} />;
    } else {
        input = (
            <input
                id={id}
                className="input"
                type={
                    field.format === 'password'
                        ? 'password'
                        : field.type === 'number' || field.type === 'integer'
                          ? 'number'
                          : 'text'
                }
                value={value ?? ''}
                onChange={(e) => onChange(e.target.value)}
            />
        );
    }
    return (
        <label className="palette-field" htmlFor={id}>
            <span>
                {field.title}
                {field.required && <span className="palette-required"> *</span>}
            </span>
            {input}
        </label>
    );
}

/** 결과 한 덩어리: message가 있으면 그것, 아니면 짧게 JSON */
function resultText(res) {
    const r = res?.result ?? res;
    if (typeof r === 'string') return r;
    if (typeof r?.message === 'string') return r.message;
    try {
        return JSON.stringify(r, null, 2);
    } catch {
        return String(r);
    }
}

function Palette({ onClose, busyRef }) {
    const { t, i18n } = useTranslation('gui');
    const activeServerId = useUIStore((s) => s.activeServerId);
    const servers = useServerStore((s) => s.servers);
    const server = servers.find((s) => s.id === activeServerId) || null;

    const [actions, setActions] = useState(null); // null = 불러오는 중
    const [loadError, setLoadError] = useState(null);
    const [query, setQuery] = useState('');
    const [cursor, setCursor] = useState(0);
    // step: list → input → confirm → result
    const [step, setStep] = useState('list');
    const [chosen, setChosen] = useState(null);
    const [values, setValues] = useState({});
    const [plan, setPlan] = useState(null);
    const [busy, setBusy] = useState(false);
    // 실행 중에 닫거나 돌아가면 결과(성공 알림·실패 문구)를 볼 곳이 없다
    busyRef.current = busy;
    const close = () => {
        if (!busy) onClose();
    };
    const [error, setError] = useState(null);
    const [result, setResult] = useState(null);
    const inputRef = useRef(null);

    useEffect(() => {
        let alive = true;
        (async () => {
            try {
                const res = await window.api.actionsList({
                    surface: 'gui',
                    instance: server?.id,
                    locale: i18n.language,
                });
                if (!alive) return;
                if (!Array.isArray(res) && failed(res)) setLoadError(errorText(res, t));
                else setActions(listOf(res));
            } catch (e) {
                if (alive) setLoadError(e.message);
            }
        })();
        return () => {
            alive = false;
        };
    }, [server?.id, t, i18n.language]);

    const shown = useMemo(() => filterActions(actions || [], query), [actions, query]);
    const fields = useMemo(() => inputFields(chosen?.input), [chosen]);
    const instanceOf = (a) => (a?.scope === 'instance' ? server?.id : undefined);

    const finish = (action, res) => {
        if (action.kind === 'query') {
            setResult(resultText(res));
            setStep('result');
            return;
        }
        safeShowToast(t('palette.done', { action: actionTitle(action) }), 'success', 3000);
        onClose();
    };

    const run = async (action, body) => {
        setBusy(true);
        setError(null);
        try {
            const res = await window.api.actionsRun(action.id, body);
            if (failed(res)) setError(errorText(res, t));
            else finish(action, res);
        } catch (e) {
            setError(e.message);
        }
        setBusy(false);
    };

    /** plan을 받고, 확인이 필요 없으면 바로 실행 */
    const planAndRun = async (action, input) => {
        setBusy(true);
        setError(null);
        try {
            const res = await window.api.actionsPlan(action.id, { input, instance: instanceOf(action) });
            setBusy(false);
            if (failed(res)) {
                setError(errorText(res, t));
            } else if (res.needsConfirm) {
                setPlan(res);
                setStep('confirm');
            } else {
                await run(action, { planId: res.planId });
            }
        } catch (e) {
            setError(e.message);
            setBusy(false);
        }
    };

    const choose = (action) => {
        if (!action || busy) return;
        if (action.scope === 'instance' && !server) {
            setError(t('palette.need_server'));
            return;
        }
        setChosen(action);
        setError(null);
        const f = inputFields(action.input);
        if (f.length) {
            setValues(initialInput(f));
            setStep('input');
        } else {
            planAndRun(action, {});
        }
    };

    const back = () => {
        if (busy) return;
        setStep('list');
        setChosen(null);
        setPlan(null);
        setError(null);
        setResult(null);
        setTimeout(() => inputRef.current?.focus(), 0);
    };

    const onKeyDown = (e) => {
        if (e.key === 'Escape') {
            e.preventDefault();
            if (step === 'list') close();
            else back();
            return;
        }
        if (step !== 'list') return;
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            setCursor((c) => Math.min(c + 1, shown.length - 1));
        } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setCursor((c) => Math.max(c - 1, 0));
        } else if (e.key === 'Enter') {
            e.preventDefault();
            choose(shown[cursor]);
        }
    };

    return (
        // biome-ignore lint/a11y/noStaticElementInteractions: 바깥을 누르면 닫는다
        <div className="palette-overlay" onMouseDown={(e) => e.target === e.currentTarget && close()}>
            <div
                className="palette"
                role="dialog"
                aria-modal="true"
                aria-label={t('palette.title')}
                onKeyDown={onKeyDown}
            >
                {step === 'list' && (
                    <>
                        <div className="palette-search">
                            <Icon name="command" size="sm" />
                            <input
                                ref={inputRef}
                                // biome-ignore lint/a11y/noAutofocus: 팔레트를 열면 바로 친다
                                autoFocus
                                value={query}
                                onChange={(e) => {
                                    setQuery(e.target.value);
                                    setCursor(0);
                                }}
                                placeholder={t('palette.placeholder')}
                                aria-label={t('palette.placeholder')}
                                role="combobox"
                                aria-expanded="true"
                                aria-controls="palette-list"
                                aria-activedescendant={shown[cursor] ? `palette-opt-${cursor}` : undefined}
                            />
                            {server && <span className="palette-scope">{server.name}</span>}
                        </div>
                        {loadError && <p className="palette-error">{loadError}</p>}
                        {!loadError && actions === null && (
                            <p className="palette-empty">
                                <SabaSpinner size="sm" /> {t('palette.loading')}
                            </p>
                        )}
                        {actions && shown.length === 0 && <p className="palette-empty">{t('palette.empty')}</p>}
                        <div className="palette-list" id="palette-list" role="listbox" aria-label={t('palette.title')}>
                            {shown.map((a, i) => (
                                <div
                                    key={a.id}
                                    id={`palette-opt-${i}`}
                                    role="option"
                                    aria-selected={i === cursor}
                                    className={clsx('palette-item', { active: i === cursor })}
                                    onMouseEnter={() => setCursor(i)}
                                    onClick={() => choose(a)}
                                    tabIndex={-1}
                                    onKeyDown={() => {}}
                                >
                                    <span className="palette-item-title">{actionTitle(a)}</span>
                                    {a.scope === 'instance' && server && (
                                        <span className="palette-item-sub">{server.name}</span>
                                    )}
                                    {(a.risk === 'danger' || a.kind === 'admin') && (
                                        <span className="palette-item-warn">
                                            <Icon name="alertTriangle" size="xs" />
                                        </span>
                                    )}
                                </div>
                            ))}
                        </div>
                        {error && <p className="palette-error">{error}</p>}
                    </>
                )}

                {step === 'input' && chosen && (
                    <form
                        className="palette-step"
                        onSubmit={(e) => {
                            e.preventDefault();
                            planAndRun(chosen, cleanInput(fields, values));
                        }}
                    >
                        <h3>{actionTitle(chosen)}</h3>
                        {fields.map((f) => (
                            <Field
                                key={f.name}
                                field={f}
                                value={values[f.name]}
                                onChange={(v) => setValues((prev) => ({ ...prev, [f.name]: v }))}
                            />
                        ))}
                        {error && <p className="palette-error">{error}</p>}
                        <div className="palette-actions">
                            <button type="button" className="btn btn-ghost" onClick={back} disabled={busy}>
                                {t('palette.back')}
                            </button>
                            <button
                                type="submit"
                                className="btn btn-primary"
                                disabled={busy || fields.some((f) => f.required && (values[f.name] ?? '') === '')}
                            >
                                {busy ? <SabaSpinner size="sm" light /> : t('palette.next')}
                            </button>
                        </div>
                    </form>
                )}

                {step === 'confirm' && chosen && plan && (
                    <div className="palette-step">
                        <h3>{t('palette.confirm_title', { action: actionTitle(chosen) })}</h3>
                        <dl className="palette-summary">
                            {(plan.summary || []).map((s, i) => (
                                <div key={`${s.labelKey || s.label}-${i}`}>
                                    <dt>{s.label || s.labelKey}</dt>
                                    <dd>{summaryValue(s, t)}</dd>
                                </div>
                            ))}
                        </dl>
                        {error && <p className="palette-error">{error}</p>}
                        <div className="palette-actions">
                            <button type="button" className="btn btn-ghost" onClick={back} disabled={busy}>
                                {t('modals.cancel')}
                            </button>
                            <button
                                type="button"
                                className="btn btn-danger"
                                disabled={busy}
                                onClick={() => run(chosen, { planId: plan.planId })}
                            >
                                {busy ? <SabaSpinner size="sm" light /> : t('palette.confirm_run')}
                            </button>
                        </div>
                    </div>
                )}

                {step === 'result' && chosen && (
                    <div className="palette-step">
                        <h3>{actionTitle(chosen)}</h3>
                        <pre className="palette-result">{result}</pre>
                        <div className="palette-actions">
                            <button type="button" className="btn btn-ghost" onClick={back} disabled={busy}>
                                {t('palette.back')}
                            </button>
                        </div>
                    </div>
                )}
                {step === 'list' && <footer className="palette-foot">{t('palette.hint')}</footer>}
            </div>
        </div>
    );
}

/** Ctrl+K (macOS ⌘K)로 여닫는다. 액션 IPC가 없으면 아무것도 하지 않는다 */
export function CommandPalette() {
    const [open, setOpen] = useState(false);
    const busyRef = useRef(false);

    useEffect(() => {
        const onKey = (e) => {
            if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey && e.key.toLowerCase() === 'k') {
                e.preventDefault();
                setOpen((o) => (o && busyRef.current ? o : !o));
            }
        };
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
    }, []);

    return open ? (
        <Palette
            busyRef={busyRef}
            onClose={() => {
                busyRef.current = false;
                setOpen(false);
            }}
        />
    ) : null;
}
