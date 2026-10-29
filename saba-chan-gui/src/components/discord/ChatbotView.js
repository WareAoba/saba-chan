import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaFail } from '../ui/SabaArt';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { PROVIDER_BRANDS } from './providerBrands';

/**
 * 공통 설정 기본값 (데몬 `chatbot.rs` relay_defaults + allowedTools). 기본값과 같으면 null로 보내 기본을 따른다.
 * 호출어는 디스코드 접두사가, 대화 유지시간은 릴레이 고정값이 맡는다 (chatbot-settings-v2 §2).
 * 켜기와 켠 도구는 서버마다 정한다 (ChatbotGuildTab). allowedTools: 전체 허용 도구, null = 모두
 */
export const CHATBOT_DEFAULTS = {
    model: 'gpt-6-luna',
    effort: 'medium',
    ambient: true,
    ambientMinutes: 3,
    ambientEffort: 'low',
    confirmLevel: 'all',
    memory: true,
    allowedTools: null,
};
export const EFFORTS = ['low', 'medium', 'high'];
export const AMBIENT_MINUTES = [1, 3, 5, 10, 30];
export const MEMO_MAX = 8000;

export const chatbotAvailable = () => !!window.api?.chatbotGet;

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/** 저장된 settings → 화면 초안 (계약에서 빠진 키는 버린다) */
export function settingsDraft(saved = {}) {
    return Object.fromEntries(Object.entries(CHATBOT_DEFAULTS).map(([k, def]) => [k, saved[k] ?? def]));
}

/** 초안 → PUT settings 패치: 기본값과 같으면 null(키 지움) */
export function settingsPatch(draft) {
    const out = {};
    for (const [k, def] of Object.entries(CHATBOT_DEFAULTS)) {
        let v = draft[k];
        if (k === 'model') v = String(v ?? '').trim();
        if (k === 'model' && !v) v = def;
        out[k] = same(v, def) ? null : v;
    }
    // 예전 계약의 persona는 memo(길드별)로 바뀌었다: 남아 있으면 지운다
    out.persona = null;
    return out;
}

/**
 * 서버 하나의 초안: 켜기·켠 도구·서버 프롬프트. 값이 없으면 데몬이 준 기본(guildDefault: 새 서버는 꺼짐, 도구 없음.
 * 옛 공통 설정이 아직 옮겨지지 않았으면 그 값)
 */
export const guildDraft = (g = {}, fallback = {}) => ({
    enabled: typeof g.enabled === 'boolean' ? g.enabled : !!fallback.enabled,
    tools: Array.isArray(g.tools) ? g.tools : Array.isArray(fallback.tools) ? fallback.tools : [],
    memo: typeof g.memo === 'string' ? g.memo : '',
});

/** 서버 초안 → PUT guilds 값: 켜기·도구는 늘 그대로 적는다 (공통을 따르지 않는다) */
export const guildPatch = (d) => ({
    enabled: d.enabled,
    tools: d.tools,
    memo: d.memo.trim() ? d.memo : null,
});

/**
 * 고를 수 있는 도구 목록 — 릴레이가 원본 (GET /api/relay/node/:guild/chatbot/tools).
 * {categories, categoryLabels, tools: [{name, category, available, unavailable?, label, hint}]}
 * 표시 이름·설명(label·hint, 언어 → 문구)도 릴레이가 준다 — 새 도구가 앱 업데이트 없이 제 이름으로 보인다
 */
export function useChatbotTools(guildId, keyVersion = 0) {
    const [res, setRes] = useState(null);
    const canAsk = !!(guildId && window.api?.chatbotTools);
    // biome-ignore lint/correctness/useExhaustiveDependencies: keyVersion 은 다시 묻는 신호 (제공사가 바뀐다)
    const load = useCallback(async () => {
        if (!canAsk) return;
        setRes(null);
        try {
            const r = await window.api.chatbotTools(guildId);
            setRes(Array.isArray(r?.tools) ? r : { error: r?.error || 'FAILED' });
        } catch (err) {
            setRes({ error: err.message || 'FAILED' });
        }
    }, [canAsk, guildId, keyVersion]);
    useEffect(() => {
        load();
    }, [load]);
    return { catalog: canAsk ? res : { error: 'NO_GUILD' }, reload: load };
}

/**
 * 카테고리별 도구 스위치. blocked(name) 이면 잠그고 이유를 보인다; 쓸 수 없는 도구(운영 키·제공사)는 이유만 덧붙인다
 */
export function ToolGroups({ catalog, reload, isOn, onToggle, blocked = () => false }) {
    const { t, i18n } = useTranslation('gui');
    const local = (texts, fallback) => texts?.[i18n.language] || texts?.en || fallback;
    if (!catalog) return <SabaSpinner size="sm" />;
    if (catalog.error)
        return (
            <p className="dc-field-error">
                <Icon name="info" size="xs" />{' '}
                {catalog.error === 'NO_GUILD' ? t('chatbot.tools_need_guild') : t('chatbot.tools_failed')}
                {catalog.error !== 'NO_GUILD' && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={reload}>
                        {t('maintenance.retry')}
                    </button>
                )}
            </p>
        );
    return (catalog.categories || []).map((category) => {
        const tools = catalog.tools.filter((tool) => tool.category === category);
        if (!tools.length) return null;
        return (
            <div className="cb-tool-group" key={category}>
                <h3 className="cb-tool-category">{local(catalog.categoryLabels?.[category], category)}</h3>
                <div className="cb-toggles">
                    {tools.map((tool) => {
                        const label = local(tool.label, tool.name);
                        const locked = blocked(tool.name);
                        const notes = [
                            local(tool.hint, ''),
                            locked ? t('chatbot.tool_blocked') : '',
                            !tool.available ? t(`chatbot.tool_unavailable_${tool.unavailable || 'operator_key'}`) : '',
                        ].filter(Boolean);
                        return (
                            <div className={clsx('cb-toggle-row', { locked })} key={tool.name}>
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{label}</span>
                                    {notes.map((n) => (
                                        <span className="cb-hint" key={n}>
                                            {n}
                                        </span>
                                    ))}
                                </span>
                                <SabaToggle
                                    checked={!locked && isOn(tool.name)}
                                    disabled={locked}
                                    aria-label={label}
                                    onChange={(on) => onToggle(tool.name, on)}
                                />
                            </div>
                        );
                    })}
                </div>
            </div>
        );
    });
}

/** 릴레이 400 {guildId, status, error, details} → 읽을 수 있는 줄 */
export function relayErrorLines(res, nameOf) {
    const lines = [];
    const head = [res?.guildId && nameOf(res.guildId), res?.error].filter(Boolean).join(': ');
    if (head) lines.push(head);
    const details = res?.details;
    if (Array.isArray(details)) {
        for (const d of details) {
            if (typeof d === 'string') lines.push(d);
            else if (d)
                lines.push([d.path || d.field, d.message || d.error].filter(Boolean).join(': ') || JSON.stringify(d));
        }
    } else if (details && typeof details === 'object') {
        lines.push(JSON.stringify(details));
    } else if (typeof details === 'string') {
        lines.push(details);
    }
    return lines;
}

/**
 * AI 챗봇 설정 (디스코드 › 봇 › AI 챗봇). 원본은 데몬 config.json `chatbot`,
 * 데몬이 길드마다 릴레이에 맞춘다. API 키는 쓰기만: 화면에는 등록 여부만 보인다.
 */
export function ChatbotView({ nodes = [] }) {
    const { t } = useTranslation('gui');
    const [data, setData] = useState(null);
    const [loadError, setLoadError] = useState(null);
    // 키를 바꿀 때마다 올린다 — 모델 목록과 연결된 제공사를 다시 묻는다
    const [keyVersion, setKeyVersion] = useState(0);

    const load = useCallback(async () => {
        setLoadError(null);
        try {
            const res = await window.api.chatbotGet();
            if (res?.success === false) throw new Error(res.error || t('chatbot.load_failed'));
            setData(res);
        } catch (err) {
            setLoadError(err.message || String(err));
        }
    }, [t]);

    useEffect(() => {
        load();
    }, [load]);

    const nameOf = (id) => nodes.find((n) => n.guildId === id)?.guildName || id;

    return (
        <div className="dc-view">
            <header className="dc-view-head">
                <h1>
                    <Icon name="sabaTalking" size="lg" /> {t('chatbot.title')}
                </h1>
                <p className="muted">{t('chatbot.desc')}</p>
            </header>
            {loadError && <SabaFail message={loadError} onRetry={load} retryLabel={t('maintenance.retry')} />}
            {!data && !loadError && <SabaSpinner />}
            {data && (
                <>
                    {data.pendingRelayDelete && (
                        <div className="notice tone-info" role="status">
                            <Icon name="cloudOff" size="sm" />
                            <span>{t('chatbot.relay_delete_pending')}</span>
                        </div>
                    )}
                    <ApiKeyCard
                        hasApiKey={!!data.hasApiKey}
                        onChanged={(res) => {
                            setData(res);
                            setKeyVersion((v) => v + 1);
                        }}
                        nameOf={nameOf}
                    />
                    <CreditCard
                        guildIds={[...new Set([...nodes.map((n) => n.guildId), ...Object.keys(data.guilds || {})])]}
                        nameOf={nameOf}
                        hasApiKey={!!data.hasApiKey}
                        keyVersion={keyVersion}
                    />
                    <SettingsEditor
                        data={data}
                        nodes={nodes}
                        nameOf={nameOf}
                        onSaved={(res) => setData(res)}
                        keyVersion={keyVersion}
                    />
                </>
            )}
        </div>
    );
}

function ApiKeyCard({ hasApiKey, onChanged, nameOf }) {
    const { t } = useTranslation('gui');
    const [value, setValue] = useState('');
    const [busy, setBusy] = useState(false);
    const [armed, setArmed] = useState(false);

    const send = async (apiKey) => {
        setBusy(true);
        setArmed(false);
        try {
            const res = await window.api.chatbotSave({ apiKey });
            if (res?.success === false) {
                safeShowToast(relayErrorLines(res, nameOf).join('\n') || t('chatbot.save_failed'), 'error', 6000);
                return;
            }
            setValue('');
            onChanged(res);
            safeShowToast(apiKey ? t('chatbot.key_saved') : t('chatbot.key_removed'), 'success', 3000);
        } catch (err) {
            safeShowToast(err.message || String(err), 'error', 6000);
        } finally {
            setBusy(false);
        }
    };

    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('chatbot.key_title')}</h2>
                <StatusChip
                    tone={hasApiKey ? 'ok' : 'idle'}
                    label={hasApiKey ? t('chatbot.key_set') : t('chatbot.key_none')}
                />
            </div>
            <p className="dc-card-desc">{t('chatbot.key_desc')}</p>
            <p className="dc-hint">{t('chatbot.key_providers')}</p>
            <form
                className="cb-key-row"
                onSubmit={(e) => {
                    e.preventDefault();
                    if (value.trim()) send(value.trim());
                }}
            >
                <input
                    className="input"
                    type="password"
                    autoComplete="off"
                    spellCheck={false}
                    value={value}
                    aria-label={t('chatbot.key_title')}
                    placeholder={hasApiKey ? t('chatbot.key_replace_placeholder') : t('chatbot.key_placeholder')}
                    onChange={(e) => setValue(e.target.value)}
                />
                <button type="submit" className="btn btn-primary" disabled={busy || !value.trim()}>
                    <Icon name="key" size="sm" /> {hasApiKey ? t('chatbot.key_replace') : t('chatbot.key_save')}
                </button>
                {hasApiKey && (
                    <button
                        type="button"
                        className={clsx('btn', armed ? 'btn-danger' : 'btn-ghost btn-danger-text')}
                        disabled={busy}
                        onClick={() => (armed ? send(null) : setArmed(true))}
                    >
                        {armed ? t('chatbot.key_remove_confirm') : t('chatbot.key_remove')}
                    </button>
                )}
            </form>
            {!hasApiKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t('chatbot.key_needed')}
                </p>
            )}
        </section>
    );
}

function SettingsEditor({ data, nodes, nameOf, onSaved, keyVersion }) {
    const { t } = useTranslation('gui');
    const guildIds = [...new Set([...nodes.map((n) => n.guildId), ...Object.keys(data.guilds || {})])];
    const { catalog, reload } = useChatbotTools(guildIds[0], keyVersion);

    const resetKey = JSON.stringify(data.settings);
    const [base, setBase] = useState(() => settingsDraft(data.settings));
    const [draft, setDraft] = useState(() => settingsDraft(data.settings));
    // biome-ignore lint/correctness/useExhaustiveDependencies: 저장된 값이 바뀔 때만 초기화
    useEffect(() => {
        const next = settingsDraft(data.settings);
        setBase(next);
        setDraft(next);
    }, [resetKey]);

    const [saving, setSaving] = useState(false);
    const [errors, setErrors] = useState([]);
    const [syncWarn, setSyncWarn] = useState(false);

    const s = draft;
    const set = (k, v) => setDraft((d) => ({ ...d, [k]: v }));
    const dirty = !same(base, draft);

    // 전체 허용 도구: null = 모두. 목록의 도구를 모두 허용하면 다시 null로 (나중에 생긴 도구도 허용되게)
    const allNames = Array.isArray(catalog?.tools) ? catalog.tools.map((x) => x.name) : [];
    const isAllowed = (name) => !Array.isArray(s.allowedTools) || s.allowedTools.includes(name);
    const toggleAllowed = (name, on) => {
        const now = Array.isArray(s.allowedTools) ? s.allowedTools : allNames;
        const next = on ? [...new Set([...now, name])] : now.filter((x) => x !== name);
        set('allowedTools', allNames.every((x) => next.includes(x)) ? null : allNames.filter((x) => next.includes(x)));
    };

    const save = async () => {
        setSaving(true);
        setErrors([]);
        try {
            const res = await window.api.chatbotSave({ settings: settingsPatch(draft) });
            if (res?.success === false) {
                setErrors(
                    relayErrorLines(res, nameOf).concat(res.guildId || res.error ? [] : [t('chatbot.save_failed')]),
                );
                return;
            }
            setSyncWarn(Array.isArray(res?.sync) && res.sync.some((r) => !r.ok));
            onSaved(res);
            safeShowToast(t('chatbot.saved'), 'success', 3000);
        } catch (err) {
            setErrors([err.message || String(err)]);
        } finally {
            setSaving(false);
        }
    };

    const toggleRow = (key, label, hint) => (
        <div className="cb-toggle-row">
            <span className="dc-list-text">
                <span className="dc-list-name">{label}</span>
                {hint && <span className="cb-hint">{hint}</span>}
            </span>
            <SabaToggle checked={!!s[key]} aria-label={label} onChange={(v) => set(key, v)} />
        </div>
    );

    const selectField = (key, label, hint, options, disabled = false) => (
        <div className="mt-field">
            <span>{label}</span>
            <CustomDropdown
                className="cb-select"
                value={s[key]}
                ariaLabel={label}
                disabled={disabled}
                onChange={(v) => set(key, v)}
                options={options}
            />
            {hint && <p className="dc-hint">{hint}</p>}
        </div>
    );
    const effortOptions = EFFORTS.map((e) => ({ value: e, label: t(`chatbot.effort_${e}`) }));

    return (
        <>
            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.basic_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.basic_desc')}</p>
                <p className="dc-hint">{t('chatbot.guilds_moved')}</p>
                <ModelField
                    value={s.model}
                    saved={base.model}
                    onChange={(v) => set('model', v)}
                    guildId={guildIds[0]}
                    hasApiKey={!!data.hasApiKey}
                    keyVersion={keyVersion}
                />
                {selectField('effort', t('chatbot.effort'), t('chatbot.effort_hint'), effortOptions)}
                <p className="dc-hint">{t('chatbot.prefix_calls')}</p>
                <div className="cb-toggles">{toggleRow('memory', t('chatbot.memory'), t('chatbot.memory_hint'))}</div>
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.ambient_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.ambient_desc')}</p>
                <div className="cb-toggles">
                    {toggleRow('ambient', t('chatbot.ambient'), t('chatbot.ambient_hint'))}
                </div>
                {selectField(
                    'ambientMinutes',
                    t('chatbot.ambient_minutes'),
                    null,
                    // CLI·디스코드로 목록 밖 값(1-60)을 저장했으면 그 값도 보인다
                    [...new Set([...AMBIENT_MINUTES, s.ambientMinutes].filter(Number.isInteger))]
                        .sort((a, b) => a - b)
                        .map((m) => ({ value: m, label: t('chatbot.ambient_minutes_option', { count: m }) })),
                    !s.ambient,
                )}
                {selectField(
                    'ambientEffort',
                    t('chatbot.ambient_effort'),
                    t('chatbot.ambient_effort_hint'),
                    effortOptions,
                    !s.ambient,
                )}
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.tools_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.tools_desc')}</p>
                <h3 className="cb-tool-category">{t('chatbot.allowed_title')}</h3>
                <p className="dc-hint">{t('chatbot.allowed_desc')}</p>
                <ToolGroups catalog={catalog} reload={reload} isOn={isAllowed} onToggle={toggleAllowed} />
            </section>

            {errors.length > 0 && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="mt-pre">{errors.join('\n')}</span>
                </div>
            )}
            {syncWarn && (
                <div className="notice tone-info">
                    <Icon name="cloudOff" size="sm" />
                    <span>{t('chatbot.sync_pending')}</span>
                </div>
            )}

            <SaveBar
                dirty={dirty}
                saving={saving}
                onRevert={() => {
                    setDraft(base);
                    setErrors([]);
                }}
                onSave={save}
            />
        </>
    );
}

/** 저장 막대 — 공통 메뉴와 서버 탭이 같이 쓴다 */
export function SaveBar({ dirty, saving, blocked = false, onRevert, onSave }) {
    const { t } = useTranslation('gui');
    return (
        <div className={clsx('dc-savebar', { dirty })}>
            <span className="dc-savebar-text">{dirty ? t('discord.unsaved') : ''}</span>
            {dirty && (
                <button type="button" className="btn btn-ghost" disabled={saving} onClick={onRevert}>
                    {t('discord.revert')}
                </button>
            )}
            <button type="button" className="btn btn-primary" disabled={!dirty || saving || blocked} onClick={onSave}>
                <Icon name="save" size="sm" /> {t('discord.save')}
            </button>
        </div>
    );
}

const CREDIT_ERRORS = ['NO_API_KEY', 'INVALID_API_KEY', 'KEY_PERMISSION', 'QUOTA_EXCEEDED', 'PROVIDER_UNAVAILABLE'];

/**
 * 남은 크레딧: 잔액은 키로 잔액을 알려 주는 제공사만(릴레이가 묻는다), 사용량은 이번 달 길드마다 (릴레이 집계).
 * 키는 길드마다 같으니 잔액은 첫 길드 답의 것을 보인다
 */
export function CreditCard({ guildIds, nameOf, hasApiKey, keyVersion = 0 }) {
    const { t, i18n } = useTranslation('gui');
    const [rows, setRows] = useState(null);
    // 늦게 온 앞 요청의 답이 새 답을 덮지 않게
    const asked = useRef(0);
    const canAsk = !!(guildIds.length && hasApiKey && window.api?.chatbotCredit);
    const idsKey = guildIds.join(',');

    // biome-ignore lint/correctness/useExhaustiveDependencies: idsKey·keyVersion 이 다시 묻는 신호
    const load = useCallback(async () => {
        const seq = ++asked.current;
        setRows(null);
        if (!canAsk) return;
        const ids = idsKey.split(',');
        const answers = await Promise.all(
            ids.map((id) =>
                window.api
                    .chatbotCredit(id)
                    .then((r) =>
                        r?.error || r?.success === false || !r?.usage
                            ? { id, error: r?.error || 'FAILED' }
                            : { id, ...r },
                    )
                    .catch(() => ({ id, error: 'FAILED' })),
            ),
        );
        if (seq === asked.current) setRows(answers);
    }, [canAsk, idsKey, keyVersion]);

    useEffect(() => {
        load();
    }, [load]);

    if (!window.api?.chatbotCredit) return null;
    const first = rows?.find((r) => !r.error);
    const number = (n) => Number(n || 0).toLocaleString(i18n.language);
    const money = (b) => {
        try {
            return new Intl.NumberFormat(i18n.language, { style: 'currency', currency: b.currency }).format(b.amount);
        } catch {
            return `${b.amount} ${b.currency}`;
        }
    };
    const providerName = (id) => (PROVIDER_BRANDS[id] ?? PROVIDER_BRANDS[String(id).replace(/-cn$/, '')])?.name ?? id;
    const exhausted = rows?.map((r) => r.exhaustedAt).find(Boolean);
    const errorKey = !hasApiKey
        ? 'chatbot.models_need_key'
        : !guildIds.length
          ? 'chatbot.models_need_guild'
          : rows && !first
            ? `chatbot.models_err_${CREDIT_ERRORS.includes(rows[0]?.error) ? rows[0].error : 'FAILED'}`
            : null;

    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('chatbot.credit_title')}</h2>
                {canAsk && (
                    <button type="button" className="btn btn-ghost btn-sm" disabled={!rows} onClick={load}>
                        {t('chatbot.credit_refresh')}
                    </button>
                )}
            </div>
            <p className="dc-card-desc">{t('chatbot.credit_desc')}</p>
            {canAsk && !rows && <SabaSpinner size="sm" />}
            {errorKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t(errorKey)}
                </p>
            )}
            {exhausted && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('chatbot.credit_exhausted', { time: new Date(exhausted).toLocaleString(i18n.language) })}
                    </span>
                </div>
            )}
            {first && (
                <>
                    <div className="mt-field">
                        <span>{t('chatbot.credit_balance')}</span>
                        {first.balance ? (
                            <strong>{money(first.balance)}</strong>
                        ) : (
                            <p className="dc-hint">
                                {first.balanceSupported
                                    ? t('chatbot.credit_balance_none')
                                    : t('chatbot.credit_balance_unsupported', {
                                          provider: providerName(first.provider),
                                      })}
                            </p>
                        )}
                    </div>
                    <div className="mt-field">
                        <span>{t('chatbot.credit_usage', { month: first.usage.month })}</span>
                        <ul className="dc-list">
                            {rows.map((r) => (
                                <li key={r.id} className="dc-list-row">
                                    <span className="dc-list-text">
                                        <span className="dc-list-name">{nameOf(r.id)}</span>
                                        <span className="cb-hint">
                                            {r.error
                                                ? t('chatbot.credit_guild_failed')
                                                : t('chatbot.credit_usage_line', {
                                                      calls: number(r.usage.calls),
                                                      input: number(r.usage.input),
                                                      cached: number(r.usage.cached),
                                                      output: number(r.usage.output),
                                                  })}
                                        </span>
                                    </span>
                                </li>
                            ))}
                        </ul>
                    </div>
                </>
            )}
        </section>
    );
}

/**
 * 모델 고르기: 릴레이가 사용자 키로 물은 목록에서 고른다 (chatbot-settings-v2 §1).
 * 고른 모델을 쓸 수 없어 릴레이가 다른 모델로 대신 쓰고 있으면 알리고, 그 모델을 고른 상태로 둔다
 * (저장하면 그 모델이 새 설정이 된다).
 */
export function ModelField({ value, saved, onChange, guildId, hasApiKey, keyVersion = 0 }) {
    const { t } = useTranslation('gui');
    const [res, setRes] = useState(null);
    const canAsk = !!(guildId && hasApiKey && window.api?.chatbotModels);

    // keyVersion: 키를 바꾸면 같은 길드라도 다시 묻는다 (제공사·목록이 바뀐다)
    // biome-ignore lint/correctness/useExhaustiveDependencies: keyVersion 은 다시 묻는 신호
    const load = useCallback(async () => {
        if (!canAsk) return;
        setRes({ loading: true });
        try {
            const r = await window.api.chatbotModels(guildId);
            setRes(r?.error || r?.success === false || !Array.isArray(r?.models) ? { error: r?.error || 'FAILED' } : r);
        } catch (err) {
            setRes({ error: err.message || 'FAILED' });
        }
    }, [canAsk, guildId, keyVersion]);

    useEffect(() => {
        setRes(null);
        load();
    }, [load]);

    const fallback = res?.effectiveModel && res.effectiveModel !== saved ? res.effectiveModel : null;
    // 대신 쓰는 모델이 있으면 처음 한 번 그쪽을 고른다 (저장 전까지는 되돌릴 수 있다)
    // biome-ignore lint/correctness/useExhaustiveDependencies: 목록을 새로 받았을 때만
    useEffect(() => {
        if (fallback && value === saved) onChange(fallback);
    }, [fallback]);

    const models = res?.models || [];
    const options = value && !models.includes(value) ? [value, ...models] : models;
    const errorKey = !hasApiKey
        ? 'chatbot.models_need_key'
        : !guildId
          ? 'chatbot.models_need_guild'
          : res?.error
            ? `chatbot.models_err_${['NO_API_KEY', 'INVALID_API_KEY', 'KEY_PERMISSION', 'QUOTA_EXCEEDED', 'PROVIDER_UNAVAILABLE'].includes(res.error) ? res.error : 'FAILED'}`
            : null;

    return (
        <div className="mt-field">
            <span>{t('chatbot.model')}</span>
            <span className="mt-inline">
                <CustomDropdown
                    className="cb-select"
                    value={value}
                    ariaLabel={t('chatbot.model')}
                    disabled={!res || res.loading || !!res.error}
                    onChange={onChange}
                    options={(options.length ? options : [value]).map((m) => ({ value: m, label: m }))}
                />
                {res?.loading && <SabaSpinner size="sm" />}
                {res?.error && canAsk && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={load}>
                        {t('maintenance.retry')}
                    </button>
                )}
            </span>
            {errorKey && (
                <p className="dc-field-error">
                    <Icon name="info" size="xs" /> {t(errorKey)}
                </p>
            )}
            {res?.provider && !res.error && <ProviderBadge provider={res.provider} />}
            {fallback && <p className="dc-hint">{t('chatbot.model_fallback', { model: saved, fallback })}</p>}
        </div>
    );
}

/** 릴레이가 키로 알아낸 제공사 — 로고와 이름 */
export function ProviderBadge({ provider }) {
    const { t } = useTranslation('gui');
    // 중국판이 따로 이름이 있으면(zhipu-cn) 그대로, 없으면 국제판 이름에 지역을 붙인다
    const own = PROVIDER_BRANDS[provider];
    const brand = own ?? PROVIDER_BRANDS[String(provider).replace(/-cn$/, '')];
    if (!brand) return null;
    const name =
        !own && String(provider).endsWith('-cn') ? t('chatbot.provider_region_cn', { name: brand.name }) : brand.name;
    return (
        <p className="cb-provider">
            <span
                className="cb-provider-logo"
                // biome-ignore lint/security/noDangerouslySetInnerHtml: 저장소에 넣은 고정 SVG (providerBrands.js)
                dangerouslySetInnerHTML={{ __html: brand.svg }}
            />
            {t('chatbot.provider_connected', { provider: name })}
        </p>
    );
}
