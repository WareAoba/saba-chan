import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SaveBar } from './ChatbotView';
import * as L from './levels';
import type { PermissionsState } from './PermissionsTab';
import { usePointerDrag } from './pointerDrag';
import type { DiscordNode } from './types';

const keyOf = (level: L.LadderLevel) => (level.id !== undefined ? `id-${level.id}` : level.key);
const isCustom = L.isCustomLevel;

interface LadderTabProps {
    node: DiscordNode;
    permissions: PermissionsState;
    onClose?: () => void;
}

// ── 레벨 편집: 블록을 끌어 순서를 정한다 (위가 높은 레벨). 소유자·금지는 고정, 나머지는 옮기고 지울 수 있다 ──
export function LadderTab({ node, permissions, onClose }: LadderTabProps) {
    const { t } = useTranslation('gui');
    const { busy, write } = permissions;
    const perm = permissions.perm as L.GuildPermissions;
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    // 블록 끌기: key·칸 값은 사다리 자리(위에서부터 0)
    const blockDrag = usePointerDrag((from: string, to: string) =>
        setDraft((d) => L.moveLevel(d, Number(from), Number(to))),
    );
    const dragging = blockDrag.dragging === null ? null : Number(blockDrag.dragging);
    const over = blockDrag.over === null ? null : Number(blockDrag.over);
    const [newName, setNewName] = useState('');

    // 데몬 값이 바뀌면(저장·다시 읽기) 초안을 그 값으로
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);

    const label = (level: L.LadderLevel) => (isCustom(level) ? level.name : L.levelLabel(t, perm, level.id));
    const counts: Record<number, number> = {};
    for (const l of Object.values(perm.members || {})) counts[l] = (counts[l] || 0) + 1;
    if (perm.ownerDiscordId) counts[L.OWNER] = 1;
    const patch = L.ladderPatch(draft);
    const changed = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const movable = (i: number) => i > 0 && i < draft.length - 1;
    const name = newName.trim();

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
                                {...(fixed ? {} : { 'data-drop': i })}
                                {...blockDrag.bind(String(i), !fixed)}
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
                {/* 다른 설정 화면과 같은 화면 아래 저장 막대 (사용자 11-06 #9) */}
                <SaveBar
                    dirty={changed}
                    saving={busy}
                    blocked={!valid}
                    onRevert={() => setDraft(saved)}
                    onSave={() => write(() => window.api.permissionsUpdate(node.guildId, { ladder: patch }))}
                />
            </section>
        </div>
    );
}
