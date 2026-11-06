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

// ── 레벨 편집: 블록을 끌어 순서를 정한다 (위가 높은 레벨). 소유자·금지는 고정, 나머지는 옮긴다.
// 모든 레벨은 이름을 바꾸고(비우면 로케일 이름), 소유자·게스트·금지가 아니면 지운다 (사용자 지정 2026-11-06).
// 조작 가능 줄: 시작·정지·재시작·백업 만들기를 쓸 수 있는 가장 낮은 레벨 바로 아래에 긋고, 끌어서 옮긴다 ──
export function LadderTab({ node, permissions, onClose }: LadderTabProps) {
    const { t } = useTranslation('gui');
    const { busy, write, board } = permissions;
    const perm = permissions.perm as L.GuildPermissions;
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const line = L.controllerLine(board);
    const [lineLevel, setLineLevel] = useState(line?.level ?? null);
    // 블록 끌기: key·칸 값은 사다리 자리(위에서부터 0)
    const blockDrag = usePointerDrag((from: string, to: string) =>
        setDraft((d) => L.moveLevel(d, Number(from), Number(to))),
    );
    // 줄 끌기: 놓은 칸 바로 아래로 (금지·새 칸이면 그 위 칸)
    const lineDrag = usePointerDrag((_key: string, to: string) => {
        const at = L.lineDrop(draft, Number(to));
        setLineLevel(draft[at].id ?? null);
    });
    const dragging = blockDrag.dragging === null ? null : Number(blockDrag.dragging);
    const over = blockDrag.over === null ? null : Number(blockDrag.over);
    const lineOver = lineDrag.over === null ? null : L.lineDrop(draft, Number(lineDrag.over));
    const [newName, setNewName] = useState('');
    const [editing, setEditing] = useState<number | null>(null); // 이름을 고치는 칸

    // 데몬 값이 바뀌면(저장·다시 읽기) 초안을 그 값으로
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    useEffect(() => setLineLevel(line?.level ?? null), [line?.level]);

    const localeName = (level: L.LadderLevel) => t(`discord.level_${L.levelKey(level.id)}`);
    const label = (level: L.LadderLevel) => level.name?.trim() || (isCustom(level) ? '' : localeName(level));
    const counts: Record<number, number> = {};
    for (const l of Object.values(perm.members || {})) counts[l] = (counts[l] || 0) + 1;
    if (perm.ownerDiscordId) counts[L.OWNER] = 1;
    const patch = L.ladderPatch(draft);
    const ladderChanged = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const lineAt = line && lineLevel !== null ? L.lineAt(saved, draft, lineLevel) : null;
    const lineTarget = lineAt === null ? null : (draft[lineAt].id as number);
    const lineChanged = !!line && lineTarget !== null && lineTarget !== line.level;
    const changed = ladderChanged || lineChanged;
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const movable = (i: number) => i > 0 && i < draft.length - 1;
    const name = newName.trim();
    const moveLine = (dir: -1 | 1) => {
        if (lineAt === null) return;
        const to = L.lineStep(draft, lineAt, dir);
        if (to !== null) setLineLevel(draft[to].id ?? null);
    };
    const save = () =>
        write(() =>
            window.api.permissionsUpdate(node.guildId, {
                ...(ladderChanged ? { ladder: patch } : {}),
                ...(lineChanged && line ? { commandLevels: L.levelPatch(line.followers, lineTarget as number) } : {}),
            }),
        );
    const revert = () => {
        setDraft(saved);
        setLineLevel(line?.level ?? null);
    };

    const lineRow = (at: number) => {
        const target = draft[at];
        const text = t(at === 0 ? 'discord.perm_line_only' : 'discord.perm_line_from', { level: label(target) });
        return (
            <li
                key="controllers-line"
                aria-label={t('discord.perm_line')}
                className={clsx('dc-ladder-line', { 'is-dragging': lineDrag.dragging !== null })}
                {...lineDrag.bind('line')}
            >
                <span className="dc-ladder-rank" aria-hidden="true">
                    <Icon name="user" size="sm" />
                </span>
                <span className="dc-ladder-line-text">
                    <strong>{text}</strong>
                    <small>
                        {t('discord.perm_line_hint')}
                        {line && line.apart > 0 && ` ${t('discord.perm_line_apart', { count: line.apart })}`}
                    </small>
                </span>
                <span className="dc-mini-actions">
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={L.lineStep(draft, at, -1) === null}
                        aria-label={t('discord.perm_line_up')}
                        data-tooltip={t('discord.perm_line_up')}
                        onClick={() => moveLine(-1)}
                    >
                        <Icon name="chevronUp" size="sm" />
                    </button>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={L.lineStep(draft, at, 1) === null}
                        aria-label={t('discord.perm_line_down')}
                        data-tooltip={t('discord.perm_line_down')}
                        onClick={() => moveLine(1)}
                    >
                        <Icon name="chevronDown" size="sm" />
                    </button>
                </span>
            </li>
        );
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
                    {draft.flatMap((level, i) => {
                        const name = label(level);
                        const fixed = !movable(i);
                        const block = (
                            <li
                                key={keyOf(level)}
                                aria-label={name}
                                className={clsx('dc-ladder-block', `level-${L.levelKey(level.id)}`, {
                                    'is-fixed': fixed,
                                    'is-dragging': dragging === i,
                                    'is-over': over === i && dragging !== i && !fixed,
                                    'is-line-over': lineOver === i && lineOver !== lineAt,
                                })}
                                // 줄은 소유자·금지 칸에도 놓는다 (블록은 소유자·금지 칸으로 옮기지 않는다)
                                data-drop={i}
                                {...blockDrag.bind(String(i), !fixed)}
                            >
                                <span className="dc-ladder-rank">Lv {draft.length - 1 - i}</span>
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
                            </li>
                        );
                        return lineAt === i ? [block, lineRow(i)] : [block];
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
                <SaveBar dirty={changed} saving={busy} blocked={!valid} onRevert={revert} onSave={save} />
            </section>
        </div>
    );
}
