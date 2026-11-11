import clsx from 'clsx';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SaveBar } from '../ui/SaveBar';
import * as L from './levels';
import type { PermissionsState } from './PermissionsTab';
import type { DiscordNode } from './types';
import { ladderKeyOf as keyOf, useLadderDraft } from './useLadderDraft';

const isCustom = L.isCustomLevel;

interface LadderTabProps {
    node: DiscordNode;
    permissions: PermissionsState;
    onClose?: () => void;
}

// ── 레벨 편집: 블록을 끌어 순서를 정한다 (위가 높은 레벨). 소유자·금지는 고정, 나머지는 옮긴다.
// 모든 레벨은 이름을 바꾸고(비우면 로케일 이름), 소유자·게스트·금지가 아니면 지운다 (사용자 지정 2026-11-06).
// 조작 가능 줄: 시작·정지·재시작·백업 만들기를 쓸 수 있는 가장 낮은 레벨 바로 아래에 긋고, 끌어서 옮긴다.
// 권한 맞바꾸기 (사용자 지정 2026-11-07): 아래 버튼을 누르고 칸 두 개를 고르면 두 레벨의 명령 칩이 서로 자리를 바꾼다 ──
export function LadderTab({ node, permissions, onClose }: LadderTabProps) {
    const { t } = useTranslation('gui');
    const { busy, write, board } = permissions;
    const perm = permissions.perm as L.GuildPermissions;
    const ladder = useLadderDraft<HTMLOListElement>(perm, board, '.dc-ladder-chip');
    const { draft, setDraft, editing, setEditing, choosing, picked, blockDrag, chipDrag, dragging, over, chipOver } =
        ladder;
    const { chips, hidden, expanded, lastSwap, removed, valid, changed } = ladder;
    const [newName, setNewName] = useState('');

    const localeName = (level: L.LadderLevel) => t(`discord.level_${L.levelKey(level.id)}`);
    const label = (level: L.LadderLevel) => level.name?.trim() || (isCustom(level) ? '' : localeName(level));
    const counts: Record<number, number> = {};
    for (const l of Object.values(perm.members || {})) counts[l] = (counts[l] || 0) + 1;
    if (perm.ownerDiscordId) counts[L.OWNER] = 1;
    const levelName = (id: number) => {
        const l = draft.find((x) => x.id === id);
        return l ? label(l) : '';
    };
    const movable = (i: number) => i > 0 && i < draft.length - 1;
    const name = newName.trim();
    const save = () => write(() => window.api.permissionsUpdate(node.guildId, ladder.body()));

    return (
        <div className="dc-tab">
            <section className="dc-card">
                <div className="dc-card-head">
                    <h3>{t('discord.perm_edit_levels')}</h3>
                    {onClose && (
                        <button type="button" className="btn btn-ghost btn-sm" onClick={onClose}>
                            <Icon name="chevronLeft" size="sm" /> {t('discord.perm_ladder_done')}
                        </button>
                    )}
                </div>
                <p className="dc-hint">{t('discord.perm_ladder_hint')}</p>
                <ol ref={ladder.listRef} className="dc-ladder" aria-label={t('discord.perm_edit_levels')}>
                    {draft.flatMap((level, i) => {
                        const name = label(level);
                        const fixed = !movable(i);
                        const pickable = choosing && L.swappable(level);
                        const own = L.swappable(level) ? chips.filter((c) => c.level === level.id) : [];
                        const block = (
                            <li
                                key={keyOf(level)}
                                data-flip={`block:${String(keyOf(level))}`}
                                aria-label={name}
                                className={clsx('dc-ladder-block', `level-${L.levelKey(level.id)}`, {
                                    'is-fixed': fixed,
                                    'is-dragging': dragging === i,
                                    // 카드 끌기: 들어갈 자리(이 카드 위 또는 아래)에 선 (사용자 지정 2026-11-07)
                                    'is-insert-before': dragging !== null && over === i && i < dragging && !fixed,
                                    'is-insert-after': dragging !== null && over === i && i > dragging && !fixed,
                                    // 칩 끌기: 놓을 칸
                                    'is-over': chipDrag.dragging !== null && chipOver === i && L.swappable(level),
                                    'is-pickable': pickable,
                                    'is-picked': picked !== null && picked === level.id,
                                })}
                                // 줄은 소유자·금지 칸에도 놓는다 (블록은 소유자·금지 칸으로 옮기지 않는다)
                                data-drop={i}
                                {...blockDrag.bind(String(i), !fixed && !choosing)}
                            >
                                {pickable && (
                                    <button
                                        type="button"
                                        className="dc-ladder-pick"
                                        aria-pressed={picked === level.id}
                                        aria-label={t('discord.perm_swap_choose', { name })}
                                        onClick={() => ladder.pick(level.id as number)}
                                    />
                                )}
                                <span className="dc-ladder-rank">Lv {draft.length - 2 - i}</span>
                                {editing === i ? (
                                    <input
                                        className="input dc-ladder-name"
                                        // biome-ignore lint/a11y/noAutofocus: 이름을 누른 사람이 바로 고친다
                                        autoFocus
                                        value={level.name ?? ''}
                                        placeholder={isCustom(level) ? '' : localeName(level)}
                                        maxLength={L.MAX_NAME}
                                        aria-label={t('discord.perm_ladder_name')}
                                        onChange={(e) => setDraft((d) => L.renameLevel(d, i, e.target.value))}
                                        onBlur={() => setEditing(null)}
                                        onKeyDown={(e) => (e.key === 'Enter' || e.key === 'Escape') && setEditing(null)}
                                    />
                                ) : (
                                    // 이름 자리도 끌 수 있게 버튼이 아니라 span 이다 (끌지 않고 누르면 고친다)
                                    // biome-ignore lint/a11y/useSemanticElements: button 위에서는 끌기가 시작되지 않는다 (pointerDrag)
                                    <span
                                        role="button"
                                        tabIndex={0}
                                        className={clsx('dc-ladder-label', 'dc-ladder-rename', { muted: !name })}
                                        aria-label={t('discord.perm_ladder_rename', { name })}
                                        onClick={() => setEditing(i)}
                                        onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && setEditing(i)}
                                    >
                                        {name || t('discord.perm_ladder_name')}
                                        <Icon name="edit" size="xs" />
                                    </span>
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
                                        <button
                                            type="button"
                                            className="btn btn-ghost btn-sm"
                                            disabled={draft.length <= 3 || !L.removable(draft, i)}
                                            aria-label={t('discord.perm_ladder_delete', { name })}
                                            data-tooltip={t('discord.perm_ladder_delete', { name })}
                                            onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                        >
                                            <Icon name="trash" size="sm" />
                                        </button>
                                    )}
                                </span>
                                {L.swappable(level) && (
                                    <span
                                        className={clsx('dc-ladder-chips', { 'is-expanded': expanded[keyOf(level)] })}
                                        ref={ladder.chipBox(keyOf(level))}
                                    >
                                        {own.map((c) => (
                                            <button
                                                type="button"
                                                key={c.id}
                                                data-flip={c.id}
                                                aria-label={ladder.chipLabel(c)}
                                                className={clsx('dc-ladder-chip', {
                                                    'is-fixed': c.fixed,
                                                    'is-changed': ladder.unsaved(c),
                                                    'is-dragging': chipDrag.dragging === c.id,
                                                })}
                                                disabled={choosing}
                                                {...chipDrag.bind(c.id, !c.fixed && !choosing)}
                                            >
                                                {c.fixed && <Icon name="pin" size="xs" />}
                                                {ladder.chipLabel(c)}
                                            </button>
                                        ))}
                                        {own.length === 0 && (
                                            <span className="muted">{t('discord.perm_swap_none')}</span>
                                        )}
                                    </span>
                                )}
                                {L.swappable(level) && (hidden[keyOf(level)] || 0) > 0 && (
                                    <button
                                        type="button"
                                        className="dc-ladder-chip is-more"
                                        onClick={() => ladder.expand(keyOf(level))}
                                    >
                                        {t('discord.perm_swap_more', { count: hidden[keyOf(level)] })}
                                    </button>
                                )}
                            </li>
                        );
                        return [block];
                    })}
                </ol>
                <div className="dc-ladder-swapbar">
                    <button type="button" className="btn btn-sm btn-ghost" disabled={busy} onClick={ladder.resetToDefault}>
                        <Icon name="undo" size="sm" /> {t('discord.perm_ladder_reset')}
                    </button>
                    <button
                        type="button"
                        className={clsx('btn btn-sm', choosing ? 'btn-ghost' : 'btn-ghost')}
                        aria-pressed={choosing}
                        onClick={ladder.toggleChoosing}
                    >
                        <Icon name={choosing ? 'close' : 'swap'} size="sm" />{' '}
                        {choosing ? t('discord.perm_swap_cancel') : t('discord.perm_swap')}
                    </button>
                    <span className="dc-hint" aria-live="polite">
                        {choosing
                            ? picked === null
                                ? t('discord.perm_swap_pick')
                                : t('discord.perm_swap_pick_more', { name: levelName(picked) })
                            : lastSwap &&
                              t('discord.perm_swap_done', { a: levelName(lastSwap[0]), b: levelName(lastSwap[1]) })}
                    </span>
                </div>
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
                        className="btn btn-ghost"
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
                <SaveBar dirty={changed} saving={busy} blocked={!valid} onRevert={ladder.revert} onSave={save} />
            </section>
        </div>
    );
}
