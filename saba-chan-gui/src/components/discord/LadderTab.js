import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import * as L from './levels';

const keyOf = (level) => (level.id !== undefined ? `id-${level.id}` : level.key);
const isCustom = L.isCustomLevel;

// ── 레벨 편집: 블록을 끌어 순서를 정한다 (위가 높은 레벨). 소유자·금지는 고정, 나머지는 옮기고 지울 수 있다 ──
export function LadderTab({ node, permissions, onClose }) {
    const { t } = useTranslation('gui');
    const { perm, busy, write } = permissions;
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const [dragging, setDragging] = useState(null);
    const [over, setOver] = useState(null);
    const [newName, setNewName] = useState('');

    // 데몬 값이 바뀌면(저장·다시 읽기) 초안을 그 값으로
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);

    const label = (level) => (isCustom(level) ? level.name : L.levelLabel(t, perm, level.id));
    const counts = {};
    for (const l of Object.values(perm.members || {})) counts[l] = (counts[l] || 0) + 1;
    if (perm.ownerDiscordId) counts[L.OWNER] = 1;
    const patch = L.ladderPatch(draft);
    const changed = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const movable = (i) => i > 0 && i < draft.length - 1;
    const name = newName.trim();

    const drop = (to) => {
        if (dragging !== null) setDraft((d) => L.moveLevel(d, dragging, to));
        setDragging(null);
        setOver(null);
    };

    return (
        <div className="dc-tab">
            <section className="dc-card">
                <div className="dc-card-head">
                    <h3>{t('discord.perm_ladder')}</h3>
                    {onClose && (
                        <button type="button" className="btn btn-ghost btn-sm" onClick={onClose}>
                            <Icon name="chevronLeft" size="sm" /> {t('discord.perm_ladder_done')}
                        </button>
                    )}
                </div>
                <p className="dc-hint">{t('discord.perm_ladder_hint')}</p>
                <ol className="dc-ladder" aria-label={t('discord.perm_ladder')}>
                    {draft.map((level, i) => {
                        const name = label(level);
                        const fixed = !movable(i);
                        return (
                            <li
                                key={keyOf(level)}
                                aria-label={name}
                                className={clsx('dc-ladder-block', `level-${L.levelKey(level.id)}`, {
                                    'is-fixed': fixed,
                                    'is-dragging': dragging === i,
                                    'is-over': over === i && dragging !== i,
                                })}
                                draggable={fixed ? undefined : 'true'}
                                onDragStart={(e) => {
                                    if (fixed) return;
                                    e.dataTransfer?.setData('text/plain', String(i));
                                    setDragging(i);
                                }}
                                onDragOver={(e) => {
                                    if (dragging === null || fixed) return;
                                    e.preventDefault();
                                    setOver(i);
                                }}
                                onDragLeave={() => setOver((o) => (o === i ? null : o))}
                                onDrop={(e) => {
                                    e.preventDefault();
                                    drop(i);
                                }}
                                onDragEnd={() => {
                                    setDragging(null);
                                    setOver(null);
                                }}
                            >
                                <span className="dc-ladder-rank">Lv {draft.length - 1 - i}</span>
                                {isCustom(level) ? (
                                    <input
                                        className="input dc-ladder-name"
                                        value={level.name}
                                        maxLength={L.MAX_NAME}
                                        aria-label={t('discord.perm_ladder_name')}
                                        onChange={(e) => setDraft((d) => L.renameLevel(d, i, e.target.value))}
                                    />
                                ) : (
                                    <span className="dc-ladder-label">{name}</span>
                                )}
                                {level.id !== undefined && counts[level.id] > 0 && (
                                    <span className="muted">
                                        {t('discord.perm_ladder_members', { count: counts[level.id] })}
                                    </span>
                                )}
                                {level.id === perm.defaultLevel && (
                                    <span className="dc-badge tone-info">{t('discord.perm_default_level')}</span>
                                )}
                                <span className="dc-mini-actions">
                                    {fixed ? (
                                        <span className="dc-badge">
                                            <Icon name="pin" size="xs" /> {t('discord.perm_owner_fixed')}
                                        </span>
                                    ) : (
                                        <>
                                            <button
                                                type="button"
                                                className="btn btn-ghost btn-sm"
                                                disabled={!movable(i - 1)}
                                                aria-label={t('discord.perm_ladder_up', { name })}
                                                data-tooltip={t('discord.perm_ladder_up', { name })}
                                                onClick={() => setDraft((d) => L.moveLevel(d, i, i - 1))}
                                            >
                                                <Icon name="chevronUp" size="sm" />
                                            </button>
                                            <button
                                                type="button"
                                                className="btn btn-ghost btn-sm"
                                                disabled={!movable(i + 1)}
                                                aria-label={t('discord.perm_ladder_down', { name })}
                                                data-tooltip={t('discord.perm_ladder_down', { name })}
                                                onClick={() => setDraft((d) => L.moveLevel(d, i, i + 1))}
                                            >
                                                <Icon name="chevronDown" size="sm" />
                                            </button>
                                            <button
                                                type="button"
                                                className="btn btn-ghost btn-sm"
                                                disabled={draft.length <= 3}
                                                aria-label={t('discord.perm_ladder_delete', { name })}
                                                data-tooltip={t('discord.perm_ladder_delete', { name })}
                                                onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                            >
                                                <Icon name="trash" size="sm" />
                                            </button>
                                        </>
                                    )}
                                </span>
                            </li>
                        );
                    })}
                </ol>
                <div className="dc-toolbar">
                    <input
                        className="input"
                        value={newName}
                        maxLength={L.MAX_NAME}
                        onChange={(e) => setNewName(e.target.value)}
                        placeholder={t('discord.perm_ladder_new')}
                        aria-label={t('discord.perm_ladder_new')}
                    />
                    <button
                        type="button"
                        className="btn btn-secondary"
                        disabled={!name || !L.canAddLevel(draft)}
                        onClick={() => {
                            setDraft((d) => L.addLevel(d, name));
                            setNewName('');
                        }}
                    >
                        <Icon name="plus" size="sm" /> {t('discord.perm_ladder_add')}
                    </button>
                </div>
                {removed && <p className="dc-hint">{t('discord.perm_ladder_removed')}</p>}
                {!valid && <p className="dc-field-error">{t('discord.perm_err_invalid_ladder')}</p>}
                <div className="dc-row-actions">
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={busy || !changed}
                        onClick={() => setDraft(saved)}
                    >
                        {t('discord.perm_ladder_revert')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-primary"
                        disabled={busy || !changed || !valid}
                        onClick={() => write(() => window.api.permissionsUpdate(node.guildId, { ladder: patch }))}
                    >
                        {t('discord.perm_ladder_save')}
                    </button>
                </div>
            </section>
        </div>
    );
}
