import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SaveBar } from './ChatbotView';
import { useFlip } from './flip';
import * as L from './levels';
import type { PermissionsState } from './PermissionsTab';
import { usePointerDrag } from './pointerDrag';
import type { DiscordNode } from './types';

const keyOf = (level: L.LadderLevel) => (level.id !== undefined ? `id-${level.id}` : level.key);
const isCustom = L.isCustomLevel;
const MAX_CHIPS = 12; // 칸마다 보이는 칩 수 (넘치면 +N)

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
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const line = L.controllerLine(board);
    // 명령 옮긴 차례 (맞바꾸기·줄 옮기기). 줄 자리는 이 차례에서 나온다
    const [steps, setSteps] = useState<L.CommandStep[]>([]);
    // 블록 끌기: key·칸 값은 사다리 자리(위에서부터 0)
    const blockDrag = usePointerDrag((from: string, to: string) =>
        setDraft((d) => L.moveLevel(d, Number(from), Number(to))),
    );
    const dragging = blockDrag.dragging === null ? null : Number(blockDrag.dragging);
    const over = blockDrag.over === null ? null : Number(blockDrag.over);
    const [newName, setNewName] = useState('');
    const [editing, setEditing] = useState<number | null>(null); // 이름을 고치는 칸
    const [choosing, setChoosing] = useState(false);
    const [picked, setPicked] = useState<number | null>(null);
    const listRef = useRef<HTMLOListElement>(null);
    const flip = useFlip(listRef);

    // 데몬 값이 바뀌면(저장·다시 읽기) 초안을 그 값으로
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    // 데몬 값을 다시 읽으면(저장 뒤) 맞바꾸기 초안을 비운다
    // biome-ignore lint/correctness/useExhaustiveDependencies: board 가 바뀔 때만 비운다
    useEffect(() => setSteps([]), [board]);

    const localeName = (level: L.LadderLevel) => t(`discord.level_${L.levelKey(level.id)}`);
    const label = (level: L.LadderLevel) => level.name?.trim() || (isCustom(level) ? '' : localeName(level));
    const counts: Record<number, number> = {};
    for (const l of Object.values(perm.members || {})) counts[l] = (counts[l] || 0) + 1;
    if (perm.ownerDiscordId) counts[L.OWNER] = 1;
    const patch = L.ladderPatch(draft);
    const ladderChanged = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const lineLevel = L.draftLine(line, draft, steps);
    const lineAt = line && lineLevel !== null ? L.lineAt(saved, draft, lineLevel) : null;
    const lineTarget = lineAt === null ? null : (draft[lineAt].id as number);
    const levels = L.draftLevels(board, draft, steps, line, lineTarget);
    const commandLevels = L.boardPatch(board, levels);
    const commandsChanged = Object.keys(commandLevels).length > 0;
    const changed = ladderChanged || commandsChanged;
    // 레벨 편집은 사바쨩 기능만 보인다 (사용자 지정 2026-11-07): 인스턴스마다 다른 게임(모듈) 명령과
    // 인스턴스마다 정하는 조작 가능(서버 조작)은 인스턴스 권한 화면에서 정한다. 맞바꾸기는 모든 명령을 그대로 옮긴다
    const chips = L.stairChips(L.draftBoard(board, levels)).filter((c) => !c.module && !c.controllers);
    const instanceName = (id?: string) => board?.instances?.find((i) => i.id === id)?.name || id;
    const chipLabel = (c: L.StairChip) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title || c.name);
    // 마지막 맞바꾸기 (두 칸이 아직 초안에 있을 때만 알린다)
    const lastSwap = L.activeSwaps(steps, draft).at(-1) ?? null;
    const levelName = (id: number) => {
        const l = draft.find((x) => x.id === id);
        return l ? label(l) : '';
    };
    const pick = (id: number) => {
        if (picked === null || picked === id) return setPicked(picked === id ? null : id);
        flip();
        setSteps((s) => [...s, [picked, id]]);
        setPicked(null);
        setChoosing(false);
    };
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const movable = (i: number) => i > 0 && i < draft.length - 1;
    const name = newName.trim();
    const save = () =>
        write(() =>
            window.api.permissionsUpdate(node.guildId, {
                ...(ladderChanged ? { ladder: patch } : {}),
                ...(commandsChanged ? { commandLevels } : {}),
            }),
        );
    const revert = () => {
        setDraft(saved);
        setSteps([]);
        setChoosing(false);
        setPicked(null);
    };

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
                <ol ref={listRef} className="dc-ladder" aria-label={t('discord.perm_edit_levels')}>
                    {draft.flatMap((level, i) => {
                        const name = label(level);
                        const fixed = !movable(i);
                        const pickable = choosing && L.swappable(level);
                        const own = L.swappable(level) ? chips.filter((c) => c.level === level.id) : [];
                        const block = (
                            <li
                                key={keyOf(level)}
                                aria-label={name}
                                className={clsx('dc-ladder-block', `level-${L.levelKey(level.id)}`, {
                                    'is-fixed': fixed,
                                    'is-dragging': dragging === i,
                                    'is-over': over === i && dragging !== i && !fixed,
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
                                        onClick={() => pick(level.id as number)}
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
                                                disabled={draft.length <= 3 || !L.removable(draft, i)}
                                                aria-label={t('discord.perm_ladder_delete', { name })}
                                                data-tooltip={t('discord.perm_ladder_delete', { name })}
                                                onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                            >
                                                <Icon name="trash" size="sm" />
                                            </button>
                                        </>
                                    )}
                                </span>
                                {L.swappable(level) && (
                                    <span className="dc-ladder-chips">
                                        {own.slice(0, MAX_CHIPS).map((c) => (
                                            <span
                                                key={c.id}
                                                data-flip={c.id}
                                                className={clsx('dc-ladder-chip', { 'is-fixed': c.fixed })}
                                            >
                                                {c.fixed && <Icon name="pin" size="xs" />}
                                                {chipLabel(c)}
                                            </span>
                                        ))}
                                        {own.length > MAX_CHIPS && (
                                            <span className="dc-ladder-chip is-more">
                                                {t('discord.perm_swap_more', { count: own.length - MAX_CHIPS })}
                                            </span>
                                        )}
                                        {own.length === 0 && (
                                            <span className="muted">{t('discord.perm_swap_none')}</span>
                                        )}
                                    </span>
                                )}
                            </li>
                        );
                        return [block];
                    })}
                </ol>
                <div className="dc-ladder-swapbar">
                    <button
                        type="button"
                        className={clsx('btn btn-sm', choosing ? 'btn-ghost' : 'btn-secondary')}
                        aria-pressed={choosing}
                        onClick={() => {
                            setChoosing((c) => !c);
                            setPicked(null);
                        }}
                    >
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
                <SaveBar dirty={changed} saving={busy} blocked={!valid} onRevert={revert} onSave={save} />
            </section>
        </div>
    );
}
