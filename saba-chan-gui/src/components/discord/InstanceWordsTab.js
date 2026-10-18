import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { TagInput } from './TagInput';

export const instanceWordsAvailable = () => Boolean(window.api?.guildInstances && window.api?.guildInstanceWordsSave);

const failed = (res) => !res || res.success === false || Boolean(res.error_code) || Boolean(res.error);
const sameList = (a = [], b = []) => a.length === b.length && a.every((w, i) => w === b[i]);

/** 데몬 항목 → 고칠 수 있는 초안 `{words, commands: {명령: [낱말]}}` */
export function draftOf(inst) {
    return {
        words: [...(inst.words || [])],
        commands: Object.fromEntries((inst.commands || []).map((c) => [c.name, [...(c.aliases || [])]])),
    };
}

export function isDraftDirty(draft, inst) {
    if (!draft || !inst) return false;
    if (!sameList(draft.words, inst.words || [])) return true;
    return (inst.commands || []).some((c) => !sameList(draft.commands[c.name], c.aliases || []));
}

/** 400 `validation_failed`의 details를 칸별로 나눈다: `words`, `commands.<명령>` */
export function errorsByField(details) {
    const out = {};
    for (const d of details || []) {
        const key = String(d?.field || '').replace(/\[\d+\]$/, '') || 'words';
        (out[key] ||= []).push(d);
    }
    return out;
}

function ErrorLines({ details }) {
    const { t } = useTranslation('gui');
    if (!details?.length) return null;
    const text = (d) => {
        const vars = { word: d.word || '', other: d.conflictWith || '' };
        if (d.conflictWith) return t('discord.iw_err_conflict', vars);
        const key = { too_many: 'too_many', reserved: 'reserved', unknown_command: 'unknown_command' }[d.reason];
        return key ? t(`discord.iw_err_${key}`, vars) : t('discord.iw_err_invalid', vars);
    };
    return (
        <p className="dc-field-error">
            <Icon name="alertTriangle" size="xs" /> {details.map(text).join(' · ')}
        </p>
    );
}

function InstanceCard({ guildId, inst, onSaved }) {
    const { t } = useTranslation('gui');
    const [draft, setDraft] = useState(() => draftOf(inst));
    const [errors, setErrors] = useState({});
    const [busy, setBusy] = useState(false);
    useEffect(() => {
        setDraft(draftOf(inst));
        setErrors({});
    }, [inst]);

    const dirty = isDraftDirty(draft, inst);
    const badWords = (field) => (errors[field] || []).map((d) => d.word).filter(Boolean);

    const save = async () => {
        setBusy(true);
        try {
            const res = await window.api.guildInstanceWordsSave(guildId, inst.id, draft);
            if (res?.error_code === 'validation_failed') {
                const byField = errorsByField(res.details);
                setErrors(byField);
                // 본문 모양 문제(`input`)는 칸이 없으므로 알림으로 보인다
                if (byField.input)
                    safeShowToast(t('discord.iw_save_failed', { error: res.error || 'input' }), 'error', 5000);
            } else if (failed(res))
                safeShowToast(t('discord.iw_save_failed', { error: res?.error || '-' }), 'error', 5000);
            else {
                setErrors({});
                // 데몬은 저장한 `{words, commands:{명령:[..]}}`를 돌려준다 (빈 명령은 빠진다)
                const saved = res?.words ? { words: res.words, commands: res.commands || {} } : draft;
                onSaved({ ...inst, ...draftToItem(inst, saved) });
                safeShowToast(t('discord.iw_saved', { name: inst.name }), 'success', 2500);
            }
        } catch (e) {
            safeShowToast(t('discord.iw_save_failed', { error: e.message }), 'error', 5000);
        }
        setBusy(false);
    };

    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{inst.name}</h2>
                <code className="dc-code-tag">{inst.module}</code>
            </div>

            <div className="dc-word-table">
                <div className="dc-word-row">
                    <div className="dc-word-cmd">
                        <span className="dc-list-name">{t('discord.iw_server_words')}</span>
                        <span className="muted">{t('discord.iw_default', { words: inst.name })}</span>
                    </div>
                    <TagInput
                        value={draft.words}
                        onChange={(v) => setDraft((d) => ({ ...d, words: v }))}
                        placeholder={t('discord.words_add_placeholder')}
                        ariaLabel={t('discord.iw_server_words_aria', { name: inst.name })}
                        invalid={badWords('words')}
                    />
                </div>
                <ErrorLines details={errors.words} />

                {(inst.commands || []).length === 0 && <p className="muted">{t('discord.no_commands')}</p>}
                {(inst.commands || []).map((c) => {
                    const field = `commands.${c.name}`;
                    const defaults = [c.name, ...(c.moduleAliases || [])].join(', ');
                    return (
                        <div key={c.name}>
                            <div className="dc-word-row">
                                <div className="dc-word-cmd">
                                    <span className={clsx('dc-list-name', { danger: c.risk === 'danger' })}>
                                        {c.label || c.name}
                                    </span>
                                    <span className="muted">{t('discord.iw_default', { words: defaults })}</span>
                                </div>
                                <TagInput
                                    value={draft.commands[c.name] || []}
                                    onChange={(v) =>
                                        setDraft((d) => ({ ...d, commands: { ...d.commands, [c.name]: v } }))
                                    }
                                    placeholder={t('discord.words_add_placeholder')}
                                    ariaLabel={c.label || c.name}
                                    invalid={badWords(field)}
                                />
                            </div>
                            <ErrorLines details={errors[field]} />
                        </div>
                    );
                })}
            </div>

            {dirty && (
                <div className="dc-row-actions">
                    <span className="muted">{t('discord.unsaved')}</span>
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={busy}
                        onClick={() => {
                            setDraft(draftOf(inst));
                            setErrors({});
                        }}
                    >
                        {t('discord.revert')}
                    </button>
                    <button type="button" className="btn btn-primary" disabled={busy} onClick={save}>
                        <Icon name="save" size="sm" /> {t('discord.save')}
                    </button>
                </div>
            )}
        </section>
    );
}

function draftToItem(inst, draft) {
    return {
        words: draft.words,
        commands: (inst.commands || []).map((c) => ({ ...c, aliases: draft.commands[c.name] || [] })),
    };
}

/**
 * 디스코드 서버(길드) 하나에서 인스턴스마다 부르는 말과 명령마다 부르는 말을 정한다.
 * 데몬 `GET /api/guilds/:guild/instances`, `PUT /api/guilds/:guild/instances/:id/words`.
 */
export function InstanceWordsTab({ node }) {
    const { t } = useTranslation('gui');
    const guildId = node.guildId;
    const [items, setItems] = useState(null);
    const [error, setError] = useState(null);

    const load = useCallback(async () => {
        setError(null);
        try {
            const res = await window.api.guildInstances(guildId);
            if (!Array.isArray(res)) throw new Error(res?.error || '-');
            setItems(res);
        } catch (e) {
            setError(e.message);
        }
    }, [guildId]);
    useEffect(() => {
        load();
    }, [load]);

    if (error) {
        return (
            <div className="dc-tab">
                <p className="dc-field-error">
                    <Icon name="alertTriangle" size="xs" /> {t('discord.iw_load_failed', { error })}
                </p>
                <button type="button" className="btn btn-secondary btn-sm" onClick={load}>
                    {t('maintenance.retry')}
                </button>
            </div>
        );
    }
    if (!items) {
        return (
            <div className="dc-tab">
                <SabaSpinner size="sm" />
            </div>
        );
    }
    return (
        <div className="dc-tab">
            <p className="dc-card-desc">{t('discord.iw_desc')}</p>
            {items.length === 0 && <p className="muted pad">{t('discord.iw_none')}</p>}
            {items.map((inst) => (
                <InstanceCard
                    key={inst.id}
                    guildId={guildId}
                    inst={inst}
                    onSaved={(next) => setItems((list) => list.map((x) => (x.id === inst.id ? next : x)))}
                />
            ))}
        </div>
    );
}
