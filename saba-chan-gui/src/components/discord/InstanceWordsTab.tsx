import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { Server } from '../../stores/useServerStore';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { FindKeyTag } from '../servers/PageFind';
import { SabaSpinner } from '../ui/SabaUI';
import { SaveBar } from './ChatbotView';
import type { InstanceWordsCommand, InstanceWordsDraft, InstanceWordsEntry, ValidationDetail } from './instanceWords';
import { draftOf, errorsByField, isDraftDirty, wordsCommandName } from './instanceWords';
import { TagInput } from './TagInput';
import { failed } from '../../utils/apiResult';

export { draftOf, errorsByField, isDraftDirty };

export const instanceWordsAvailable = () =>
    typeof window.api?.instanceWords === 'function' && typeof window.api?.instanceWordsSave === 'function';

/** 데몬이 돌려주는 인스턴스 낱말 항목 */
interface InstanceWords extends InstanceWordsEntry {
    id: string;
    name: string;
    module?: string;
    defaultWords?: string[];
}

function ErrorLines({ details }: { details?: ValidationDetail[] }) {
    const { t } = useTranslation('gui');
    if (!details?.length) return null;
    const text = (d: ValidationDetail) => {
        const vars = { word: d.word || '', other: d.conflictWith || '' };
        if (d.conflictWith) return t('discord.iw_err_conflict', vars);
        const key = { too_many: 'too_many', reserved: 'reserved', unknown_command: 'unknown_command' }[
            d.reason as string
        ];
        return key ? t(`discord.iw_err_${key}`, vars) : t('discord.iw_err_invalid', vars);
    };
    return (
        <p className="dc-field-error">
            <Icon name="alertTriangle" size="xs" /> {details.map(text).join(' · ')}
        </p>
    );
}

interface InstanceCardProps {
    inst: InstanceWords;
    onSaved: (inst: InstanceWords) => void;
}

/** 이 서버를 부르는 말과 명령마다 부르는 말 (어느 디스코드 서버에서나 같다) */
function InstanceCard({ inst, onSaved }: InstanceCardProps) {
    const { t } = useTranslation('gui');
    const [draft, setDraft] = useState(() => draftOf(inst));
    const [errors, setErrors] = useState<Record<string, ValidationDetail[]>>({});
    const [busy, setBusy] = useState(false);
    useEffect(() => {
        setDraft(draftOf(inst));
        setErrors({});
    }, [inst]);

    const dirty = isDraftDirty(draft, inst);
    const nameOf = (c: InstanceWordsCommand) => wordsCommandName(t, inst.module || '', c);
    const badWords = (field: string) => (errors[field] || []).map((d) => d.word as string).filter(Boolean);

    const save = async () => {
        setBusy(true);
        try {
            const res = await window.api.instanceWordsSave(inst.id, draft);
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
            safeShowToast(t('discord.iw_save_failed', { error: (e as Error).message }), 'error', 5000);
        }
        setBusy(false);
    };

    return (
        <section className="dc-card">
            {/* 다른 서버 탭(설정·유지보수)과 같은 카드: 제목·설명이 카드 안에 (사용자 11-07) */}
            <div className="dc-card-head">
                <h2 className="ss-card-title">
                    <Icon name="discord" size="sm" /> {t('discord.iw_title')}
                </h2>
            </div>
            <p className="dc-card-desc">{t('discord.iw_desc')}</p>
            <div className="dc-word-table">
                <div className="dc-word-row" data-find-row>
                    <div className="dc-word-cmd">
                        <span className="dc-list-name" data-find="name">
                            {t('discord.iw_server_words')}
                        </span>
                        <span className="muted">
                            {t('discord.iw_default', { words: (inst.defaultWords || [inst.name]).join(', ') })}
                        </span>
                    </div>
                    <TagInput
                        allowSpaces
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
                            <div className="dc-word-row" data-find-row>
                                <div className="dc-word-cmd">
                                    <span className={clsx('dc-list-name', { danger: c.risk === 'danger' })} data-find="name">
                                        {nameOf(c)}
                                        {/* 명령 원문(kick)으로 찾을 때만 보인다 */}
                                        <FindKeyTag findKey={c.name} label={nameOf(c)} />
                                    </span>
                                    <span className="muted">{t('discord.iw_default', { words: defaults })}</span>
                                </div>
                                <TagInput
                                    allowSpaces
                                    value={draft.commands[c.name] || []}
                                    onChange={(v) =>
                                        setDraft((d) => ({ ...d, commands: { ...d.commands, [c.name]: v } }))
                                    }
                                    placeholder={t('discord.words_add_placeholder')}
                                    ariaLabel={nameOf(c)}
                                    invalid={badWords(field)}
                                />
                            </div>
                            <ErrorLines details={errors[field]} />
                        </div>
                    );
                })}
            </div>

            {/* 긴 명령 목록 맨 아래가 아니라 화면 아래에 붙는 막대 (사용자 11-06 #9) */}
            {dirty && (
                <SaveBar
                    dirty
                    saving={busy}
                    onRevert={() => {
                        setDraft(draftOf(inst));
                        setErrors({});
                    }}
                    onSave={save}
                />
            )}
        </section>
    );
}

function draftToItem(inst: InstanceWords, draft: InstanceWordsDraft) {
    return {
        words: draft.words,
        commands: (inst.commands || []).map((c) => ({ ...c, aliases: draft.commands[c.name] || [] })),
    };
}

interface InstanceDiscordTabProps {
    server: Server;
}

/**
 * 서버 상세의 디스코드 탭: 이 서버를 부르는 말과 명령마다 부르는 말을 정한다. 낱말은 인스턴스에 딸려 어느 디스코드 서버에서나 같다.
 * 데몬 `GET`·`PUT /api/instance/:id/words`.
 */
export function InstanceDiscordTab({ server }: InstanceDiscordTabProps) {
    const { t } = useTranslation('gui');
    const [inst, setInst] = useState<InstanceWords | null>(null);
    const [error, setError] = useState<string | null>(null);

    const current = useRef(server.id);
    current.current = server.id;

    const load = useCallback(async () => {
        const id = server.id;
        setError(null);
        setInst((prev) => (prev?.id === id ? prev : null));
        try {
            const res = await window.api.instanceWords(id);
            if (current.current !== id) return; // 그사이 다른 서버로 바뀌었다
            if (failed(res) || !Array.isArray(res?.commands)) throw new Error(res?.error || '-');
            setInst(res as InstanceWords);
        } catch (e) {
            if (current.current === id) setError((e as Error).message);
        }
    }, [server.id]);
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
    if (!inst) {
        return (
            <div className="dc-tab">
                <SabaSpinner size="sm" />
            </div>
        );
    }
    return (
        <div className="dc-tab">
            <InstanceCard key={inst.id} inst={inst} onSaved={setInst} />
        </div>
    );
}
