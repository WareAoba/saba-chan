import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';
import { Avatar } from './NodeView';
import { TagInput } from './TagInput';

/** 릴레이 계약 기본값 (/mnt/project-files/chatbot/contract.md). 기본값과 같으면 null로 보내 릴레이 기본을 따른다 */
export const CHATBOT_DEFAULTS = {
    enabled: false,
    model: 'gpt-6-luna',
    persona: '',
    triggerWords: ['사바쨩', '사바'],
    confirmCommands: true,
    ambientSeconds: 0,
    sessionSeconds: 180,
    memory: true,
    webSearch: true,
};
const LIMITS = { ambientSeconds: [0, 86400], sessionSeconds: [0, 600] };
const PERSONA_MAX = 40000;
const TRIGGER_MAX = 10;
const CHANNEL_MAX = 20;

export const chatbotAvailable = () => !!window.api?.chatbotGet;

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/** 저장된 settings → 화면 초안 (숫자 칸은 문자열) */
export function settingsDraft(saved = {}) {
    const v = { ...CHATBOT_DEFAULTS, ...saved };
    return { ...v, ambientSeconds: String(v.ambientSeconds), sessionSeconds: String(v.sessionSeconds) };
}

/** 초안 → PUT settings 패치: 기본값과 같으면 null(키 지움) */
export function settingsPatch(draft) {
    const out = {};
    for (const [k, def] of Object.entries(CHATBOT_DEFAULTS)) {
        let v = draft[k];
        if (k in LIMITS) v = Number(v);
        if (k === 'persona' || k === 'model') v = String(v ?? '').trim();
        if (k === 'model' && !v) v = def;
        out[k] = same(v, def) ? null : v;
    }
    return out;
}

export function settingsProblems(draft, t) {
    const out = [];
    for (const [k, [min, max]] of Object.entries(LIMITS)) {
        const n = Number(draft[k]);
        if (draft[k] === '' || !Number.isInteger(n) || n < min || n > max) {
            out.push(t(`chatbot.err_${k}`, { min, max }));
        }
    }
    if ((draft.persona || '').length > PERSONA_MAX) out.push(t('chatbot.err_persona', { max: PERSONA_MAX }));
    if ((draft.triggerWords || []).length > TRIGGER_MAX) out.push(t('chatbot.err_triggers', { max: TRIGGER_MAX }));
    return out;
}

const guildDraft = (g = {}) => ({
    enabled: typeof g.enabled === 'boolean' ? (g.enabled ? 'on' : 'off') : 'inherit',
    channelIds: Array.isArray(g.channelIds) ? g.channelIds.map(String) : [],
});

const guildPatch = (d) => ({
    enabled: d.enabled === 'inherit' ? null : d.enabled === 'on',
    channelIds: d.channelIds.length ? d.channelIds : null,
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
            {loadError && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span className="mt-pre">{loadError}</span>
                    <button type="button" className="btn btn-ghost btn-sm" onClick={load}>
                        {t('maintenance.retry')}
                    </button>
                </div>
            )}
            {!data && !loadError && <SabaSpinner />}
            {data && (
                <>
                    <ApiKeyCard hasApiKey={!!data.hasApiKey} onChanged={(res) => setData(res)} nameOf={nameOf} />
                    <SettingsEditor data={data} nodes={nodes} nameOf={nameOf} onSaved={(res) => setData(res)} />
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
                    placeholder={hasApiKey ? t('chatbot.key_replace_placeholder') : 'sk-...'}
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

function SettingsEditor({ data, nodes, nameOf, onSaved }) {
    const { t } = useTranslation('gui');
    const savedGuilds = data.guilds || {};
    const guildIds = [...new Set([...nodes.map((n) => n.guildId), ...Object.keys(savedGuilds)])];

    const resetKey = `${JSON.stringify([data.settings, data.guilds])}|${guildIds.join(',')}`;
    const fresh = () => ({
        settings: settingsDraft(data.settings),
        guilds: Object.fromEntries(guildIds.map((id) => [id, guildDraft(savedGuilds[id])])),
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

    const s = draft.settings;
    const set = (k, v) => setDraft((d) => ({ ...d, settings: { ...d.settings, [k]: v } }));
    const setGuild = (id, change) =>
        setDraft((d) => ({ ...d, guilds: { ...d.guilds, [id]: { ...d.guilds[id], ...change } } }));

    const problems = settingsProblems(s, t);
    const badChannels = Object.values(draft.guilds).some((g) => g.channelIds.length > CHANNEL_MAX);
    const dirty = !same(base, draft);

    const save = async () => {
        setSaving(true);
        setErrors([]);
        try {
            const patch = {};
            if (!same(base.settings, draft.settings)) patch.settings = settingsPatch(draft.settings);
            const guilds = {};
            for (const id of guildIds) {
                if (!same(base.guilds[id], draft.guilds[id])) guilds[id] = guildPatch(draft.guilds[id]);
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

    const toggleRow = (key, label, hint) => (
        <div className="cb-toggle-row">
            <span className="dc-list-text">
                <span className="dc-list-name">{label}</span>
                {hint && <span className="cb-hint">{hint}</span>}
            </span>
            <SabaToggle checked={!!s[key]} aria-label={label} onChange={(v) => set(key, v)} />
        </div>
    );

    return (
        <>
            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.basic_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.basic_desc')}</p>
                {toggleRow('enabled', t('chatbot.enabled'), t('chatbot.enabled_hint'))}
                <div className="cb-fields">
                    <label className="mt-field">
                        <span>{t('chatbot.model')}</span>
                        <input
                            className="input"
                            value={s.model}
                            spellCheck={false}
                            placeholder={CHATBOT_DEFAULTS.model}
                            onChange={(e) => set('model', e.target.value)}
                        />
                    </label>
                    <label className="mt-field">
                        <span>{t('chatbot.session_seconds')}</span>
                        <span className="mt-inline">
                            <input
                                className="input mt-num"
                                type="number"
                                min="0"
                                max="600"
                                value={s.sessionSeconds}
                                onChange={(e) => set('sessionSeconds', e.target.value)}
                            />
                            <span className="muted">{t('chatbot.seconds')}</span>
                        </span>
                    </label>
                    <label className="mt-field">
                        <span>{t('chatbot.ambient_seconds')}</span>
                        <span className="mt-inline">
                            <input
                                className="input mt-num"
                                type="number"
                                min="0"
                                max="86400"
                                value={s.ambientSeconds}
                                onChange={(e) => set('ambientSeconds', e.target.value)}
                            />
                            <span className="muted">{t('chatbot.ambient_hint')}</span>
                        </span>
                    </label>
                </div>
                <div className="mt-field">
                    <span>{t('chatbot.triggers')}</span>
                    <TagInput
                        value={s.triggerWords}
                        onChange={(v) => set('triggerWords', v)}
                        placeholder={t('discord.words_add_placeholder')}
                        ariaLabel={t('chatbot.triggers')}
                    />
                </div>
                <label className="mt-field">
                    <span>{t('chatbot.persona')}</span>
                    <textarea
                        className="input cb-persona"
                        rows={5}
                        value={s.persona}
                        placeholder={t('chatbot.persona_placeholder')}
                        onChange={(e) => set('persona', e.target.value)}
                    />
                </label>
                <div className="cb-toggles">
                    {toggleRow('confirmCommands', t('chatbot.confirm_commands'), t('chatbot.confirm_commands_hint'))}
                    {toggleRow('memory', t('chatbot.memory'), t('chatbot.memory_hint'))}
                    {toggleRow('webSearch', t('chatbot.web_search'), t('chatbot.web_search_hint'))}
                </div>
                {problems.length > 0 && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" /> {problems.join(' · ')}
                    </p>
                )}
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('chatbot.guilds_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('chatbot.guilds_desc')}</p>
                {guildIds.length === 0 ? (
                    <p className="muted">{t('chatbot.guilds_empty')}</p>
                ) : (
                    <ul className="dc-list">
                        {guildIds.map((id) => {
                            const node = nodes.find((n) => n.guildId === id);
                            const g = draft.guilds[id];
                            const name = nameOf(id);
                            return (
                                <li key={id} className="cb-guild">
                                    <div className="dc-list-row">
                                        <Avatar name={name} url={node?.iconUrl} size={32} />
                                        <span className="dc-list-text">
                                            <span className="dc-list-name">{name}</span>
                                        </span>
                                        <select
                                            className="input cb-select"
                                            value={g.enabled}
                                            aria-label={t('chatbot.guild_enabled_aria', { name })}
                                            onChange={(e) => setGuild(id, { enabled: e.target.value })}
                                        >
                                            <option value="inherit">
                                                {t('chatbot.guild_inherit', {
                                                    state: s.enabled ? t('chatbot.on') : t('chatbot.off'),
                                                })}
                                            </option>
                                            <option value="on">{t('chatbot.on')}</option>
                                            <option value="off">{t('chatbot.off')}</option>
                                        </select>
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
                                    <div className="cb-guild-channels">
                                        <span>{t('chatbot.channels')}</span>
                                        <TagInput
                                            value={g.channelIds}
                                            onChange={(v) =>
                                                setGuild(id, { channelIds: v.filter((c) => /^\d+$/.test(c)) })
                                            }
                                            placeholder={t('chatbot.channels_placeholder')}
                                            ariaLabel={t('chatbot.channels_aria', { name })}
                                        />
                                    </div>
                                </li>
                            );
                        })}
                    </ul>
                )}
                {badChannels && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" /> {t('chatbot.err_channels', { max: CHANNEL_MAX })}
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
                <button
                    type="button"
                    className="btn btn-primary"
                    disabled={!dirty || saving || problems.length > 0 || badChannels}
                    onClick={save}
                >
                    <Icon name="save" size="sm" /> {t('discord.save')}
                </button>
            </div>
        </>
    );
}
