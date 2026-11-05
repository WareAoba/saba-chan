import {
    draftOf,
    errorsByField,
    type InstanceWordsCommand,
    type InstanceWordsDraft,
    type InstanceWordsEntry,
    isDraftDirty,
    type ValidationDetail,
} from '@gui/components/discord/instanceWords';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Button, Card, errorText, failed, Loading } from './ui';
import type { Server, WebApi } from '../types';

/** GET /api/instance/:id/words */
interface InstanceWordsInfo extends InstanceWordsEntry {
    id: string;
    name: string;
    defaultWords?: string[];
    commands: InstanceWordsCommand[];
}

/** 검증 실패 한 건 (데몬 validation_failed 의 details) */
type WordError = ValidationDetail & { word?: string; conflictWith?: string; reason?: string };

/** 낱말 칩과 입력 칸: Enter·쉼표로 더하고 칩의 ×로 뺀다 */
interface WordsInputProps {
    value: string[];
    onChange: (value: string[]) => void;
    label: string;
    placeholder: string;
    invalid?: string[];
}

function WordsInput({ value, onChange, label, placeholder, invalid = [] }: WordsInputProps) {
    const { t } = useTranslation();
    const [text, setText] = useState('');
    const add = () => {
        const words = text
            .split(',')
            .map((w) => w.trim())
            .filter((w) => w && !value.includes(w));
        if (words.length) onChange([...value, ...words]);
        setText('');
    };
    return (
        <div className="words-input">
            {value.map((w) => (
                <span key={w} className={invalid.includes(w) ? 'chip word-chip bad' : 'chip word-chip'}>
                    {w}
                    <button
                        type="button"
                        className="word-remove"
                        aria-label={t('words.remove', { word: w })}
                        onClick={() => onChange(value.filter((x) => x !== w))}
                    >
                        ×
                    </button>
                </span>
            ))}
            <input
                type="text"
                aria-label={label}
                placeholder={placeholder}
                value={text}
                maxLength={64}
                onChange={(e) => setText(e.target.value)}
                onBlur={add}
                onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ',') {
                        e.preventDefault();
                        add();
                    }
                }}
            />
        </div>
    );
}

/**
 * 디스코드 호출어 (2026-11-03 사용자 지정 시안: 서버 상세 › 설정 › 사바쨩). GUI 서버 상세의 디스코드 탭과 같은 값:
 * 이 서버를 부르는 말과 명령마다 부르는 말. 어느 디스코드 서버에서나 같다. 데몬 GET·PUT /api/instance/:id/words
 */
export function InstanceWords({ api, server }: { api: WebApi; server: Pick<Server, 'id'> }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const [inst, setInst] = useState<InstanceWordsInfo | false | null>(null);
    const [draft, setDraft] = useState<InstanceWordsDraft | null>(null);
    const [errors, setErrors] = useState<Record<string, ValidationDetail[]>>({});
    const [busy, setBusy] = useState(false);

    const load = useCallback(async () => {
        const res = await api.instanceWords(server.id);
        if (failed(res) || !Array.isArray(res?.commands)) {
            toast(tg('discord.iw_load_failed', { error: errorText(t, res) }), 'error');
            return setInst(false);
        }
        setInst(res as InstanceWordsInfo);
        setDraft(draftOf(res as InstanceWordsInfo));
        setErrors({});
    }, [api, server.id, t, tg, toast]);

    useEffect(() => {
        setInst(null);
        load();
    }, [load]);

    if (inst === false) return null;
    if (!inst || !draft) return <Loading />;

    const dirty = isDraftDirty(draft, inst);
    const bad = (field: string) =>
        ((errors[field] || []) as WordError[]).map((d) => d.word).filter((w): w is string => !!w);
    const errorLine = (field: string) => {
        const list = errors[field] as WordError[] | undefined;
        if (!list?.length) return null;
        const text = (d: WordError) => {
            const vars = { word: d.word || '', other: d.conflictWith || '' };
            if (d.conflictWith) return tg('discord.iw_err_conflict', vars);
            const key = { too_many: 'too_many', reserved: 'reserved', unknown_command: 'unknown_command' }[d.reason as 'too_many' | 'reserved' | 'unknown_command'];
            return tg(`discord.iw_err_${key || 'invalid'}`, vars);
        };
        return <small className="hint hint-error">{list.map(text).join(' · ')}</small>;
    };

    const save = async () => {
        setBusy(true);
        const res = await api.instanceWordsSave(inst.id, draft);
        setBusy(false);
        if (res?.error_code === 'validation_failed') {
            const byField = errorsByField(res.details);
            setErrors(byField);
            if (byField.input) toast(tg('discord.iw_save_failed', { error: res.error || 'input' }), 'error');
            return;
        }
        if (failed(res)) return toast(tg('discord.iw_save_failed', { error: errorText(t, res) }), 'error');
        // 데몬은 저장한 {words, commands:{명령:[..]}}를 돌려준다 (빈 명령은 빠진다)
        const saved = Array.isArray(res?.words) ? { words: res.words, commands: res.commands || {} } : draft;
        const next = {
            ...inst,
            words: saved.words,
            commands: inst.commands.map((c) => ({ ...c, aliases: saved.commands[c.name] || [] })),
        };
        setInst(next);
        setDraft(draftOf(next));
        setErrors({});
        toast(tg('discord.iw_saved', { name: inst.name }), 'ok');
    };

    return (
        <Card icon="sabaTalking" tone="3" title={t('words.title')} desc={tg('discord.iw_desc')}>
            <div className="form">
                <div className="field">
                    <span>{tg('discord.iw_server_words')}</span>
                    <small className="hint">
                        {tg('discord.iw_default', { words: (inst.defaultWords || [inst.name]).join(', ') })}
                    </small>
                    <WordsInput
                        value={draft.words}
                        onChange={(words) => setDraft((d) => d && { ...d, words })}
                        label={tg('discord.iw_server_words_aria', { name: inst.name })}
                        placeholder={tg('discord.words_add_placeholder')}
                        invalid={bad('words')}
                    />
                    {errorLine('words')}
                </div>
                {inst.commands.length === 0 && <p className="hint">{tg('discord.no_commands')}</p>}
                {inst.commands.map((c) => {
                    const field = `commands.${c.name}`;
                    return (
                        <div className="field" key={c.name}>
                            <span>{c.label || c.name}</span>
                            <small className="hint">
                                {tg('discord.iw_default', { words: [c.name, ...(c.moduleAliases || [])].join(', ') })}
                            </small>
                            <WordsInput
                                value={draft.commands[c.name] || []}
                                onChange={(v) => setDraft((d) => d && { ...d, commands: { ...d.commands, [c.name]: v } })}
                                label={c.label || c.name}
                                placeholder={tg('discord.words_add_placeholder')}
                                invalid={bad(field)}
                            />
                            {errorLine(field)}
                        </div>
                    );
                })}
            </div>
            {dirty && (
                <div className="savebar">
                    <Button
                        disabled={busy}
                        onClick={() => {
                            setDraft(draftOf(inst));
                            setErrors({});
                        }}
                    >
                        {tg('discord.revert')}
                    </Button>
                    <Button tone="accent" busy={busy} onClick={save}>
                        {t('words.save')}
                    </Button>
                </div>
            )}
        </Card>
    );
}
