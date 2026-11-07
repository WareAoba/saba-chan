import * as L from '@gui/components/discord/levels';
import type { CommandBoard, GuildPermissions as PermData, LadderLevel } from '@gui/components/discord/levels';
import { useFlip } from '@gui/components/discord/flip';
import { usePointerDrag } from '@gui/components/discord/pointerDrag';
import * as P from '@gui/components/discord/nodePermissions';
import type { DiscordMember } from '@gui/components/discord/types';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { Dropdown } from './Dropdown';
import { ControllersPanel } from './ControllersPanel';
import { Icon } from './icons';
import { AccessBar, accessOf, KickButton, MembersKick } from './InstanceAccess';
import { MemberSearch } from './MemberSearch';
import { PermissionStairs, type PermissionWriter } from './PermissionStairs';
import { Avatar, Button, Card, Empty, errorText, failed, Loading, Segments } from './ui';
import type { Server, TFn, WebApi } from '../types';

/** 켜짐 알림·보일 서버 한 벌 (GET /api/guilds/:g/start-notice). null = 읽는 중, false = 읽지 못함 */
type NoticeState = { guildId?: string; instances?: string[]; allowedInstances?: string[] | null } | false | null;

/** 명령 계단이 알려 주는 인스턴스 한 대 */
interface BoardInstance {
    id: string;
    name: string;
    module?: string;
}

/** 레벨 이름: 길드에 더한 단계는 그 이름, 기본 단계는 웹 문구 */
const levelName = (t: TFn, perm: PermData | null | undefined, id: number | undefined) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);

const nameOf = (m: DiscordMember) => m.displayName || m.globalName || m.username || String(m.id);

interface LevelSelectProps {
    value: number | null;
    perm: PermData;
    onChange: (level: number | null) => void;
    withDefault?: boolean;
    t: TFn;
    disabled: boolean;
    label: string;
}

function LevelSelect({ value, perm, onChange, withDefault, t, disabled, label }: LevelSelectProps) {
    return (
        <Dropdown
            className="level-select"
            value={value === null ? 'default' : String(value)}
            disabled={disabled}
            ariaLabel={label}
            onChange={(v) => onChange(v === 'default' ? null : Number(v))}
            options={[
                ...(withDefault ? [{ value: 'default', label: t('perm.use_default') }] : []),
                ...L.grantable(perm).map((l) => ({ value: String(l), label: levelName(t, perm, l) })),
            ]}
        />
    );
}

/** 레벨 편집 (위가 높은 레벨, 앱 LadderTab 과 같은 규칙, 사용자 지정 2026-11-07). 카드를 끌어 순서를 바꾸고(들어갈 자리에 선),
 * 이름을 누르면 고친다. 소유자·금지는 고정이고 소유자·게스트·금지는 지울 수 없다. 칸에는 사바쨩 기능 칩만 보이고(게임 명령·조작 가능은
 * 인스턴스 권한 화면에서 정한다) 칩을 끌어 다른 레벨로 옮긴다. 노란 테두리는 저장하지 않은 변경이다 */
interface LadderEditorProps {
    perm: PermData;
    board: CommandBoard | null;
    busy: boolean;
    onSave: (body: {
        ladder?: ReturnType<typeof L.ladderPatch>;
        commandLevels?: Record<string, number | null>;
    }) => void;
}

const keyOf = (l: LadderLevel) => String(l.id !== undefined ? `id-${l.id}` : l.key);

export function LadderEditor({ perm, board, busy, onSave }: LadderEditorProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const [name, setName] = useState('');
    const [editing, setEditing] = useState<number | null>(null); // 이름을 고치는 칸
    const line = L.controllerLine(board);
    // 권한 맞바꾸기: 아래 버튼을 누르고 칸 두 개를 고르면 두 레벨의 명령이 자리를 바꾼다. 명령 옮긴 차례
    const [steps, setSteps] = useState<L.CommandStep[]>([]);
    const [choosing, setChoosing] = useState(false);
    const [picked, setPicked] = useState<number | null>(null);
    // 칩 끌기로 옮긴 명령: 명령 키 → 레벨 (맞바꾸기 뒤에 덮는다)
    const [moves, setMoves] = useState<Record<string, number>>({});
    const listRef = useRef<HTMLUListElement>(null);
    const flip = useFlip(listRef);
    // 카드 끌기: key·칸 값은 사다리 자리(위에서부터 0). 놓으면 카드들이 새 자리로 미끄러진다
    const blockDrag = usePointerDrag((from: string, to: string) => {
        flip();
        setDraft((d) => L.moveLevel(d, Number(from), Number(to)));
    });
    const chipDropRef = useRef<(id: string, at: number) => void>(() => {});
    const chipDrag = usePointerDrag((id: string, to: string) => chipDropRef.current(id, Number(to)));
    const dragging = blockDrag.dragging === null ? null : Number(blockDrag.dragging);
    const over = blockDrag.over === null ? null : Number(blockDrag.over);
    const chipOver = chipDrag.over === null ? null : Number(chipDrag.over);
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    // 데몬 값을 다시 읽으면(저장 뒤) 초안을 비운다
    // biome-ignore lint/correctness/useExhaustiveDependencies: board 가 바뀔 때만 비운다
    useEffect(() => {
        setSteps([]);
        setMoves({});
    }, [board]);
    const patch = L.ladderPatch(draft);
    const ladderChanged = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const lineLevel = L.draftLine(line, draft, steps);
    const lineAt = line && lineLevel !== null ? L.lineAt(saved, draft, lineLevel) : null;
    const lineTarget = lineAt === null ? null : (draft[lineAt].id as number);
    const items = board?.items || [];
    const levels = L.draftLevels(board, draft, steps, line, lineTarget).map((level, i) =>
        items[i] && !items[i].fixed && moves[items[i].key as string] !== undefined
            ? moves[items[i].key as string]
            : level,
    );
    const commandLevels = L.boardPatch(board, levels);
    const commandsChanged = Object.keys(commandLevels).length > 0;
    const changed = ladderChanged || commandsChanged;
    // 사바쨩 기능만 보인다: 게임(모듈) 명령과 조작 가능(서버 조작)은 인스턴스 권한 화면에서 정한다. 맞바꾸기는 모든 명령을 옮긴다
    const chips = L.stairChips(L.draftBoard(board, levels)).filter((c) => !c.module && !c.controllers);
    const savedLevel = new Map(items.map((it) => [it.key as string, it.level]));
    const unsaved = (c: L.StairChip) => c.items.some((it) => savedLevel.get(it.key as string) !== it.level);
    chipDropRef.current = (id, at) => {
        const chip = chips.find((c) => c.id === id);
        const target = draft[at];
        if (!chip || chip.fixed || !target || !L.swappable(target)) return;
        const level = target.id as number;
        flip();
        setMoves((m) => ({
            ...m,
            ...Object.fromEntries(chip.items.filter((i) => !i.fixed).map((i) => [i.key as string, level])),
        }));
    };
    // 칩은 숨기지 않는다. 카드가 칩 세 줄을 넘을 때만 접고 "+N"을 누르면 그 카드의 칩을 모두 편다
    const chipBoxes = useRef(new Map<string, HTMLElement>());
    const [hidden, setHidden] = useState<Record<string, number>>({});
    const [expanded, setExpanded] = useState<Record<string, boolean>>({});
    useLayoutEffect(() => {
        // 끄는 동안은 세지 않는다: 손을 따라 나간 칩을 접힌 칩으로 세지 않게
        if (chipDrag.dragging !== null) return;
        const next: Record<string, number> = {};
        for (const [k, box] of chipBoxes.current) {
            if (expanded[k]) continue;
            const top = box.getBoundingClientRect().top;
            const limit = box.clientHeight;
            const n = [...box.querySelectorAll<HTMLElement>('.ladder-chip')].filter(
                (el) => el.getBoundingClientRect().top - top >= limit,
            ).length;
            if (n > 0) next[k] = n;
        }
        if (JSON.stringify(next) !== JSON.stringify(hidden)) setHidden(next);
    });
    const instanceName = (id?: string) => board?.instances?.find((i) => i.id === id)?.name || id;
    const chipLabel = (c: L.StairChip) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title || c.name);
    // 마지막 맞바꾸기 (두 칸이 아직 초안에 있을 때만 알린다)
    const lastSwap = L.activeSwaps(steps, draft).at(-1) ?? null;
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const localeName = (l: LadderLevel) => levelName(t, null, l.id);
    const label = (l: LadderLevel) => l.name?.trim() || (L.isCustomLevel(l) ? '' : localeName(l));
    const last = draft.length - 1;
    const nameById = (id: number) => {
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
    const save = () =>
        onSave({
            ...(ladderChanged ? { ladder: patch } : {}),
            ...(commandsChanged ? { commandLevels } : {}),
        });
    const revert = () => {
        setDraft(saved);
        setSteps([]);
        setMoves({});
        setChoosing(false);
        setPicked(null);
    };
    // 기본값으로 초기화: 기본 사다리와 프리셋 레벨 (초안, 저장해야 반영)
    const resetToDefault = () => {
        flip();
        setDraft(L.ladderOf(null));
        setSteps([]);
        setChoosing(false);
        setPicked(null);
        setMoves(Object.fromEntries(items.filter((it) => !it.fixed).map((it) => [it.key as string, it.preset as number])));
    };
    return (
        <div className="field">
            <span>{tg('discord.perm_edit_levels')}</span>
            <small className="hint">{tg('discord.perm_ladder_hint')}</small>
            <ul ref={listRef} className="rows ladder" aria-label={tg('discord.perm_edit_levels')}>
                {draft.map((l, i) => {
                    const fixed = i === 0 || i === last;
                    const k = keyOf(l);
                    const pickable = choosing && L.swappable(l);
                    const own = L.swappable(l) ? chips.filter((c) => c.level === l.id) : [];
                    const classes = [
                        'row ladder-row',
                        fixed && 'is-fixed',
                        dragging === i && 'is-dragging',
                        dragging !== null && over === i && i < dragging && !fixed && 'is-insert-before',
                        dragging !== null && over === i && i > dragging && !fixed && 'is-insert-after',
                        chipDrag.dragging !== null && chipOver === i && L.swappable(l) && 'is-over',
                        pickable && 'is-pickable',
                        picked !== null && picked === l.id && 'is-picked',
                    ];
                    return (
                        <li
                            key={k}
                            data-flip={`block:${k}`}
                            data-drop={i}
                            className={classes.filter(Boolean).join(' ')}
                            aria-label={label(l)}
                            {...blockDrag.bind(String(i), !fixed && !choosing)}
                        >
                            {pickable && (
                                <button
                                    type="button"
                                    className="ladder-pick"
                                    aria-pressed={picked === l.id}
                                    aria-label={tg('discord.perm_swap_choose', { name: label(l) })}
                                    onClick={() => pick(l.id as number)}
                                />
                            )}
                            <div className="row-main">
                                <div className="ladder-head">
                                    {/* 휴대폰은 이 손잡이를 끌어 카드를 옮긴다 (나머지 자리는 화면 스크롤) */}
                                    {!fixed && (
                                        <span className="ladder-grip" aria-hidden="true">
                                            <Icon name="grip" size={16} />
                                        </span>
                                    )}
                                    <span className="ladder-rank">Lv {draft.length - 2 - i}</span>
                                    {editing === i ? (
                                        <input
                                            // biome-ignore lint/a11y/noAutofocus: 이름을 누른 사람이 바로 고친다
                                            autoFocus
                                            value={l.name ?? ''}
                                            placeholder={L.isCustomLevel(l) ? '' : localeName(l)}
                                            maxLength={L.MAX_NAME}
                                            aria-label={t('perm.ladder_name')}
                                            onChange={(e) => setDraft((d) => L.renameLevel(d, i, e.target.value))}
                                            onBlur={() => setEditing(null)}
                                            onKeyDown={(e) =>
                                                (e.key === 'Enter' || e.key === 'Escape') && setEditing(null)
                                            }
                                        />
                                    ) : (
                                        // 이름 자리도 끌 수 있게 버튼이 아니라 span 이다 (끌지 않고 누르면 고친다)
                                        // biome-ignore lint/a11y/useSemanticElements: button 위에서는 끌기가 시작되지 않는다 (pointerDrag)
                                        <span
                                            role="button"
                                            tabIndex={0}
                                            className={`ladder-name${label(l) ? '' : ' hint'}`}
                                            aria-label={tg('discord.perm_ladder_rename', { name: label(l) })}
                                            onClick={() => setEditing(i)}
                                            onKeyDown={(e) => (e.key === 'Enter' || e.key === ' ') && setEditing(i)}
                                        >
                                            {label(l) || t('perm.ladder_name')} <Icon name="edit" size={12} />
                                        </span>
                                    )}
                                    {fixed ? (
                                        <span className="pill">{t('perm.ladder_fixed')}</span>
                                    ) : (
                                        <Button
                                            tone="ghost-danger"
                                            disabled={draft.length <= 3 || !L.removable(draft, i)}
                                            aria-label={t('perm.ladder_delete', { name: label(l) })}
                                            onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                        >
                                            <Icon name="trash" size={16} />
                                        </Button>
                                    )}
                                </div>
                                {L.swappable(l) && (
                                    <div
                                        className={`ladder-chips${expanded[k] ? ' is-expanded' : ''}`}
                                        ref={(el) => {
                                            if (el) chipBoxes.current.set(k, el);
                                            else chipBoxes.current.delete(k);
                                        }}
                                    >
                                        {own.map((c) => (
                                            <button
                                                type="button"
                                                key={c.id}
                                                data-flip={c.id}
                                                aria-label={chipLabel(c)}
                                                className={`pill ladder-chip${c.fixed ? ' is-fixed' : ''}${
                                                    unsaved(c) ? ' is-changed' : ''
                                                }${chipDrag.dragging === c.id ? ' is-dragging' : ''}`}
                                                disabled={choosing}
                                                {...chipDrag.bind(c.id, !c.fixed && !choosing)}
                                            >
                                                {chipLabel(c)}
                                            </button>
                                        ))}
                                        {own.length === 0 && (
                                            <span className="row-sub">{tg('discord.perm_swap_none')}</span>
                                        )}
                                    </div>
                                )}
                                {L.swappable(l) && (hidden[k] || 0) > 0 && (
                                    <button
                                        type="button"
                                        className="pill ladder-more"
                                        onClick={() => setExpanded((x) => ({ ...x, [k]: true }))}
                                    >
                                        {tg('discord.perm_swap_more', { count: hidden[k] })}
                                    </button>
                                )}
                            </div>
                        </li>
                    );
                })}
            </ul>
            <div className="field-row">
                <Button disabled={busy} onClick={resetToDefault}>
                    <Icon name="undo" size={16} /> {tg('discord.perm_ladder_reset')}
                </Button>
                <Button
                    aria-pressed={choosing}
                    onClick={() => {
                        setChoosing((c) => !c);
                        setPicked(null);
                    }}
                >
                    <Icon name={choosing ? 'close' : 'swap'} size={16} />{' '}
                    {choosing ? tg('discord.perm_swap_cancel') : tg('discord.perm_swap')}
                </Button>
                <small className="hint" aria-live="polite">
                    {choosing
                        ? picked === null
                            ? tg('discord.perm_swap_pick')
                            : tg('discord.perm_swap_pick_more', { name: nameById(picked) })
                        : lastSwap &&
                          tg('discord.perm_swap_done', { a: nameById(lastSwap[0]), b: nameById(lastSwap[1]) })}
                </small>
            </div>
            <div className="field-row">
                <input
                    value={name}
                    maxLength={L.MAX_NAME}
                    placeholder={t('perm.ladder_new')}
                    aria-label={t('perm.ladder_new')}
                    onChange={(e) => setName(e.target.value)}
                />
                <Button
                    disabled={!name.trim() || !L.canAddLevel(draft)}
                    onClick={() => {
                        setDraft((d) => L.addLevel(d, name));
                        setName('');
                    }}
                >
                    {t('perm.ladder_add')}
                </Button>
            </div>
            {removed && <small className="hint">{t('perm.ladder_removed')}</small>}
            {!valid && <small className="hint">{t('perm.ladder_invalid')}</small>}
            <div className="field-row">
                <Button disabled={busy || !changed} onClick={revert}>
                    {t('perm.ladder_revert')}
                </Button>
                <Button tone="accent" busy={busy} disabled={!changed || !valid} onClick={save}>
                    {t('perm.ladder_save')}
                </Button>
            </div>
        </div>
    );
}

/** 켜짐 알림 줄: 이 디스코드 서버가 쓰기로 한 서버만 (allowedInstances 가 null 이면 전부), 켠 것은 on */
export function noticeRows(servers: Pick<Server, 'id' | 'name'>[], notice: NoticeState) {
    if (!notice) return [];
    const on = new Set(Array.isArray(notice.instances) ? notice.instances : []);
    const allowed = Array.isArray(notice.allowedInstances) ? new Set(notice.allowedInstances) : null;
    return servers
        .filter((s) => !allowed || allowed.has(s.id))
        .map((s) => ({ id: s.id, name: s.name, on: on.has(s.id) }));
}

/**
 * 이 디스코드 서버에서 보일 서버 (2026-11-03 사용자 지정 시안, GUI 노드의 인스턴스 탭과 같은 규칙):
 * 모두 보이기(제한 없음, 새 서버도 바로 보인다) 또는 고른 것만. 서버 하나는 디스코드 서버 하나만 고를 수 있다.
 * `allowed`: 이 길드의 allowedInstances (null 이면 제한 없음), `others`: 이 PC 의 다른 길드 {guildId: allowedInstances}
 */
interface VisibleServersProps {
    api: WebApi;
    guildId: string;
    allowed: string[] | null | undefined;
    others: Record<string, string[]> | null;
    onSaved: (allowedInstances: string[] | null) => void;
}

export function VisibleServers({ api, guildId, allowed, others, onSaved }: VisibleServersProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const servers = useData((s) => s.servers);
    const [busy, setBusy] = useState(false);
    const settings: P.NodeSettings = {
        ...Object.fromEntries(Object.entries(others || {}).map(([g, list]) => [g, { allowedInstances: list }])),
    };
    if (Array.isArray(allowed)) settings[guildId] = { allowedInstances: allowed };
    const nodeIds = [guildId, ...Object.keys(others || {})];
    const mode = P.instanceMode(settings, guildId);
    const ownerOf = (id: string) => P.instanceOwner(settings, nodeIds, id, guildId);

    const save = async (next: P.NodeSettings) => {
        const list = P.nodeConfig(next, guildId).allowedInstances;
        setBusy(true);
        const res = await api.guildAllowedInstancesSave(guildId, Array.isArray(list) ? list : null);
        setBusy(false);
        if (failed(res)) return toast(tg('discord.allowed_save_failed', { error: errorText(t, res) }), 'error');
        onSaved(res.allowedInstances ?? null);
        toast(t('perm.saved'), 'ok');
    };
    const visible = servers.filter((s) => P.isInstanceAllowed(settings, guildId, s.id) && !ownerOf(s.id)).length;
    return (
        <Card
            icon="discord"
            tone="7"
            title={t('perm.visible')}
            desc={t('perm.visible_hint')}
            action={<small className="pill">{`${visible}/${servers.length}`}</small>}
        >
            <ul className="rows">
                <li className="row">
                    <div className="row-main">
                        <div className="row-title">{tg('discord.instances_all')}</div>
                        <div className="row-sub">{tg('discord.instances_all_desc')}</div>
                    </div>
                    <label className="toggle">
                        <input
                            type="checkbox"
                            className="switch"
                            aria-label={tg('discord.instances_all')}
                            checked={mode === 'all'}
                            disabled={busy}
                            onChange={(ev) =>
                                save(
                                    P.setInstanceMode(
                                        settings,
                                        guildId,
                                        ev.target.checked ? 'all' : 'pick',
                                        servers.map((x) => x.id),
                                        nodeIds,
                                    ),
                                )
                            }
                        />
                    </label>
                </li>
                {servers.map((srv) => {
                    const owner = ownerOf(srv.id);
                    return (
                        <li key={srv.id} className="row">
                            <div className="row-main">
                                <div className="row-title row-title-one">{srv.name}</div>
                                {owner && <div className="row-sub">{t('perm.visible_owned')}</div>}
                            </div>
                            <label className="toggle">
                                <input
                                    type="checkbox"
                                    className="switch"
                                    aria-label={srv.name}
                                    checked={P.isInstanceAllowed(settings, guildId, srv.id) && !owner}
                                    disabled={busy || mode === 'all' || !!owner}
                                    onChange={() => save(P.toggleInstance(settings, guildId, srv.id, nodeIds))}
                                />
                            </label>
                        </li>
                    );
                })}
            </ul>
        </Card>
    );
}

/**
 * 서버별 알림 목록 (서버마다, 기본 꺼짐): 켜짐·꺼짐 알림(`start`)과 자리 꽉 참 알림(`full`).
 * 켠 서버만 이 디스코드 서버의 알림 채널에 알린다
 */
interface StartNoticeProps {
    api: WebApi;
    guildId: string;
    notice: NoticeState;
    setNotice: React.Dispatch<React.SetStateAction<NoticeState>>;
    kind?: 'start' | 'full';
}

function StartNotice({ api, guildId, notice, setNotice, kind = 'start' }: StartNoticeProps) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const servers = useData((s) => s.servers);
    const serversLoaded = useData((s) => s.serversLoaded);
    const [busy, setBusy] = useState(false);

    // 한 번에 하나씩: 응답의 목록이 그때의 전체라 겹치면 늦게 온 것이 앞 변경을 덮는다
    const toggle = async (id: string, enabled: boolean) => {
        setBusy(true);
        const save = kind === 'full' ? api.guildFullNoticeSave : api.guildStartNoticeSave;
        const res = await save(id, guildId, enabled);
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        setNotice((n) => (n ? { ...n, instances: res.instances } : n));
        toast(enabled ? t(`perm.${prefix}_on`) : t(`perm.${prefix}_off`), 'ok');
    };
    const prefix = kind === 'full' ? 'full_notice' : 'notice';

    const rows = noticeRows(servers, notice);
    return (
        <Card
            icon={kind === 'full' ? 'user' : 'bell'}
            tone={kind === 'full' ? '4' : '3'}
            title={t(`perm.${prefix}`)}
            desc={t(`perm.${prefix}_hint`)}
        >
            {notice === false ? null : !notice || !serversLoaded ? (
                <Loading />
            ) : rows.length === 0 ? (
                <Empty>{t('perm.notice_empty')}</Empty>
            ) : (
                <ul className="rows">
                    {rows.map((r) => (
                        <li key={r.id} className="row">
                            <div className="row-main">
                                <div className="row-title row-title-one">{r.name}</div>
                            </div>
                            <label className="toggle">
                                <input
                                    type="checkbox"
                                    className="switch"
                                    aria-label={r.name}
                                    checked={r.on}
                                    disabled={busy}
                                    onChange={(ev) => toggle(r.id, ev.target.checked)}
                                />
                            </label>
                        </li>
                    ))}
                </ul>
            )}
        </Card>
    );
}

/** 길드 하나의 권한과 명령 계단. 늦게 온 앞선 읽기는 버린다 */
function useGuildPermissions(api: WebApi, guildId: string) {
    const { t, i18n } = useTranslation();
    const locale = i18n.language;
    const toast = useApp((s) => s.toast);
    const [perm, setPerm] = useState<PermData | null>(null);
    const [board, setBoard] = useState<CommandBoard | null>(null);
    const [members, setMembers] = useState<DiscordMember[] | null>(null);
    const [busy, setBusy] = useState(false);
    const seq = useRef(0);

    /** 권한과 계단. 디스코드 멤버 목록은 느려서(최대 20초) 처음 열 때만 받는다 */
    const load = useCallback(async () => {
        const mine = ++seq.current;
        const [p, b] = await Promise.all([api.permissionsGet(guildId), api.permissionsCommands(guildId, locale)]);
        if (mine !== seq.current) return;
        if (failed(p)) toast(errorText(t, p), 'error');
        else setPerm(p);
        setBoard(failed(b) ? null : b);
    }, [api, guildId, locale, t, toast]);

    useEffect(() => {
        setPerm(null);
        setBoard(null);
        load();
    }, [load]);

    // 새로고침을 누르면 다시 받는다 (늦게 온 옛 응답은 버린다)
    const [membersGen, setMembersGen] = useState(0);
    // biome-ignore lint/correctness/useExhaustiveDependencies: membersGen 이 바뀌면 다시 받는다
    useEffect(() => {
        let live = true;
        setMembers(null);
        api.relayListNodeMembers(guildId, '').then((m) => live && setMembers(Array.isArray(m?.members) ? m.members : []));
        return () => {
            live = false;
        };
    }, [api, guildId, membersGen]);
    /** 권한과 멤버 목록을 다시 읽는다 (멤버 탭 새로고침) */
    const reload = () => {
        load();
        setMembersGen((n) => n + 1);
    };

    const write: PermissionWriter = async (fn) => {
        setBusy(true);
        const res = await fn();
        if (failed(res)) toast(errorText(t, res), 'error');
        else toast(t('perm.saved'), 'ok');
        await load();
        setBusy(false);
    };
    return { perm, board, members, busy, write, reload };
}

/** 멤버별 레벨 (앱 `PeopleView`와 같다, 사용자 지정 2026-11-07): 레벨 칸마다 사람 칩. 윗줄은 기본 레벨 · 멤버 찾기 · 새로고침.
 * 칩을 끌어 다른 레벨 칸에 놓거나 눌러 레벨을 고른다. 찾기 결과도 레벨 칸으로 끈다. 사람 추가는 닉네임으로 찾는다 (길드 전체에 적용) */
interface MemberLevelsProps {
    api: WebApi;
    guildId: string;
    perm: PermData;
    board: CommandBoard | null;
    members: DiscordMember[];
    busy: boolean;
    write: PermissionWriter;
    reload: () => void;
}

/** 레벨마다 그 레벨에서 새로 쓸 수 있게 되는 명령 수 (큰 칩만, help·list 제외) — 앱 `levelSummary`와 같다 */
function levelSummary(tg: TFn, perm: PermData, board: CommandBoard | null, id: number) {
    if (id === L.OWNER) return tg('discord.perm_level_all');
    if (id === L.BANNED) return tg('discord.perm_level_none');
    if (!board) return null;
    const up = L.ascending(perm);
    const below = up[up.indexOf(id) - 1];
    const gain = L.stairChips(board).filter((c) => !c.instance && c.level === id).length;
    if (below === L.BANNED) return tg('discord.perm_level_base', { count: gain + 1 });
    return tg('discord.perm_level_gain', { below: L.levelTitle(tg, perm, below), count: gain });
}

function MemberLevels({ api, guildId, perm, board, members, busy, write, reload }: MemberLevelsProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [open, setOpen] = useState<string | null>(null); // 레벨을 고치는 사람 id
    const [adding, setAdding] = useState<number | null>(null); // 사람을 더하는 레벨
    const [showDefault, setShowDefault] = useState(false);
    // 사람 칩 끌기: key는 `id|레벨`. 놓으면 dropRef가 옮긴다
    const dropRef = useRef<(id: string, from: number, to: number) => void>(() => {});
    const peopleDrag = usePointerDrag((key: string, to: string) => {
        const [id, from] = key.split('|');
        dropRef.current(id, Number(from), Number(to));
    });
    const dragging = peopleDrag.dragging
        ? { id: peopleDrag.dragging.split('|')[0], level: Number(peopleDrag.dragging.split('|')[1]) }
        : null;
    const over = peopleDrag.over === null ? null : Number(peopleDrag.over);
    const byId = useMemo(() => new Map(members.map((m) => [String(m.id), m])), [members]);
    const memberName = useCallback((id: string) => {
        const m = byId.get(id);
        return m ? nameOf(m) : id;
    }, [byId]);
    const avatarOf = (id: string) => byId.get(id)?.avatarUrl || byId.get(id)?.avatar;
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const owner = perm.ownerDiscordId ? String(perm.ownerDiscordId) : null;
    const setMember = (id: string, level: number) => write(() => api.permissionsSetMember(guildId, id, level));
    // 소유자 레벨과 원래 레벨에는 놓아도 바꾸지 않는다
    dropRef.current = (id, from, to) => {
        if (to !== L.OWNER && to !== from && !busy) setMember(id, to);
    };
    const groups = new Map(L.membersByLevel(perm).map((g) => [g.level, g.ids]));
    const others = L.sortByName(L.defaultMembers(perm, members), (m) => memberName(String(m.id)));
    // 이 레벨에 더할 수 있는 디스코드 멤버 (소유자와 이미 이 레벨인 사람 제외)
    const addable = (level: number) =>
        members.filter((m) => {
            const id = String(m.id);
            return id !== owner && (perm.members?.[id] ?? defaultLevel) !== level;
        });

    const chip = (id: string, level: number) => (
        <li key={id} className="person">
            <button
                type="button"
                className={`person-chip${open === id ? ' is-open' : ''}${dragging?.id === id ? ' is-dragging' : ''}`}
                aria-expanded={open === id}
                aria-label={memberName(id)}
                {...peopleDrag.bind(`${id}|${level}`)}
                onClick={() => setOpen(open === id ? null : id)}
            >
                <Avatar url={avatarOf(id)} name={memberName(id)} className="avatar avatar-chip" />
                <span>{memberName(id)}</span>
            </button>
            {open === id && (
                <span className="person-edit">
                    <LevelSelect
                        value={level}
                        perm={perm}
                        label={tg('discord.perm_member_level', { name: memberName(id) })}
                        t={t}
                        disabled={busy}
                        onChange={(l) => {
                            if (l !== null && l !== level) {
                                setOpen(null);
                                setMember(id, l);
                            }
                        }}
                    />
                    {perm.members?.[id] !== undefined && (
                        <Button
                            disabled={busy}
                            onClick={() => write(() => api.permissionsRemoveMember(guildId, id))}
                        >
                            {tg('discord.perm_reset_member')}
                        </Button>
                    )}
                </span>
            )}
        </li>
    );

    return (
        <>
            {/* 윗줄 (사용자 지정 2026-11-07): 기본 레벨 · 멤버 찾기 · 새로고침(아이콘) */}
            <div className="people-bar">
                <span className="people-default" title={tg('discord.perm_default_level_hint')}>
                    <strong>{tg('discord.perm_default_level_label')}</strong>
                    <LevelSelect
                        value={defaultLevel}
                        perm={perm}
                        label={t('perm.default')}
                        t={t}
                        disabled={busy}
                        onChange={(l) => l !== null && write(() => api.permissionsUpdate(guildId, { defaultLevel: l }))}
                    />
                </span>
                {/* 멤버 찾기: 결과를 끌어 레벨 칸에 놓는다. 소유자는 따로 정한다 */}
                <MemberSearch
                    members={members.filter((m) => String(m.id) !== owner)}
                    label={tg('discord.perm_member_find')}
                    nameOf={memberName}
                    avatarOf={avatarOf}
                    disabled={busy}
                    dragging={dragging?.id ?? null}
                    dragProps={(id) => peopleDrag.bind(`${id}|${perm.members?.[id] ?? defaultLevel}`)}
                />
                <button
                    type="button"
                    className="icon-btn"
                    aria-label={tg('discord.reload')}
                    title={tg('discord.reload')}
                    disabled={busy}
                    onClick={reload}
                >
                    <Icon name="refresh" size={18} />
                </button>
            </div>
            <small className="hint">
                {tg('discord.perm_inherit_hint')} {tg('discord.perm_people_server_wide')}
            </small>

            <ol className="level-rows">
                {L.ladderOf(perm).map(({ id: levelId }) => {
                    const level = levelId as number;
                    const ids = L.sortByName(groups.get(level) || [], memberName);
                    const summary = levelSummary(tg, perm, board, level);
                    const isDefault = level === defaultLevel;
                    const droppable = !!dragging && level !== L.OWNER && level !== dragging.level;
                    return (
                        <li
                            key={level}
                            className={`level-row${over === level && droppable ? ' is-over' : ''}`}
                            aria-label={L.levelLabel(tg, perm, level)}
                            data-drop={level}
                        >
                            <div className="level-head">
                                <span className="stair-num">Lv {L.levelNumber(perm, level)}</span>
                                <strong>{L.levelLabel(tg, perm, level)}</strong>
                                {isDefault && <span className="pill tone-info">{tg('discord.perm_default_badge')}</span>}
                                {(level === L.OWNER || level === L.BANNED) && (
                                    <span className="pill">{tg('discord.perm_owner_fixed')}</span>
                                )}
                            </div>
                            {summary && <small className="hint">{summary}</small>}
                            {level === L.OWNER ? (
                                <div className="people">
                                    {owner ? (
                                        <span className="person-chip is-static is-owner">
                                            <Avatar url={avatarOf(owner)} name={memberName(owner)} className="avatar avatar-chip" />
                                            <span>{memberName(owner)}</span>
                                        </span>
                                    ) : (
                                        <span className="hint">{tg('discord.perm_owner_none')}</span>
                                    )}
                                </div>
                            ) : (
                                <ul className="people">
                                    {ids.map((id) => chip(id, level))}
                                    {isDefault && others.length > 0 && !showDefault && (
                                        <li>
                                            <button
                                                type="button"
                                                className="person-chip is-add"
                                                onClick={() => setShowDefault(true)}
                                            >
                                                {tg('discord.perm_default_more', { count: others.length })}
                                            </button>
                                        </li>
                                    )}
                                    {isDefault && showDefault && others.map((m) => chip(String(m.id), level))}
                                    <li>
                                        <button
                                            type="button"
                                            className={`person-chip is-add${adding === level ? ' is-active' : ''}`}
                                            aria-expanded={adding === level}
                                            aria-label={tg('discord.perm_add_person_to', {
                                                level: L.levelLabel(tg, perm, level),
                                            })}
                                            onClick={() => setAdding(adding === level ? null : level)}
                                        >
                                            <Icon name="plus" size={14} /> {tg('discord.perm_add_person')}
                                        </button>
                                    </li>
                                </ul>
                            )}
                            {adding === level && (
                                <div className="add-person">
                                    <MemberSearch
                                        members={addable(level)}
                                        label={tg('discord.perm_add_search', { level: L.levelLabel(tg, perm, level) })}
                                        nameOf={memberName}
                                        avatarOf={avatarOf}
                                        disabled={busy}
                                        autoFocus
                                        onPick={(id) => {
                                            setAdding(null);
                                            setMember(id, level);
                                        }}
                                    />
                                </div>
                            )}
                        </li>
                    );
                })}
            </ol>
            {members.length === 0 && <Empty>{t('perm.no_members')}</Empty>}
            <LadderEditor
                perm={perm}
                board={board}
                busy={busy}
                onSave={(body) => write(() => api.permissionsUpdate(guildId, body))}
            />
        </>
    );
}

/** 권한과 멤버를 다 읽은 뒤의 길드 권한 */
type ReadyPermissions = ReturnType<typeof useGuildPermissions> & { perm: PermData; members: DiscordMember[] };

interface InstanceMenuProps {
    api: WebApi;
    guildId: string;
    data: ReadyPermissions;
    instance: string;
    onBack: () => void;
}

/**
 * 인스턴스 메뉴 (GUI 와 같다): 맨 위 사용 모드. 서버 공용이면 레벨별 명령과 자동 강퇴 단추, 전용 멤버면 전용 멤버 명단과
 * 명단 밖 자동 강퇴 스위치. 멤버 레벨은 디스코드 서버 단위라 서버의 멤버 탭에 있다
 */
function InstanceMenu({ api, guildId, data, instance, onBack }: InstanceMenuProps) {
    const { t } = useTranslation();
    const { perm, board, busy, write } = data;
    const srv = (board?.instances as BoardInstance[] | undefined)?.find((i) => i.id === instance);
    const server = useData((s) => s.servers.find((x) => x.id === instance));
    if (!srv) return null;
    const members = accessOf(server) === 'members';
    return (
        <div className="instance-menu">
            <button type="button" className="back" onClick={onBack}>
                <Icon name="back" size={16} /> {t('perm.back_instances')}
            </button>
            <h3>
                {srv.name}
                {srv.module && <small className="hint"> · {srv.module}</small>}
            </h3>
            {/* 사용 모드와 명단은 인스턴스에 붙어 이은 모든 디스코드 서버에 같이 적용된다 */}
            {server && (
                <AccessBar api={api} server={server}>
                    {!members && <KickButton api={api} server={server} />}
                </AccessBar>
            )}
            {members && server ? (
                <>
                    <ControllersPanel api={api} server={server} />
                    <MembersKick api={api} server={server} />
                </>
            ) : board ? (
                <PermissionStairs
                    api={api}
                    guildId={guildId}
                    perm={perm}
                    board={board}
                    instance={srv.id}
                    busy={busy}
                    write={write}
                />
            ) : (
                <Empty>{t('perm.stairs_failed')}</Empty>
            )}
        </div>
    );
}

/** 디스코드 서버 하나: 인스턴스 탭(목록 → 인스턴스 메뉴, 켜짐·꺼짐 알림·자리 꽉 참 알림은 목록 아래) | 멤버 탭(서버 전체의 멤버 레벨) */
export function GuildPermissions({
    api,
    guildId,
    siblings = [],
}: {
    api: WebApi;
    guildId: string;
    siblings?: string[];
}) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const data = useGuildPermissions(api, guildId);
    const [instance, setInstance] = useState<string | null>(null);
    const [tab, setTab] = useState('instances');
    // 켜짐 알림과 보일 서버: {guildId, instances, allowedInstances}. null = 읽는 중, false = 읽지 못함
    const [notice, setNotice] = useState<NoticeState>(null);
    // 자리 꽉 참 알림 {guildId, instances, allowedInstances} (모양은 위와 같다)
    const [full, setFull] = useState<NoticeState>(null);
    // 이 PC 의 다른 디스코드 서버가 고른 서버 (서버 하나는 디스코드 서버 하나만)
    const [others, setOthers] = useState<Record<string, string[]> | null>(null);
    const siblingKey = siblings.join(',');

    useEffect(() => {
        let alive = true;
        setNotice(null);
        setFull(null);
        api.guildStartNoticeGet(guildId).then((res) => {
            if (!alive) return;
            if (failed(res)) {
                toast(errorText(t, res), 'error');
                setNotice(false);
            } else setNotice(res as NoticeState);
        });
        api.guildFullNoticeGet(guildId).then((res) => {
            if (alive) setFull(failed(res) ? false : (res as NoticeState));
        });
        return () => {
            alive = false;
        };
    }, [api, guildId, t, toast]);

    // biome-ignore lint/correctness/useExhaustiveDependencies: siblingKey 가 siblings 의 값이다
    useEffect(() => {
        let alive = true;
        setOthers(null);
        Promise.all(siblings.map((g) => api.guildStartNoticeGet(g))).then((list) => {
            if (!alive) return;
            const out: Record<string, string[]> = {};
            list.forEach((res, i) => {
                if (!failed(res) && Array.isArray(res?.allowedInstances)) out[siblings[i]] = res.allowedInstances;
            });
            setOthers(out);
        });
        return () => {
            alive = false;
        };
    }, [api, siblingKey]);

    if (!data.perm || !data.members) return <Loading />;
    if (instance)
        return (
            <InstanceMenu
                api={api}
                guildId={guildId}
                data={data as ReadyPermissions}
                instance={instance}
                onBack={() => setInstance(null)}
            />
        );
    const instances: BoardInstance[] = data.board?.instances || [];
    const tabs = (
        <Segments
            value={tab}
            onChange={setTab}
            options={[
                { value: 'instances', label: t('perm.instances') },
                { value: 'members', label: t('perm.view_members') },
            ]}
        />
    );
    if (tab === 'members')
        return (
            <>
                {tabs}
                <MemberLevels
                    api={api}
                    guildId={guildId}
                    perm={data.perm}
                    board={data.board}
                    members={data.members}
                    busy={data.busy}
                    write={data.write}
                    reload={data.reload}
                />
            </>
        );
    return (
        <>
            {tabs}
            <div className="set-card-grid">
                <Card icon="server" tone="1" title={t('perm.instances')} desc={t('perm.instances_hint')}>
                    <ul className="rows">
                        {instances.map((i) => (
                            <li key={i.id} className="row">
                                <button type="button" className="row-open" onClick={() => setInstance(i.id)}>
                                    <span className="row-main">
                                        <span className="row-title row-title-one">{i.name}</span>
                                        {i.module && <span className="row-sub">{i.module}</span>}
                                    </span>
                                    <Icon name="chevronRight" size={16} />
                                </button>
                            </li>
                        ))}
                    </ul>
                </Card>
                {notice && others && (
                    <VisibleServers
                        api={api}
                        guildId={guildId}
                        allowed={notice.allowedInstances ?? null}
                        others={others}
                        onSaved={(allowedInstances) => setNotice((n) => (n ? { ...n, allowedInstances } : n))}
                    />
                )}
                <StartNotice api={api} guildId={guildId} notice={notice} setNotice={setNotice} />
                <StartNotice api={api} guildId={guildId} notice={full} setNotice={setFull} kind="full" />
            </div>
        </>
    );
}
