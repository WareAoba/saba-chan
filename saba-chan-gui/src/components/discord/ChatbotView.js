import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaEmpty, SabaFail } from '../ui/SabaArt';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { Avatar } from './NodeView';
import { PROVIDER_BRANDS } from './providerBrands';

/**
 * 릴레이 계약 기본값 (데몬 `chatbot.rs` relay_defaults). 기본값과 같으면 null로 보내 릴레이 기본을 따른다.
 * 호출어는 디스코드 접두사가, 대화 유지시간은 릴레이 고정값이 맡는다 (chatbot-settings-v2 §2)
 */
export const CHATBOT_DEFAULTS = {
    enabled: false,
    model: 'gpt-6-luna',
    effort: 'medium',
    ambient: true,
    ambientMinutes: 3,
    ambientEffort: 'low',
    confirmLevel: 'all',
    memory: true,
    webSearch: true,
    disabledTools: [],
};
/** 끌 수 있는 도구 묶음 (계약 disabledTools). 눈치 메모는 끌 수 없는 기본 사양이라 없다 */
export const CHATBOT_TOOLS = ['game_servers', 'research', 'lol', 'images'];
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
        if (k === 'disabledTools') v = CHATBOT_TOOLS.filter((x) => (v || []).includes(x));
        if (k === 'model' && !v) v = def;
        out[k] = same(v, def) ? null : v;
    }
    // 예전 계약의 persona는 memo(길드별)로 바뀌었다: 남아 있으면 지운다
    out.persona = null;
    return out;
}

const guildDraft = (g = {}) => ({
    enabled: typeof g.enabled === 'boolean' ? (g.enabled ? 'on' : 'off') : 'inherit',
    memo: typeof g.memo === 'string' ? g.memo : '',
});

const guildPatch = (d) => ({
    enabled: d.enabled === 'inherit' ? null : d.enabled === 'on',
    memo: d.memo.trim() ? d.memo : null,
});

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
                <h1>{t('chatbot.title')}</h1>
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
    const { t, i18n } = useTranslation('gui');
    const savedGuilds = data.guilds || {};
    const guildIds = [...new Set([...nodes.map((n) => n.guildId), ...Object.keys(savedGuilds)])];

    // 초안에는 저장된 길드만 넣는다. 노드 목록은 나중에 올 수 있으니(릴레이 응답) 없는 길드는 그때그때 빈 초안으로 본다
    const resetKey = JSON.stringify([data.settings, data.guilds]);
    const fresh = () => ({
        settings: settingsDraft(data.settings),
        guilds: Object.fromEntries(Object.keys(savedGuilds).map((id) => [id, guildDraft(savedGuilds[id])])),
    });
    const [base, setBase] = useState(fresh);
    const [draft, setDraft] = useState(fresh);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 저장된 값이 바뀔 때만 초기화
    useEffect(() => {
        const next = fresh();
        setBase(next);
        setDraft(next);
    }, [resetKey]);

    const [saving, setSaving] = useState(false);
    const [errors, setErrors] = useState([]);
    const [syncWarn, setSyncWarn] = useState(false);
    const [memArmed, setMemArmed] = useState(null);
    const [templateBusy, setTemplateBusy] = useState(null);

    const s = draft.settings;
    const set = (k, v) => setDraft((d) => ({ ...d, settings: { ...d.settings, [k]: v } }));
    const baseGuild = (id) => base.guilds[id] ?? guildDraft(savedGuilds[id]);
    const draftGuild = (id) => draft.guilds[id] ?? baseGuild(id);
    const setGuild = (id, change) =>
        setDraft((d) => ({
            ...d,
            guilds: { ...d.guilds, [id]: { ...(d.guilds[id] ?? baseGuild(id)), ...change } },
        }));

    const badMemo = guildIds.some((id) => draftGuild(id).memo.length > MEMO_MAX);
    const dirty = !same(base.settings, draft.settings) || guildIds.some((id) => !same(baseGuild(id), draftGuild(id)));

    const save = async () => {
        setSaving(true);
        setErrors([]);
        try {
            const patch = {};
            if (!same(base.settings, draft.settings) || data.settings?.persona !== undefined) {
                patch.settings = settingsPatch(draft.settings);
            }
            const guilds = {};
            for (const id of guildIds) {
                if (!same(baseGuild(id), draftGuild(id))) guilds[id] = guildPatch(draftGuild(id));
            }
            if (Object.keys(guilds).length) patch.guilds = guilds;
            const res = await window.api.chatbotSave(patch);
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

    const clearMemory = async (guildId) => {
        setMemArmed(null);
        try {
            const res = await window.api.chatbotClearMemory(guildId);
            if (res?.success === false) throw new Error(res.error || t('chatbot.save_failed'));
            safeShowToast(t('chatbot.memory_cleared', { name: nameOf(guildId) }), 'success', 3000);
        } catch (err) {
            safeShowToast(err.message || String(err), 'error', 6000);
        }
    };

    // 서버 프롬프트 기본 틀은 데몬이 앱 언어로 준다 (GET /api/chatbot/memo-template)
    const insertTemplate = async (guildId) => {
        setTemplateBusy(guildId);
        try {
            const res = await window.api.chatbotMemoTemplate(i18n.language);
            if (res?.success === false || typeof res?.template !== 'string') throw new Error(res?.error || '');
            // 기다리는 사이 무언가 썼으면 덮지 않는다
            setDraft((d) => {
                const g = d.guilds[guildId] ?? baseGuild(guildId);
                return g.memo.trim() ? d : { ...d, guilds: { ...d.guilds, [guildId]: { ...g, memo: res.template } } };
            });
        } catch (err) {
            safeShowToast(err.message || t('chatbot.memo_template_failed'), 'error', 6000);
        } finally {
            setTemplateBusy(null);
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
                {toggleRow('enabled', t('chatbot.enabled'), t('chatbot.enabled_hint'))}
                <ModelField
                    value={s.model}
                    saved={base.settings.model}
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
                <div className="cb-toggles">
                    {toggleRow('webSearch', t('chatbot.web_search'), t('chatbot.web_search_hint'))}
                    {CHATBOT_TOOLS.map((tool) => {
                        const label = t(`chatbot.tool_${tool}`);
                        const off = (s.disabledTools || []).includes(tool);
                        return (
                            <div className="cb-toggle-row" key={tool}>
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{label}</span>
                                    <span className="cb-hint">{t(`chatbot.tool_${tool}_hint`)}</span>
                                </span>
                                <SabaToggle
                                    checked={!off}
                                    aria-label={label}
                                    onChange={(on) =>
                                        set(
                                            'disabledTools',
                                            on
                                                ? (s.disabledTools || []).filter((x) => x !== tool)
                                                : [...(s.disabledTools || []), tool],
                                        )
                                    }
                                />
                            </div>
                        );
                    })}
                </div>
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.guilds_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.guilds_desc')}</p>
                {guildIds.length === 0 ? (
                    <SabaEmpty compact body={t('chatbot.guilds_empty')} />
                ) : (
                    <ul className="dc-list">
                        {guildIds.map((id) => {
                            const node = nodes.find((n) => n.guildId === id);
                            const g = draftGuild(id);
                            const name = nameOf(id);
                            return (
                                <li key={id} className="cb-guild">
                                    <div className="dc-list-row">
                                        <Avatar name={name} url={node?.iconUrl} size={32} />
                                        <span className="dc-list-text">
                                            <span className="dc-list-name">{name}</span>
                                        </span>
                                        <CustomDropdown
                                            className="cb-select"
                                            value={g.enabled}
                                            ariaLabel={t('chatbot.guild_enabled_aria', { name })}
                                            onChange={(v) => setGuild(id, { enabled: v })}
                                            options={[
                                                {
                                                    value: 'inherit',
                                                    label: t('chatbot.guild_inherit', {
                                                        state: s.enabled ? t('chatbot.on') : t('chatbot.off'),
                                                    }),
                                                },
                                                { value: 'on', label: t('chatbot.on') },
                                                { value: 'off', label: t('chatbot.off') },
                                            ]}
                                        />
                                        {window.api?.chatbotClearMemory && (
                                            <button
                                                type="button"
                                                className={clsx(
                                                    'btn btn-sm',
                                                    memArmed === id ? 'btn-danger' : 'btn-ghost',
                                                )}
                                                onClick={() => (memArmed === id ? clearMemory(id) : setMemArmed(id))}
                                            >
                                                {memArmed === id
                                                    ? t('chatbot.memory_clear_confirm')
                                                    : t('chatbot.memory_clear')}
                                            </button>
                                        )}
                                    </div>
                                    <p className="dc-hint">{t('chatbot.channels_moved')}</p>
                                    {/* label 이 아니라 div: 안의 기본 틀 버튼이 label 의 이름을 가져가지 않게 (글 상자는 aria-label) */}
                                    <div className="cb-guild-memo">
                                        <span className="cb-memo-head">
                                            <span>{t('chatbot.memo')}</span>
                                            {!g.memo.trim() && window.api?.chatbotMemoTemplate && (
                                                <button
                                                    type="button"
                                                    className="btn btn-ghost btn-sm"
                                                    disabled={templateBusy === id}
                                                    onClick={(e) => {
                                                        e.preventDefault();
                                                        insertTemplate(id);
                                                    }}
                                                >
                                                    {t('chatbot.memo_template')}
                                                </button>
                                            )}
                                            <span className={clsx('cb-counter', { over: g.memo.length > MEMO_MAX })}>
                                                {g.memo.length.toLocaleString()} / {MEMO_MAX.toLocaleString()}
                                            </span>
                                        </span>
                                        <textarea
                                            className="input cb-persona"
                                            rows={4}
                                            value={g.memo}
                                            aria-label={t('chatbot.memo_aria', { name })}
                                            placeholder={t('chatbot.memo_placeholder')}
                                            onChange={(e) => setGuild(id, { memo: e.target.value })}
                                        />
                                    </div>
                                </li>
                            );
                        })}
                    </ul>
                )}
                {badMemo && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" /> {t('chatbot.err_memo', { max: MEMO_MAX })}
                    </p>
                )}
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

            <div className={clsx('dc-savebar', { dirty })}>
                <span className="dc-savebar-text">{dirty ? t('discord.unsaved') : ''}</span>
                {dirty && (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={saving}
                        onClick={() => {
                            setDraft(base);
                            setErrors([]);
                        }}
                    >
                        {t('discord.revert')}
                    </button>
                )}
                <button type="button" className="btn btn-primary" disabled={!dirty || saving || badMemo} onClick={save}>
                    <Icon name="save" size="sm" /> {t('discord.save')}
                </button>
            </div>
        </>
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
