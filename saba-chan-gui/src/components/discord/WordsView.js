import clsx from 'clsx';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { checkAliasConflicts } from '../../utils/validation';
import { Icon } from '../Icon';
import { aliasDraftFor, commandAliasClashes } from './nodePermissions';
import { TagInput } from './TagInput';

const sameList = (a, b) => a.length === b.length && a.every((w, i) => w === b[i]);

function isDirty(draft, base) {
    if (!draft || !base) return false;
    if (!sameList(draft.moduleAliases, base.moduleAliases)) return true;
    return Object.keys(draft.commands).some(
        (c) => !sameList(draft.commands[c].aliases, base.commands[c]?.aliases || []),
    );
}

/**
 * 모듈별 디스코드 명령어 단어: `!saba <모듈 단어> <명령 단어>`의 두 칸을 모듈마다 고친다.
 */
export function WordsView({
    modules,
    moduleName,
    onSelectModule,
    prefix,
    moduleAliasesPerModule,
    savedModuleAliases,
    savedCommandAliases,
    onSave,
    onReset,
}) {
    const { t } = useTranslation('gui');
    const names = useMemo(() => {
        const list = modules.map((m) => m.name);
        for (const n of Object.keys(moduleAliasesPerModule || {})) if (!list.includes(n)) list.push(n);
        return list;
    }, [modules, moduleAliasesPerModule]);
    const current = names.includes(moduleName) ? moduleName : names[0];
    const base = useMemo(
        () =>
            current
                ? aliasDraftFor(current, moduleAliasesPerModule?.[current], savedModuleAliases, savedCommandAliases)
                : null,
        [current, moduleAliasesPerModule, savedModuleAliases, savedCommandAliases],
    );
    const [draft, setDraft] = useState(base);
    const [busy, setBusy] = useState(false);
    useEffect(() => setDraft(base), [base]);

    if (!current) {
        return (
            <div className="dc-view">
                <p className="muted pad">{t('discord.words_no_modules')}</p>
            </div>
        );
    }

    const mod = modules.find((m) => m.name === current);
    const dirty = isDirty(draft, base);
    const moduleConflicts = draft
        ? checkAliasConflicts(current, draft.moduleAliases, moduleAliasesPerModule, savedModuleAliases)
        : [];
    const clashes = draft ? commandAliasClashes(draft.commands) : [];
    const badCmdWords = clashes.map((c) => c.alias);
    const firstModuleWord = draft?.moduleAliases[0] || current;
    const commandEntries = Object.entries(draft?.commands || {});
    const firstCmd = commandEntries[0];
    const example =
        `${prefix || '!saba'} ${firstModuleWord} ${firstCmd ? firstCmd[1].aliases[0] || firstCmd[0] : ''}`.trim();

    const run = async (fn) => {
        setBusy(true);
        try {
            await fn();
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="dc-view">
            <header className="dc-view-head">
                <h1>{t('discord.words_title')}</h1>
                <p className="muted">{t('discord.words_desc')}</p>
            </header>

            <div className="dc-module-pills" role="tablist" aria-label={t('discord.words_title')}>
                {names.map((n) => {
                    const m = modules.find((x) => x.name === n);
                    return (
                        <button
                            key={n}
                            type="button"
                            role="tab"
                            aria-selected={n === current}
                            className={clsx('dc-pill', { active: n === current })}
                            onClick={() => {
                                if (busy || n === current) return;
                                if (dirty) {
                                    // 다른 모듈로 넘어가면 고친 내용이 사라지므로 먼저 저장한다 (겹치는 단어가 있으면 머문다)
                                    if (clashes.length > 0) return;
                                    run(() => onSave(current, draft)).then(() => onSelectModule(n));
                                    return;
                                }
                                onSelectModule(n);
                            }}
                        >
                            {m?.game_name || m?.display_name || n}
                        </button>
                    );
                })}
            </div>

            <div className="dc-example">
                <Icon name="messageSquare" size="sm" />
                <span className="muted">{t('discord.words_example')}</span>
                <code>{example}</code>
            </div>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('discord.words_module_title')}</h2>
                    <code className="dc-code-tag">{current}</code>
                </div>
                <p className="dc-card-desc">{t('discord.words_module_desc', { module: mod?.game_name || current })}</p>
                <TagInput
                    value={draft?.moduleAliases || []}
                    onChange={(v) => setDraft((d) => ({ ...d, moduleAliases: v }))}
                    placeholder={t('discord.words_add_placeholder')}
                    ariaLabel={t('discord.words_module_title')}
                    invalid={moduleConflicts.map((c) => c.alias)}
                />
                {moduleConflicts.length > 0 && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" />{' '}
                        {moduleConflicts
                            .map((c) =>
                                t('discord.words_conflict_module', { alias: c.alias, module: c.conflictModule }),
                            )
                            .join(' · ')}
                    </p>
                )}
            </section>

            <section className="dc-card">
                <div className="dc-card-head">
                    <h2>{t('discord.words_commands_title')}</h2>
                </div>
                <p className="dc-card-desc">{t('discord.words_commands_desc')}</p>
                {commandEntries.length === 0 ? (
                    <p className="muted">{t('discord.no_commands')}</p>
                ) : (
                    <div className="dc-word-table">
                        {commandEntries.map(([cmd, data]) => (
                            <div key={cmd} className="dc-word-row">
                                <div className="dc-word-cmd">
                                    <span className="dc-list-name">{data.label}</span>
                                    <code>{cmd}</code>
                                </div>
                                <TagInput
                                    value={data.aliases}
                                    onChange={(v) =>
                                        setDraft((d) => ({
                                            ...d,
                                            commands: { ...d.commands, [cmd]: { ...d.commands[cmd], aliases: v } },
                                        }))
                                    }
                                    placeholder={t('discord.words_add_placeholder')}
                                    ariaLabel={data.label}
                                    invalid={badCmdWords}
                                />
                            </div>
                        ))}
                    </div>
                )}
                {clashes.length > 0 && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" />{' '}
                        {clashes
                            .map((c) =>
                                t('discord.words_conflict_command', {
                                    alias: c.alias,
                                    a: c.commands[0],
                                    b: c.commands[1],
                                }),
                            )
                            .join(' · ')}
                    </p>
                )}
            </section>

            <div className={clsx('dc-savebar', { dirty })}>
                <button
                    type="button"
                    className="btn btn-ghost"
                    disabled={busy}
                    onClick={() => run(() => onReset(current))}
                    title={t('discord.words_reset_desc')}
                >
                    <Icon name="refresh" size="sm" /> {t('discord.words_reset')}
                </button>
                <span className="dc-savebar-text">{dirty ? t('discord.unsaved') : ''}</span>
                {dirty && (
                    <button type="button" className="btn btn-ghost" disabled={busy} onClick={() => setDraft(base)}>
                        {t('discord.revert')}
                    </button>
                )}
                <button
                    type="button"
                    className="btn btn-primary"
                    disabled={!dirty || busy || clashes.length > 0}
                    onClick={() => run(() => onSave(current, draft))}
                >
                    <Icon name="save" size="sm" /> {t('discord.save')}
                </button>
            </div>
        </div>
    );
}
