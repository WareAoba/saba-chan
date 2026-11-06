import * as L from '@gui/components/discord/levels';
import type { CommandBoard, GuildPermissions as PermData, LadderLevel } from '@gui/components/discord/levels';
import { useFlip } from '@gui/components/discord/flip';
import * as P from '@gui/components/discord/nodePermissions';
import type { DiscordMember } from '@gui/components/discord/types';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { CommandList, type PermissionWriter } from './CommandList';
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
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** 실제 레벨 높은 순(길드 사다리 순서, 주인 먼저, 따로 정하지 않았으면 기본 레벨), 같은 레벨은 이름순 */
export function orderMembers(members: DiscordMember[], perm: PermData | null | undefined) {
    const own = perm?.members || {};
    const fallback = typeof perm?.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const ladder = L.ladderOf(perm || {});
    const rankOf = (m: DiscordMember) => {
        const id = String(m.id);
        const level = id === String(perm?.ownerDiscordId ?? '') ? 3 : Object.hasOwn(own, id) ? own[id] : fallback;
        const at = ladder.findIndex((l) => l.id === level);
        return at < 0 ? ladder.length : at;
    };
    return [...members].sort((a, b) => rankOf(a) - rankOf(b) || collator.compare(nameOf(a), nameOf(b)));
}

/** 멤버 프로필 사진 (ui.tsx Avatar) */
const MemberAvatar = ({ member }: { member: DiscordMember }) => (
    <Avatar url={member.avatarUrl || member.avatar} name={nameOf(member)} />
);

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

/** 단계 사다리 (위가 높은 단계). 휴대폰이라 끌기 대신 위·아래 버튼으로 옮긴다. 소유자·차단은 고정.
 * 모든 레벨은 이름을 바꾸고(비우면 원래 이름), 소유자·게스트·차단은 지울 수 없다. 조작 가능 줄도 위·아래 버튼으로 옮긴다
 * (사용자 지정 2026-11-06, GUI LadderTab 과 같은 규칙) */
interface LadderEditorProps {
    perm: PermData;
    board: CommandBoard | null;
    busy: boolean;
    onSave: (body: {
        ladder?: ReturnType<typeof L.ladderPatch>;
        commandLevels?: Record<string, number | null>;
    }) => void;
}

const MAX_CHIPS = 12; // 칸마다 보이는 칩 수 (넘치면 +N, 앱과 같다)

export function LadderEditor({ perm, board, busy, onSave }: LadderEditorProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const [name, setName] = useState('');
    const line = L.controllerLine(board);
    const [lineLevel, setLineLevel] = useState(line?.level ?? null);
    // 권한 맞바꾸기 (사용자 지정 2026-11-07): 아래 버튼을 누르고 칸 두 개를 고르면 두 레벨의 명령이 자리를 바꾼다 (앱과 같은 규칙)
    const [swaps, setSwaps] = useState<[number, number][]>([]);
    const [choosing, setChoosing] = useState(false);
    const [picked, setPicked] = useState<number | null>(null);
    const listRef = useRef<HTMLUListElement>(null);
    const flip = useFlip(listRef);
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    useEffect(() => setLineLevel(line?.level ?? null), [line?.level]);
    // 데몬 값을 다시 읽으면(저장 뒤) 맞바꾸기 초안을 비운다
    // biome-ignore lint/correctness/useExhaustiveDependencies: board 가 바뀔 때만 비운다
    useEffect(() => setSwaps([]), [board]);
    const patch = L.ladderPatch(draft);
    const ladderChanged = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const lineAt = line && lineLevel !== null ? L.lineAt(saved, draft, lineLevel) : null;
    const lineTarget = lineAt === null ? null : (draft[lineAt].id as number);
    const levels = L.draftLevels(board, draft, swaps, line, lineTarget);
    const commandLevels = L.boardPatch(board, levels);
    const commandsChanged = Object.keys(commandLevels).length > 0;
    const changed = ladderChanged || commandsChanged;
    const chips = L.stairChips(L.draftBoard(board, levels));
    const instanceName = (id?: string) => board?.instances?.find((i) => i.id === id)?.name || id;
    const chipLabel = (c: L.StairChip) => (c.instance ? `${c.title} · ${instanceName(c.instance)}` : c.title || c.name);
    // 마지막 맞바꾸기 (두 칸이 아직 초안에 있을 때만 알린다)
    const lastSwap =
        swaps.filter(([a, b]) => draft.some((l) => l.id === a) && draft.some((l) => l.id === b)).at(-1) ?? null;
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
        setSwaps((s) => [...s, [picked, id]]);
        setLineLevel((l) => (l === null ? l : L.swapLine(l, picked, id)));
        setPicked(null);
        setChoosing(false);
    };
    const moveLine = (dir: -1 | 1) => {
        const to = lineAt === null ? null : L.lineStep(draft, lineAt, dir);
        if (to !== null) setLineLevel(draft[to].id ?? null);
    };
    const save = () =>
        onSave({
            ...(ladderChanged ? { ladder: patch } : {}),
            ...(commandsChanged ? { commandLevels } : {}),
        });
    const lineRow = (at: number) => (
        <li key="controllers-line" className="row ladder-line" aria-label={tg('discord.perm_line')}>
            <div className="row-main">
                <div className="row-title">
                    {tg(at === 0 ? 'discord.perm_line_only' : 'discord.perm_line_from', { level: label(draft[at]) })}
                </div>
                <div className="row-sub">
                    {tg('discord.perm_line_hint')}
                    {line && line.apart > 0 && ` ${tg('discord.perm_line_apart', { count: line.apart })}`}
                </div>
            </div>
            <Button
                disabled={L.lineStep(draft, at, -1) === null}
                aria-label={tg('discord.perm_line_up')}
                onClick={() => moveLine(-1)}
            >
                <Icon name="chevronUp" size={16} />
            </Button>
            <Button
                disabled={L.lineStep(draft, at, 1) === null}
                aria-label={tg('discord.perm_line_down')}
                onClick={() => moveLine(1)}
            >
                <Icon name="chevronDown" size={16} />
            </Button>
        </li>
    );
    return (
        <div className="field">
            <span>{t('perm.ladder')}</span>
            <small className="hint">{t('perm.ladder_hint')}</small>
            <ul ref={listRef} className="rows">
                {draft.flatMap((l, i) => {
                    const fixed = i === 0 || i === last;
                    const pickable = choosing && L.swappable(l);
                    const own = L.swappable(l) ? chips.filter((c) => c.level === l.id) : [];
                    const row = (
                        <li
                            key={l.id ?? l.key}
                            className={`row ladder-row${pickable ? ' is-pickable' : ''}${picked !== null && picked === l.id ? ' is-picked' : ''}`}
                            aria-label={label(l)}
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
                                <input
                                    value={l.name ?? ''}
                                    placeholder={L.isCustomLevel(l) ? '' : localeName(l)}
                                    maxLength={L.MAX_NAME}
                                    aria-label={t('perm.ladder_name')}
                                    onChange={(e) => setDraft((d) => L.renameLevel(d, i, e.target.value))}
                                />
                                {L.swappable(l) && (
                                    <div className="ladder-chips">
                                        {own.slice(0, MAX_CHIPS).map((c) => (
                                            <span
                                                key={c.id}
                                                data-flip={c.id}
                                                className={`pill${c.fixed ? ' is-fixed' : ''}`}
                                            >
                                                {chipLabel(c)}
                                            </span>
                                        ))}
                                        {own.length > MAX_CHIPS && (
                                            <span className="pill is-fixed">
                                                {tg('discord.perm_swap_more', { count: own.length - MAX_CHIPS })}
                                            </span>
                                        )}
                                        {own.length === 0 && (
                                            <span className="row-sub">{tg('discord.perm_swap_none')}</span>
                                        )}
                                    </div>
                                )}
                            </div>
                            {fixed ? (
                                <span className="pill">{t('perm.ladder_fixed')}</span>
                            ) : (
                                <>
                                    <Button
                                        disabled={i <= 1}
                                        aria-label={t('perm.ladder_up', { name: label(l) })}
                                        onClick={() => setDraft((d) => L.moveLevel(d, i, i - 1))}
                                    >
                                        <Icon name="chevronUp" size={16} />
                                    </Button>
                                    <Button
                                        disabled={i >= last - 1}
                                        aria-label={t('perm.ladder_down', { name: label(l) })}
                                        onClick={() => setDraft((d) => L.moveLevel(d, i, i + 1))}
                                    >
                                        <Icon name="chevronDown" size={16} />
                                    </Button>
                                    <Button
                                        tone="ghost-danger"
                                        disabled={draft.length <= 3 || !L.removable(draft, i)}
                                        aria-label={t('perm.ladder_delete', { name: label(l) })}
                                        onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                    >
                                        <Icon name="close" size={16} />
                                    </Button>
                                </>
                            )}
                        </li>
                    );
                    return lineAt === i ? [row, lineRow(i)] : [row];
                })}
            </ul>
            <div className="field-row">
                <Button
                    aria-pressed={choosing}
                    onClick={() => {
                        setChoosing((c) => !c);
                        setPicked(null);
                    }}
                >
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
                <Button
                    disabled={busy || !changed}
                    onClick={() => {
                        setDraft(saved);
                        setLineLevel(line?.level ?? null);
                        setSwaps([]);
                        setChoosing(false);
                        setPicked(null);
                    }}
                >
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

    useEffect(() => {
        let live = true;
        setMembers(null);
        api.relayListNodeMembers(guildId, '').then((m) => live && setMembers(Array.isArray(m?.members) ? m.members : []));
        return () => {
            live = false;
        };
    }, [api, guildId]);

    const write: PermissionWriter = async (fn) => {
        setBusy(true);
        const res = await fn();
        if (failed(res)) toast(errorText(t, res), 'error');
        else toast(t('perm.saved'), 'ok');
        await load();
        setBusy(false);
    };
    return { perm, board, members, busy, write };
}

/** 멤버별 레벨: 기본 레벨, 멤버마다 레벨, 레벨 사다리 편집 (길드 전체에 적용) */
interface MemberLevelsProps {
    api: WebApi;
    guildId: string;
    perm: PermData;
    board: CommandBoard | null;
    members: DiscordMember[];
    busy: boolean;
    write: PermissionWriter;
}

function MemberLevels({ api, guildId, perm, board, members, busy, write }: MemberLevelsProps) {
    const { t } = useTranslation();
    const [query, setQuery] = useState('');
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const levelOf = (id: string) => (Object.hasOwn(perm.members || {}, id) ? (perm.members as Record<string, number>)[id] : null);
    const q = query.trim().toLowerCase();
    const shown = orderMembers(members, perm).filter(
        (m) =>
            !q ||
            [nameOf(m), m.username, m.id].some((v) =>
                String(v ?? '')
                    .toLowerCase()
                    .includes(q),
            ),
    );

    return (
        <>
            <small className="hint">{t('perm.members_server_wide')}</small>
            <div className="field">
                <span>{t('perm.default')}</span>
                <LevelSelect
                    value={defaultLevel}
                    perm={perm}
                    label={t('perm.default')}
                    t={t}
                    disabled={busy}
                    onChange={(l) => l !== null && write(() => api.permissionsUpdate(guildId, { defaultLevel: l }))}
                />
                <small className="hint">{t('perm.default_hint')}</small>
            </div>
            <input
                className="search"
                type="search"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t('perm.search')}
            />
            {shown.length === 0 ? (
                <Empty>{t('perm.no_members')}</Empty>
            ) : (
                <ul className="rows">
                    {shown.map((m) => {
                        const owner = String(m.id) === String(perm.ownerDiscordId);
                        return (
                            <li key={m.id} className="row member-row">
                                <MemberAvatar member={m} />
                                <div className="row-main">
                                    <div className="row-title row-title-one">{nameOf(m)}</div>
                                    {m.username && <div className="row-sub row-title-one">@{m.username}</div>}
                                </div>
                                {owner ? (
                                    <span className="pill">{t('perm.owner')}</span>
                                ) : (
                                    <LevelSelect
                                        value={levelOf(String(m.id))}
                                        perm={perm}
                                        label={nameOf(m)}
                                        withDefault
                                        t={t}
                                        disabled={busy}
                                        onChange={(l) =>
                                            write(() =>
                                                l === null
                                                    ? api.permissionsRemoveMember(guildId, String(m.id))
                                                    : api.permissionsSetMember(guildId, String(m.id), l),
                                            )
                                        }
                                    />
                                )}
                            </li>
                        );
                    })}
                </ul>
            )}
            <LadderEditor
                perm={perm}
                board={board}
                busy={busy}
                onSave={(body) => write(() => api.permissionsUpdate(guildId, body))}
            />
        </>
    );
}

/** 인스턴스 메뉴: 레벨별 명령과 멤버별 레벨을 따로 보인다 (GUI와 같은 동선) */
/** 권한과 멤버를 다 읽은 뒤의 길드 권한 */
type ReadyPermissions = ReturnType<typeof useGuildPermissions> & { perm: PermData; members: DiscordMember[] };

interface InstanceMenuProps {
    api: WebApi;
    guildId: string;
    data: ReadyPermissions;
    instance: string;
    onBack: () => void;
}

function InstanceMenu({ api, guildId, data, instance, onBack }: InstanceMenuProps) {
    const { t } = useTranslation();
    const [view, setView] = useState('commands');
    const { perm, board, members, busy, write } = data;
    const srv = (board?.instances as BoardInstance[] | undefined)?.find((i) => i.id === instance);
    return (
        <div className="instance-menu">
            <button type="button" className="back" onClick={onBack}>
                <Icon name="back" size={16} /> {t('perm.back_instances')}
            </button>
            <h3>
                {srv ? srv.name : t('perm.all_instances')}
                {srv?.module && <small className="hint"> · {srv.module}</small>}
            </h3>
            <Segments
                value={view}
                onChange={setView}
                options={[
                    { value: 'commands', label: t('perm.view_commands') },
                    { value: 'members', label: t('perm.view_members') },
                ]}
            />
            {view === 'commands' ? (
                board ? (
                    <CommandList
                        api={api}
                        guildId={guildId}
                        perm={perm}
                        board={board}
                        instance={srv ? srv.id : L.ALL_INSTANCES}
                        busy={busy}
                        write={write}
                    />
                ) : (
                    <Empty>{t('perm.stairs_failed')}</Empty>
                )
            ) : (
                <MemberLevels
                    api={api}
                    guildId={guildId}
                    perm={perm}
                    board={board}
                    members={members}
                    busy={busy}
                    write={write}
                />
            )}
        </div>
    );
}

/** 디스코드 서버 하나: 인스턴스 목록 → 인스턴스 메뉴. 켜짐·꺼짐 알림·자리 꽉 참 알림은 목록 아래 */
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
    return (
        <div className="set-card-grid">
            <Card icon="server" tone="1" title={t('perm.instances')} desc={t('perm.instances_hint')}>
                <ul className="rows">
                    {[{ id: L.ALL_INSTANCES, name: t('perm.all_instances') }, ...instances].map((i) => (
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
    );
}
