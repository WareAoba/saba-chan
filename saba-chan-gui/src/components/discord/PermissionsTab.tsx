import clsx from 'clsx';
import type { TFunction } from 'i18next';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties, PointerEvent as ReactPointerEvent, RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaSpinner } from '../ui/SabaUI';
import { SabaFail } from '../ui/SabaArt';
import { Segmented } from '../ui/StatusChip';
import { Avatar } from './Avatar';
import { SaveBar } from './ChatbotView';
import { LadderTab } from './LadderTab';
import * as L from './levels';
import { usePointerDrag } from './pointerDrag';
import type { DiscordMember, DiscordNode } from './types';

export const OWNER_CONFIRM_MS = 5000;

const failed = (res: ApiBody | null | undefined) =>
    !res || res.success === false || Boolean(res.error || res.error_code);

export function permissionsErrorText(res: ApiBody | null | undefined, t: TFunction) {
    switch (res?.error_code || res?.code) {
        case 'owner_fixed':
        case 'OWNER_FIXED':
            return t('discord.perm_err_owner_fixed');
        case 'invalid_level':
            return t('discord.perm_err_invalid_level');
        case 'invalid_ladder':
            return t('discord.perm_err_invalid_ladder');
        default:
            return res?.error || t('discord.perm_err_generic');
    }
}

/** 길드 하나의 권한과 명령 계단: 읽기와, 쓴 뒤 다시 읽기 */
export function usePermissions(guildId: string) {
    const { t, i18n } = useTranslation('gui');
    const [perm, setPerm] = useState<L.GuildPermissions | null>(null);
    const [board, setBoard] = useState<L.CommandBoard | null>(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState<ApiBody | null>(null);
    const [busy, setBusy] = useState(false);
    const locale = i18n.language;
    const seq = useRef(0); // 늦게 온 앞선 읽기가 새 값을 덮지 않게

    const load = useCallback(async () => {
        const mine = ++seq.current;
        try {
            const [res, stairs] = await Promise.all([
                window.api.permissionsGet(guildId),
                window.api.permissionsCommands?.(guildId, locale),
            ]);
            if (mine !== seq.current) return;
            if (failed(res)) setError(res || {});
            else {
                setPerm(res);
                setError(null);
            }
            // 계단을 못 읽어도 사람 화면은 쓴다
            setBoard(stairs && !failed(stairs) ? stairs : null);
        } catch (e) {
            if (mine !== seq.current) return;
            setError({ error: (e as Error).message });
        }
        setLoading(false);
    }, [guildId, locale]);

    useEffect(() => {
        setLoading(true);
        setPerm(null);
        setBoard(null);
        load();
    }, [load]);

    /** 쓰기 하나(또는 여러 개)를 하고 다시 읽는다. 실패하면 알림만 띄우고 화면은 데몬 값 그대로 */
    const write = useCallback(
        async (...calls: (() => Promise<ApiBody>)[]) => {
            setBusy(true);
            let ok = true;
            for (const call of calls) {
                try {
                    const res = await call();
                    if (failed(res)) {
                        ok = false;
                        safeShowToast(permissionsErrorText(res, t), 'error', 5000);
                        break;
                    }
                } catch (e) {
                    ok = false;
                    safeShowToast((e as Error).message || t('discord.perm_err_generic'), 'error', 5000);
                    break;
                }
            }
            await load();
            setBusy(false);
            return ok;
        },
        [load, t],
    );

    return { perm, board, loading, error, busy, load, write };
}

export type PermissionsState = ReturnType<typeof usePermissions>;

const memberName = (m: DiscordMember | undefined) => m?.displayName || m?.globalName || m?.username;
const isId = (v: string) => /^\d{17,20}$/.test(v); // 디스코드 snowflake

interface LevelSelectProps {
    value: number;
    levels: number[];
    perm: L.GuildPermissions;
    onChange: (level: number) => void;
    label: string;
    disabled?: boolean;
}

function LevelSelect({ value, levels, perm, onChange, label, disabled }: LevelSelectProps) {
    const { t } = useTranslation('gui');
    return (
        <CustomDropdown
            className="dc-level-select"
            value={String(value)}
            ariaLabel={label}
            disabled={disabled}
            onChange={(v) => onChange(Number(v))}
            options={levels.map((l) => ({ value: String(l), label: L.levelTitle(t, perm, l) }))}
        />
    );
}

interface PermStateProps {
    loading: boolean;
    error: ApiBody | null;
    onRetry: () => void;
}

function PermState({ loading, error, onRetry }: PermStateProps) {
    const { t } = useTranslation('gui');
    if (loading) {
        return (
            <div className="dc-pair-wait pad">
                <SabaSpinner size="sm" /> {t('discord.perm_loading')}
            </div>
        );
    }
    return <SabaFail message={permissionsErrorText(error, t)} onRetry={onRetry} retryLabel={t('discord.reload')} />;
}

/** 레벨마다 그 레벨에서 새로 쓸 수 있게 되는 명령 수 (큰 칩만, help·list 제외) */
function gainsByLevel(board: L.CommandBoard, view?: string) {
    const gains: Record<number, number> = {};
    for (const chip of L.stairChips(board, view).filter((c) => !c.instance))
        gains[chip.level] = (gains[chip.level] || 0) + 1;
    return gains;
}

/** "Lv 1의 모든 명령 + 5개" 같은 한 줄. 계단을 못 읽었으면 없다 */
function levelSummary(
    t: TFunction,
    perm: L.GuildPermissions,
    board: L.CommandBoard | null,
    id: number | undefined,
    view?: string,
) {
    if (id === L.OWNER) return t('discord.perm_level_all');
    if (id === L.BANNED) return t('discord.perm_level_none');
    if (!board) return null;
    const up = L.ascending(perm);
    const at = up.indexOf(id as number);
    const below = up[at - 1];
    const gain = gainsByLevel(board, view)[id as number] || 0;
    // 금지 바로 위 레벨은 help·list부터 쓴다
    if (below === L.BANNED) return t('discord.perm_level_base', { count: gain + 1 });
    return t('discord.perm_level_gain', { below: L.levelTitle(t, perm, below), count: gain });
}

/** 검색 결과를 한 번에 보이는 수. 목록 끝까지 내리면 이만큼 더 보인다 (사용자 지정 2026-11-07) */
export const MEMBER_PAGE = 10;

/** 서버 닉네임(표시 이름)·전역 이름·사용자 이름으로 찾는다. 아이디로는 찾지 않는다 */
export function searchMembers(members: DiscordMember[], query: string, name: (m: DiscordMember) => string) {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const hit = members.filter((m) =>
        [m.displayName, m.globalName, m.username].some((v) => typeof v === 'string' && v.toLowerCase().includes(q)),
    );
    return L.sortByName(hit, name);
}

interface MemberSearchProps {
    members: DiscordMember[];
    label: string;
    nameOf: (id: string) => string;
    avatarOf: (id: string) => string | undefined;
    /** 고르면 (사람 추가). 없으면 결과는 끌기만 한다 */
    onPick?: (id: string) => void;
    /** 결과 칩에 펼칠 끌기 속성 (위쪽 검색바) */
    dragProps?: (id: string) => Record<string, unknown>;
    dragging?: string | null;
    disabled?: boolean;
    autoFocus?: boolean;
}

/**
 * 멤버 찾기: 닉네임으로 찾고, 결과는 10명씩 보이며 목록 끝까지 내리면 더 보인다.
 * 사람 추가에서는 결과를 눌러 고르고, 위쪽 검색바에서는 결과를 끌어 레벨 칸에 놓는다
 */
function MemberSearch({
    members,
    label,
    nameOf,
    avatarOf,
    onPick,
    dragProps,
    dragging,
    disabled,
    autoFocus,
}: MemberSearchProps) {
    const { t } = useTranslation('gui');
    const [query, setQuery] = useState('');
    const [shown, setShown] = useState(MEMBER_PAGE);
    // 끄는 결과 칩은 팝업(스크롤 칸) 밖으로 띄워야 잘리지 않는다: 누른 자리를 적어 두고 끄는 동안 fixed로 그 자리에 둔다
    const [origin, setOrigin] = useState<{ id: string; left: number; top: number; width: number } | null>(null);
    const results = useMemo(
        () => searchMembers(members, query, (m) => nameOf(String(m.id))),
        [members, query, nameOf],
    );
    const withOrigin = (mid: string, props: Record<string, unknown>) => ({
        ...props,
        onPointerDown: (e: ReactPointerEvent<HTMLElement>) => {
            const box = e.currentTarget.getBoundingClientRect();
            setOrigin({ id: mid, left: box.left, top: box.top, width: box.width });
            (props.onPointerDown as ((ev: ReactPointerEvent<HTMLElement>) => void) | undefined)?.(e);
        },
    });
    const id = query.trim();
    // 멤버 목록에 없는 사람은 디스코드 ID를 그대로 적어 더한다
    const byId = onPick && isId(id) && !members.some((m) => String(m.id) === id);
    const open = !!id;
    return (
        <div className={clsx('dc-member-search', { 'is-dragging': !!dragging })}>
            <input
                className="input"
                role="combobox"
                aria-expanded={open}
                aria-label={label}
                placeholder={t('discord.perm_member_search')}
                value={query}
                disabled={disabled}
                // biome-ignore lint/a11y/noAutofocus: 사람 추가를 누르면 바로 찾게 한다
                autoFocus={autoFocus}
                onChange={(e) => {
                    setQuery(e.target.value);
                    setShown(MEMBER_PAGE);
                }}
                onKeyDown={(e) => {
                    if (e.key === 'Escape') setQuery('');
                }}
            />
            {query && (
                <button
                    type="button"
                    className="icon-btn dc-member-clear"
                    aria-label={t('discord.perm_member_search_clear')}
                    data-tooltip={t('discord.perm_member_search_clear')}
                    onClick={() => setQuery('')}
                >
                    <Icon name="close" size={12} />
                </button>
            )}
            {open && (
                <div
                    className="dc-member-results"
                    role="listbox"
                    aria-label={label}
                    onScroll={(e) => {
                        const el = e.currentTarget;
                        if (el.scrollTop + el.clientHeight >= el.scrollHeight - 24 && shown < results.length)
                            setShown((n) => n + MEMBER_PAGE);
                    }}
                >
                    {results.slice(0, shown).map((m) => {
                        const mid = String(m.id);
                        return (
                            <div key={mid} className="dc-member-result" role="none">
                                <button
                                    type="button"
                                    role="option"
                                    aria-selected={false}
                                    aria-label={nameOf(mid)}
                                    className={clsx('dc-chip', {
                                        'is-dragging': dragging === mid,
                                        'is-floating': dragging === mid && origin?.id === mid,
                                    })}
                                    style={
                                        dragging === mid && origin?.id === mid
                                            ? { left: origin.left, top: origin.top, width: origin.width }
                                            : undefined
                                    }
                                    disabled={disabled}
                                    {...(dragProps ? withOrigin(mid, dragProps(mid)) : {})}
                                    onClick={() => {
                                        if (!onPick) return;
                                        onPick(mid);
                                        setQuery('');
                                    }}
                                >
                                    <Avatar name={nameOf(mid)} url={avatarOf(mid)} size={18} />
                                    <span>{nameOf(mid)}</span>
                                    {m.username && m.username !== nameOf(mid) && (
                                        <small className="muted">@{m.username}</small>
                                    )}
                                </button>
                            </div>
                        );
                    })}
                    {byId && (
                        <div className="dc-member-result" role="none">
                            <button
                                type="button"
                                role="option"
                                aria-selected={false}
                                className="dc-chip is-add"
                                disabled={disabled}
                                onClick={() => {
                                    onPick?.(id);
                                    setQuery('');
                                }}
                            >
                                <Icon name="plus" size="xs" /> {t('discord.perm_add_by_id', { id })}
                            </button>
                        </div>
                    )}
                    {results.length === 0 && !byId && (
                        <div className="muted dc-member-empty" role="none">{t('discord.perm_member_search_empty')}</div>
                    )}
                </div>
            )}
        </div>
    );
}

interface PeopleViewProps {
    node: DiscordNode;
    instance?: string;
    members?: DiscordMember[] | null;
    membersLoading?: boolean;
    onRefreshMembers?: () => void;
    permissions: PermissionsState;
    onEditLevels?: () => void;
}

// ── 사람: 레벨마다 사람 칩 ──
export function PeopleView({
    node,
    instance,
    members,
    membersLoading,
    onRefreshMembers,
    permissions,
    onEditLevels,
}: PeopleViewProps) {
    const { t } = useTranslation('gui');
    const { perm, board, loading, error, busy, load, write } = permissions;
    const guildId = node.guildId;
    const [open, setOpen] = useState<string | null>(null); // 레벨을 고치는 사람 id
    const [adding, setAdding] = useState<number | null>(null); // 사람을 더하는 레벨
    const [showDefault, setShowDefault] = useState(false);
    const [picked, setPicked] = useState<string[]>([]);
    const [ownerDraft, setOwnerDraft] = useState<string | null>(null); // null = 편집 안 함
    const [ownerArmed, setOwnerArmed] = useState(false); // 릴레이 값은 덜 믿으므로 한 번 더 누르게 한다
    // 사람 칩 끌기: key는 `id|레벨`. 놓으면 dropRef가 옮긴다 (아래에서 권한 값을 읽은 뒤 채운다)
    const dropRef = useRef<(id: string, from: number, to: number) => void>(() => {});
    const peopleDrag = usePointerDrag((key: string, to: string) => {
        const [id, from] = key.split('|');
        dropRef.current(id, Number(from), Number(to));
    });
    const dragging = peopleDrag.dragging
        ? { id: peopleDrag.dragging.split('|')[0], level: Number(peopleDrag.dragging.split('|')[1]) }
        : null;
    const over = peopleDrag.over === null ? null : Number(peopleDrag.over);

    // 가져오기 후보는 릴레이에 남은 부여값(legacy-members)에서 고른다. 디스코드 멤버 목록에는 레벨이 없다
    // 새로고침 때도 다시 읽는다 (옛 레벨을 가져오지 않게). 늦게 온 옛 응답은 버린다
    const [legacy, setLegacy] = useState<DiscordMember[]>([]);
    const legacyGen = useRef(0);
    const loadLegacy = useCallback(async () => {
        if (!window.api?.relayListLegacyMembers) return;
        const gen = ++legacyGen.current;
        // 새 목록이 올 때까지 옛 후보와 고른 것을 비워 옛 레벨을 가져오지 못하게 한다
        setLegacy([]);
        setPicked([]);
        let res: unknown;
        try {
            res = await window.api.relayListLegacyMembers(guildId);
        } catch {
            res = null;
        }
        if (gen === legacyGen.current) setLegacy(Array.isArray(res) ? (res as DiscordMember[]) : []);
    }, [guildId]);
    useEffect(() => {
        loadLegacy();
        return () => {
            legacyGen.current += 1;
        };
    }, [loadLegacy]);
    const byId = useMemo(
        () => new Map([...legacy, ...(members || [])].map((m) => [String(m.id), m])),
        [legacy, members],
    );
    const candidates = useMemo(() => L.importCandidates(perm, legacy), [perm, legacy]);
    const others = useMemo(() => L.defaultMembers(perm, members), [perm, members]);

    // 확인 상태는 잠깐만 둔다 (나중에 무심코 한 번 눌러 바뀌지 않게)
    useEffect(() => {
        if (!ownerArmed) return undefined;
        const timer = setTimeout(() => setOwnerArmed(false), OWNER_CONFIRM_MS);
        return () => clearTimeout(timer);
    }, [ownerArmed]);

    if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;

    const nameOf = (id: string) => memberName(byId.get(id)) || id;
    const avatarOf = (id: string) => byId.get(id)?.avatarUrl || byId.get(id)?.avatar;
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const grantable = L.grantable(perm);
    const update = (patch: ApiBody) => write(() => window.api.permissionsUpdate(guildId, patch));
    const setMember = (id: string, level: number) => write(() => window.api.permissionsSetMember(guildId, id, level));
    const owner = perm.ownerDiscordId ? String(perm.ownerDiscordId) : null;
    // 데몬은 처음 받은 소유자를 고정하고, 릴레이가 다른 값을 보내면 무시한 채 알려 준다
    const relayOwner = L.ownerMismatch(perm);
    const saveOwner = async (id: string) => {
        if (await update({ ownerDiscordId: id })) setOwnerDraft(null);
    };
    // 소유자 레벨과 원래 레벨에는 놓아도 바꾸지 않는다
    dropRef.current = (id, from, to) => {
        if (to !== L.OWNER && to !== from && !busy) setMember(id, to);
    };
    const groups = new Map(L.membersByLevel(perm).map((g) => [g.level, g.ids]));
    const sortedOthers = L.sortByName(others, (m) => nameOf(String(m.id)));

    const chip = (id: string, level: number) => (
        <li key={id} className="dc-person">
            <button
                type="button"
                className={clsx('dc-chip', { 'is-open': open === id, 'is-dragging': dragging?.id === id })}
                aria-expanded={open === id}
                aria-label={nameOf(id)}
                {...peopleDrag.bind(`${id}|${level}`)}
                onClick={() => setOpen(open === id ? null : id)}
            >
                <Avatar name={nameOf(id)} url={avatarOf(id)} size={18} />
                <span>{nameOf(id)}</span>
            </button>
            {open === id && (
                <span className="dc-person-edit">
                    <LevelSelect
                        value={level}
                        levels={grantable}
                        perm={perm}
                        label={t('discord.perm_member_level', { name: nameOf(id) })}
                        disabled={busy}
                        onChange={async (l) => {
                            if (l !== level && (await setMember(id, l))) setOpen(null);
                        }}
                    />
                    {level !== defaultLevel || perm.members?.[id] !== undefined ? (
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            disabled={busy || perm.members?.[id] === undefined}
                            onClick={() => write(() => window.api.permissionsRemoveMember(guildId, id))}
                        >
                            {t('discord.perm_reset_member')}
                        </button>
                    ) : null}
                </span>
            )}
        </li>
    );

    // 이 레벨에 더할 수 있는 디스코드 멤버 (소유자와 이미 이 레벨인 사람 제외)
    const addable = (level: number) =>
        L.sortByName(
            (members || []).filter((m) => {
                const id = String(m.id);
                return id !== owner && (perm.members?.[id] ?? defaultLevel) !== level;
            }),
            (m) => nameOf(String(m.id)),
        );

    return (
        <div className="dc-tab">
            {relayOwner && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('discord.perm_owner_mismatch', {
                            relay: nameOf(relayOwner),
                            owner: owner ? nameOf(owner) : t('discord.perm_owner_none'),
                        })}
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={busy}
                        onClick={async () => {
                            if (!ownerArmed) return setOwnerArmed(true);
                            setOwnerArmed(false);
                            await saveOwner(relayOwner);
                        }}
                    >
                        {ownerArmed
                            ? t('discord.perm_owner_use_relay_confirm', { name: nameOf(relayOwner), id: relayOwner })
                            : t('discord.perm_owner_use_relay')}
                    </button>
                </div>
            )}

            {/* 윗줄 (사용자 지정 2026-11-07): 기본 레벨 · 멤버 찾기 · 새로고침(아이콘) · 레벨 편집 */}
            <div className="dc-toolbar dc-people-bar">
                <span className="dc-field-inline" data-tooltip={t('discord.perm_default_level_hint')}>
                    <strong className="dc-bar-label">{t('discord.perm_default_level_label')}</strong>
                    <LevelSelect
                        value={defaultLevel}
                        levels={grantable}
                        perm={perm}
                        label={t('discord.perm_default_level')}
                        disabled={busy}
                        onChange={(level) => update({ defaultLevel: level })}
                    />
                </span>
                {/* 멤버 찾기: 결과를 끌어 레벨 칸에 놓는다. 소유자는 따로 정한다 */}
                <MemberSearch
                    members={(members || []).filter((m) => String(m.id) !== owner)}
                    label={t('discord.perm_member_find')}
                    nameOf={nameOf}
                    avatarOf={avatarOf}
                    disabled={busy}
                    dragging={dragging?.id ?? null}
                    dragProps={(id) => peopleDrag.bind(`${id}|${perm.members?.[id] ?? defaultLevel}`)}
                />
                <span className="dc-mini-actions">
                    <button
                        type="button"
                        className="icon-btn"
                        aria-label={t('discord.reload')}
                        data-tooltip={t('discord.reload')}
                        onClick={() => {
                            load();
                            loadLegacy();
                            onRefreshMembers?.();
                        }}
                        disabled={busy || membersLoading}
                    >
                        <Icon name="refresh" size="sm" />
                    </button>
                    <button type="button" className="btn btn-secondary" onClick={onEditLevels}>
                        <Icon name="key" size="sm" /> {t('discord.perm_edit_levels')}
                    </button>
                </span>
            </div>
            <p className="dc-hint">
                {t('discord.perm_inherit_hint')} {t('discord.perm_people_server_wide')}
            </p>

            <ol className="dc-levels">
                {L.ladderOf(perm).map(({ id: levelId }) => {
                    const level = levelId as number;
                    const ids = L.sortByName(groups.get(level) || [], nameOf);
                    const summary = levelSummary(t, perm, board, level, instance);
                    const isDefault = level === defaultLevel;
                    // 소유자는 따로 정하고, 같은 레벨에 놓으면 바꿀 것이 없다
                    const droppable = !!dragging && level !== L.OWNER && level !== dragging.level;
                    return (
                        <li
                            key={level}
                            className={clsx('dc-level-row', `level-${L.levelKey(level)}`, {
                                'is-over': over === level && droppable,
                            })}
                            aria-label={L.levelLabel(t, perm, level)}
                            data-drop={level}
                        >
                            <div className="dc-level-head">
                                <span className="dc-level-num">Lv {L.levelNumber(perm, level)}</span>
                                <span className="dc-level-name">{L.levelLabel(t, perm, level)}</span>
                                {isDefault && (
                                    <span className="dc-badge tone-info">{t('discord.perm_default_badge')}</span>
                                )}
                                {(level === L.OWNER || level === L.BANNED) && (
                                    <span className="dc-badge">{t('discord.perm_owner_fixed')}</span>
                                )}
                            </div>
                            <div className="dc-level-body">
                                {summary && <span className="dc-level-can">{summary}</span>}
                                {level === L.OWNER ? (
                                    ownerDraft === null ? (
                                        <span className="dc-owner">
                                            {owner ? (
                                                <span className="dc-chip is-static is-owner">
                                                    <Avatar name={nameOf(owner)} url={avatarOf(owner)} size={18} />
                                                    <span>{nameOf(owner)}</span>
                                                </span>
                                            ) : (
                                                <span className="muted">{t('discord.perm_owner_none')}</span>
                                            )}
                                            <button
                                                type="button"
                                                className="btn btn-ghost btn-sm"
                                                disabled={busy}
                                                onClick={() => setOwnerDraft(owner || '')}
                                            >
                                                {t('discord.perm_owner_change')}
                                            </button>
                                        </span>
                                    ) : (
                                        <span className="dc-owner">
                                            <input
                                                className="input"
                                                value={ownerDraft}
                                                inputMode="numeric"
                                                onChange={(e) => setOwnerDraft(e.target.value)}
                                                placeholder={t('discord.perm_add_id')}
                                                aria-label={t('discord.perm_owner')}
                                            />
                                            <button
                                                type="button"
                                                className="btn btn-primary btn-sm"
                                                disabled={
                                                    busy || !isId(ownerDraft.trim()) || ownerDraft.trim() === owner
                                                }
                                                onClick={() => saveOwner(ownerDraft.trim())}
                                            >
                                                {t('discord.perm_owner_save')}
                                            </button>
                                            <button
                                                type="button"
                                                className="btn btn-ghost btn-sm"
                                                onClick={() => setOwnerDraft(null)}
                                            >
                                                {t('modals.cancel')}
                                            </button>
                                        </span>
                                    )
                                ) : (
                                    <ul className="dc-people">
                                        {ids.map((id) => chip(id, level))}
                                        {isDefault && sortedOthers.length > 0 && !showDefault && (
                                            <li>
                                                <button
                                                    type="button"
                                                    className="btn btn-ghost btn-sm"
                                                    onClick={() => setShowDefault(true)}
                                                >
                                                    {t('discord.perm_default_more', { count: sortedOthers.length })}
                                                </button>
                                            </li>
                                        )}
                                        {isDefault && showDefault && sortedOthers.map((m) => chip(String(m.id), level))}
                                        <li>
                                            <button
                                                type="button"
                                                className={clsx('dc-chip is-add', { 'is-active': adding === level })}
                                                aria-expanded={adding === level}
                                                aria-label={t('discord.perm_add_person_to', {
                                                    level: L.levelLabel(t, perm, level),
                                                })}
                                                onClick={() => setAdding(adding === level ? null : level)}
                                            >
                                                <Icon name="plus" size="xs" /> {t('discord.perm_add_person')}
                                            </button>
                                        </li>
                                    </ul>
                                )}
                                {adding === level && (
                                    <div className="dc-add-person">
                                        <MemberSearch
                                            members={addable(level)}
                                            label={t('discord.perm_add_search', { level: L.levelLabel(t, perm, level) })}
                                            nameOf={nameOf}
                                            avatarOf={avatarOf}
                                            disabled={busy}
                                            autoFocus
                                            onPick={async (id) => {
                                                if (await setMember(id, level)) setAdding(null);
                                            }}
                                        />
                                    </div>
                                )}
                            </div>
                        </li>
                    );
                })}
            </ol>
            {membersLoading && (
                <div className="dc-pair-wait pad">
                    <SabaSpinner size="sm" /> {t('discord.members_loading')}
                </div>
            )}
            {!membersLoading && !members?.length && <p className="muted pad">{t('discord.members_empty')}</p>}

            {candidates.length > 0 && (
                <details className="dc-card dc-import">
                    <summary>{t('discord.perm_import_count', { count: candidates.length })}</summary>
                    <p className="dc-hint">{t('discord.perm_import_hint')}</p>
                    <ul className="dc-list">
                        {candidates.map((m) => {
                            const id = String(m.id);
                            const now = perm.members?.[id];
                            return (
                                <li key={id} className="dc-list-row">
                                    <SabaCheckbox
                                        checked={picked.includes(id)}
                                        aria-label={nameOf(id)}
                                        onChange={() =>
                                            setPicked((p) => (p.includes(id) ? p.filter((x) => x !== id) : [...p, id]))
                                        }
                                    />
                                    <span className="dc-list-text">
                                        <span className="dc-list-name">{nameOf(id)}</span>
                                        <span className="dc-list-sub">
                                            {t('discord.perm_import_change', {
                                                from:
                                                    typeof now === 'number'
                                                        ? L.levelTitle(t, perm, now)
                                                        : t('discord.perm_import_default'),
                                                to: L.levelTitle(t, perm, m.level),
                                            })}
                                        </span>
                                    </span>
                                </li>
                            );
                        })}
                    </ul>
                    <div className="dc-row-actions">
                        <button
                            type="button"
                            className="btn btn-primary"
                            disabled={busy || picked.length === 0}
                            onClick={async () => {
                                const chosen = candidates.filter((m) => picked.includes(String(m.id)));
                                await write(
                                    ...chosen.map(
                                        (m) => () =>
                                            window.api.permissionsSetMember(guildId, String(m.id), m.level as number),
                                    ),
                                );
                                setPicked([]);
                            }}
                        >
                            {t('discord.perm_import_button', { count: picked.length })}
                        </button>
                    </div>
                </details>
            )}
        </div>
    );
}

const RISK_TONE: Record<string, string> = { safe: 'ok', caution: 'warn', danger: 'stop' };

// ── 명령: 계단. 칩은 "이 레벨부터 쓸 수 있다"는 자리에 있고, 위 계단은 아래 계단의 명령을 모두 쓴다 ──
/**
 * 레벨 칸이 화면에 남은 높이만 채우게 한다: 페이지는 스크롤되지 않고, 넘치는 칩은 칸 안에서 스크롤된다.
 * 가장 가까운 스크롤 조상의 보이는 높이에서 칸 아래위 내용을 빼고 남는 만큼을 준다 (창 크기·편집 창이 바뀔 때마다 다시 잰다)
 */
function useFillHeight(ref: RefObject<HTMLElement | null>, min = 220) {
    const [, bump] = useState(0);
    useLayoutEffect(() => {
        const el = ref.current;
        if (!el) return;
        let scroller = el.parentElement;
        let child: HTMLElement = el;
        while (scroller && !/(auto|scroll)/.test(getComputedStyle(scroller).overflowY)) {
            child = scroller;
            scroller = scroller.parentElement;
        }
        if (!scroller) return;
        // 스크롤 칸의 바로 아래 감싸개는 min-height로 늘어나 있을 수 있다: 그 안 내용의 끝과 아래 여백으로 잰다
        const padOf = (node: Element) => Number.parseFloat(getComputedStyle(node).paddingBottom) || 0;
        const box = scroller.getBoundingClientRect();
        const bottom = box.top + scroller.clientTop + scroller.clientHeight - padOf(scroller) - padOf(child);
        const parts = child === el ? [el] : [...child.children];
        const contentBottom =
            Math.max(...parts.map((n) => n.getBoundingClientRect().bottom)) + scroller.scrollTop;
        const cur = el.getBoundingClientRect().height;
        const next = Math.max(min, Math.floor(cur + bottom - contentBottom));
        if (Math.abs(next - cur) > 1) el.style.height = `${next}px`;
    });
    useEffect(() => {
        const on = () => bump((n) => n + 1);
        window.addEventListener('resize', on);
        return () => window.removeEventListener('resize', on);
    }, []);
}

/**
 * 칩이 다른 계단으로 옮겨지면 옛 자리에서 새 자리로 미끄러지게 한다 (FLIP).
 * 칩마다 `data-chip` 으로 찾고, 움직임 줄이기 설정이면 건너뛴다.
 * 자리는 그릴 때마다 다시 재고(보기·스크롤·알림 줄이 바뀌어도 옛 값이 남지 않게), 움직임은 `dep`(계단 값)이 바뀔 때만 준다
 */
function useChipFlip(ref: RefObject<HTMLElement | null>, dep: unknown) {
    const last = useRef(new Map<string, DOMRect>());
    const prevDep = useRef(dep);
    useLayoutEffect(() => {
        const root = ref.current;
        if (!root) return;
        const changed = prevDep.current !== dep;
        prevDep.current = dep;
        const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
        const next = new Map<string, DOMRect>();
        for (const el of root.querySelectorAll<HTMLElement>('[data-chip]')) {
            const box = el.getBoundingClientRect();
            next.set(el.dataset.chip as string, box);
            const prev = last.current.get(el.dataset.chip as string);
            if (!changed || reduce || !prev || !el.animate) continue;
            const dx = prev.left - box.left;
            const dy = prev.top - box.top;
            if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
            el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], {
                duration: 260,
                easing: 'cubic-bezier(0.2, 0.8, 0.2, 1)',
            });
        }
        last.current = next;
    });
}

interface CommandStairsProps {
    node: DiscordNode;
    permissions: PermissionsState;
    instance?: string;
}

export function CommandStairs({ node, permissions, instance = L.ALL_INSTANCES }: CommandStairsProps) {
    const { t } = useTranslation('gui');
    const { perm, board, loading, error, busy, load, write } = permissions;
    const [view, setView] = useState(instance);
    const [selected, setSelected] = useState<string | null>(null); // 칩 id
    // 칩 끌기: 놓으면 dropRef가 옮긴다 (아래에서 계단 값을 읽은 뒤 채운다)
    const dropRef = useRef<(id: string, level: number) => void>(() => {});
    const chipDrag = usePointerDrag((id: string, to: string) => dropRef.current(id, Number(to)));
    const dragging = chipDrag.dragging;
    const over = chipDrag.over === null ? null : Number(chipDrag.over);
    const [presetDraft, setPresetDraft] = useState<string | null>(null); // 명령별 레벨을 덮을 프리셋 (확인 전)
    // 옮긴 레벨은 초안이다 (사용자 지정 2026-11-07): 저장 막대에서 저장한다. 명령 키 → 레벨
    const [moves, setMoves] = useState<Record<string, number>>({});
    // biome-ignore lint/correctness/useExhaustiveDependencies: 데몬 값을 다시 읽으면(저장 뒤) 초안을 비운다
    useEffect(() => setMoves({}), [board]);
    const stairsRef = useRef<HTMLOListElement>(null);
    // 칩은 저장된 값이 바뀔 때도, 초안에서 옮길 때도 미끄러진다
    // biome-ignore lint/correctness/useExhaustiveDependencies: board·moves 가 바뀔 때마다 새 표지
    const flipToken = useMemo(() => ({}), [board, moves]);
    useChipFlip(stairsRef, flipToken);
    useFillHeight(stairsRef);
    if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
    if (!board)
        return <SabaFail message={t('discord.perm_err_generic')} onRetry={load} retryLabel={t('discord.reload')} />;

    const guildId = node.guildId;
    const update = (patch: ApiBody) => write(() => window.api.permissionsUpdate(guildId, patch));
    const items = board.items || [];
    const draftLevels = items.map((it) =>
        !it.fixed && moves[it.key as string] !== undefined ? moves[it.key as string] : it.level,
    );
    const shownBoard = L.draftBoard(board, draftLevels);
    const patch = L.boardPatch(board, draftLevels);
    const dirty = Object.keys(patch).length > 0;
    // 노란 테두리는 저장하지 않은 변경: 저장된 레벨과 초안 레벨이 다른 명령
    const savedLevel = new Map(items.map((it) => [it.key as string, it.level]));
    const unsaved = (c: L.StairChip) => c.items.some((it) => savedLevel.get(it.key as string) !== it.level);
    const draftTo = (list: L.CommandItem[], level: (it: L.CommandItem) => number) =>
        setMoves((m) => ({
            ...m,
            ...Object.fromEntries(list.filter((it) => !it.fixed).map((it) => [it.key as string, level(it)])),
        }));
    const preset = L.PRESETS.includes(perm.preset as string) ? (perm.preset as string) : 'default';
    // 프리셋은 한 번 적용하는 틀이다: 명령별 레벨이 있으면 다시 맞춘다고 먼저 묻는다 (지금 프리셋을 다시 눌러도)
    const overrides = Object.keys(perm.commandLevels || {}).length;
    const pickPreset = (p: string) => {
        if (busy || (p === preset && !overrides)) return;
        if (overrides) setPresetDraft(p);
        else update({ preset: p });
    };
    const instances = board.instances || [];
    const shownView = view === L.ALL_INSTANCES || instances.some((i) => i.id === view) ? view : L.ALL_INSTANCES;
    const instanceName = (id: string) => instances.find((i) => i.id === id)?.name || id;
    const levels = L.ascending(perm);
    const steps = levels.filter((l) => l !== L.BANNED); // 금지 칸은 그리지 않는다 (금지면 아무 명령도 못 쓴다)
    const lowest = levels.find((l) => l !== L.BANNED);
    // 한 인스턴스의 메뉴: 그 인스턴스 명령만 보인다 (서버 전체 명령·도움말은 모든 인스턴스 보기에서 정한다)
    const oneInstance = instance !== L.ALL_INSTANCES;
    const groupControl = (list: L.StairChip[]) => L.groupControllers(list, t('discord.perm_server_control'));
    const chips: L.StairChip[] = oneInstance
        ? groupControl(L.stairChips(shownBoard, shownView).filter((c) => c.items.some((i) => i.instance)))
        : [
              // help·list는 금지가 아니면 누구나 쓴다 (바꿀 수 없다)
              {
                  id: 'basic',
                  title: t('discord.perm_help_list'),
                  level: lowest as number,
                  fixed: true,
                  basic: true,
                  items: [],
              },
              ...groupControl(L.stairChips(shownBoard, shownView)),
          ];
    const current = chips.find((c) => c.id === selected);
    const move = (chip: L.StairChip, level: number) => {
        // 대표 레벨이 같아도 다른 레벨인 인스턴스가 있으면 맞춘다
        if (busy || chip.fixed || level === L.BANNED || chip.items.every((i) => i.fixed || i.level === level)) return;
        draftTo(chip.items, () => level);
    };
    // 기본값으로 초기화: 보이는 명령을 프리셋 레벨로 (초안, 저장해야 반영)
    const resetToDefault = () => draftTo(chips.flatMap((c) => c.items), (it) => it.preset as number);
    dropRef.current = (id, level) => {
        const chip = chips.find((c) => c.id === id);
        if (chip && level !== L.BANNED) move(chip, level);
    };
    const setRisk = async (chip: L.StairChip, risk: string) => {
        if (busy || chip.items.every((i) => (i.confirmRisk || i.risk) === risk)) return;
        await write(() => window.api.discordSettingsSet(guildId, { commandRisks: L.riskPatch(chip.items, risk) }));
    };
    const label = (chip: L.StairChip) =>
        chip.instance ? `${chip.title} · ${instanceName(chip.instance)}` : chip.title;
    // 게임(모듈·플러그인) 명령 칩은 명령 원문, 번역 이름은 말풍선과 화면 읽기 이름으로 (사용자 지정 2026-11-07).
    // 바닐라 게임에 없는 사바쨩 기능(시작·백업 등)과 묶음·도움말 칩은 번역 이름 그대로
    const chipText = (chip: L.StairChip) => {
        const text = chip.module && chip.name ? chip.name : chip.title;
        return chip.instance ? `${text} · ${instanceName(chip.instance)}` : text;
    };
    // 계단 하나에 새로 오는 칩 수 (위 계단은 이것들을 모두 이어받는다)
    const gains: Record<number, number> = {};
    for (const c of chips.filter((c) => !c.instance)) gains[c.level] = (gains[c.level] || 0) + 1;
    let carried = 0;

    return (
        <div className="dc-tab">
            {/* 사용 모드 줄(InstanceAccess)과 같은 모양: 제목·선택은 왼쪽, 되돌리기 단추는 오른쪽 끝 */}
            <div className="access-bar">
                <span className="access-bar-title">
                    <Icon name="command" size="sm" /> {t('discord.perm_preset')}
                </span>
                <Segmented
                    ariaLabel={t('discord.perm_preset')}
                    value={preset}
                    onChange={pickPreset}
                    items={L.PRESETS.map((p) => ({ id: p, label: t(`discord.preset_${p}`) }))}
                />
                <span className="access-bar-end">
                    {L.overriddenCount(board) > 0 && (
                        <>
                            <span className="muted access-bar-note">
                                {t('discord.perm_moved_count', { count: L.overriddenCount(board) })}
                            </span>
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={busy}
                                onClick={() => pickPreset(preset)}
                            >
                                {t('discord.perm_reset_to_preset')}
                            </button>
                        </>
                    )}
                    <button type="button" className="btn btn-ghost btn-sm" disabled={busy} onClick={resetToDefault}>
                        <Icon name="undo" size="sm" /> {t('discord.perm_ladder_reset')}
                    </button>
                </span>
            </div>
            {presetDraft && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        {t('discord.perm_preset_reset', {
                            count: overrides,
                            preset: t(`discord.preset_${presetDraft}`),
                        })}
                    </span>
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={busy}
                        onClick={async () => {
                            const p = presetDraft;
                            setPresetDraft(null);
                            await update({ preset: p });
                        }}
                    >
                        {t('discord.perm_preset_apply')}
                    </button>
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => setPresetDraft(null)}>
                        {t('modals.cancel')}
                    </button>
                </div>
            )}
            <p className="dc-hint">{t('discord.perm_stairs_hint')}</p>
            {instance === L.ALL_INSTANCES && instances.length > 1 && (
                <div className="dc-field-inline">
                    <span className="muted">{t('discord.perm_view')}</span>
                    <CustomDropdown
                        className="dc-level-select"
                        value={shownView}
                        ariaLabel={t('discord.perm_view')}
                        onChange={(v) => {
                            setView(v);
                            setSelected(null);
                        }}
                        options={[
                            { value: L.ALL_INSTANCES, label: t('discord.perm_view_all') },
                            ...instances.map((i) => ({ value: i.id, label: i.name })),
                        ]}
                    />
                </div>
            )}

            <div className="dc-stairs-scroll">
                <ol
                    ref={stairsRef}
                    className="dc-stairs"
                    style={{ '--steps': steps.length } as CSSProperties}
                    aria-label={t('discord.perm_stairs')}
                >
                    {steps.map((level, i) => {
                        const here = chips.filter((c) => c.level === level);
                        const below = carried;
                        carried += gains[level] || 0;
                        const droppable = dragging !== null;
                        return (
                            <li
                                key={level}
                                className={clsx('dc-step', `level-${L.levelKey(level)}`, {
                                    'is-over': over === level && droppable,
                                })}
                                style={{ '--step': i } as CSSProperties}
                                aria-label={L.levelTitle(t, perm, level)}
                                data-drop={level}
                            >
                                <div className="dc-step-chips">
                                    {below > 0 && (
                                        <span className="dc-step-carry">
                                            {t('discord.perm_carry', {
                                                below: L.levelTitle(t, perm, steps[i - 1]),
                                                count: below,
                                            })}
                                        </span>
                                    )}
                                    {here.map((c) => (
                                        <button
                                            key={c.id}
                                            type="button"
                                            data-chip={c.id}
                                            aria-label={label(c)}
                                            data-tooltip={label(c)}
                                            className={clsx('dc-cmd-chip', `kind-${c.kind || 'basic'}`, {
                                                'is-changed': unsaved(c),
                                                'is-moved': c.overridden,
                                                'is-fixed': c.fixed,
                                                'is-selected': selected === c.id,
                                                'is-dragging': dragging === c.id,
                                            })}
                                            aria-pressed={selected === c.id}
                                            {...chipDrag.bind(c.id, !c.fixed)}
                                            onClick={() => setSelected(selected === c.id ? null : c.id)}
                                        >
                                            <span
                                                className={clsx(
                                                    'dc-dot',
                                                    c.kind === 'control'
                                                        ? 'tone-control'
                                                        : c.kind === 'admin'
                                                          ? 'tone-info'
                                                        : `tone-${RISK_TONE[c.confirmRisk as string] || 'ok'}`,
                                                )}
                                                aria-hidden="true"
                                            />
                                            {c.fixed && <Icon name="pin" size="xs" />}
                                            <span>{chipText(c)}</span>
                                        </button>
                                    ))}
                                </div>
                                <div className="dc-step-label">
                                    <span className="dc-level-num">Lv {L.levelNumber(perm, level)}</span>
                                    <strong>{L.levelLabel(t, perm, level)}</strong>
                                </div>
                            </li>
                        );
                    })}
                </ol>
            </div>

            {current && (
                <section className="dc-card dc-chip-edit" aria-label={label(current)}>
                    <div className="dc-card-head">
                        <strong>{label(current)}</strong>
                        {current.risk && (
                            <span
                                className={clsx('dc-badge', `tone-${RISK_TONE[current.confirmRisk as string] || 'ok'}`)}
                            >
                                {current.kind === 'admin'
                                    ? t('discord.kind_admin')
                                    : current.confirmMixed
                                      ? t('discord.perm_confirm_tier_mixed')
                                      : t(`discord.risk_${current.confirmRisk}`)}
                            </span>
                        )}
                    </div>
                    {!current.basic && (
                        <div className="dc-field-row">
                            <span className="dc-field-label">
                                <strong>{t('discord.perm_confirm_tier')}</strong>
                                <small>
                                    {t('discord.perm_confirm_tier_hint')}
                                    {current.confirmMixed
                                        ? ` ${t('discord.perm_confirm_tier_mixed_hint')}`
                                        : current.confirmRisk !== current.risk &&
                                          ` ${t('discord.perm_confirm_tier_default', { risk: t(`discord.risk_${current.risk}`) })}`}
                                </small>
                            </span>
                            <Segmented
                                ariaLabel={t('discord.perm_confirm_tier')}
                                value={current.confirmRisk as string}
                                onChange={(r) => setRisk(current, r)}
                                items={L.RISKS.map((r) => ({ id: r, label: t(`discord.risk_${r}`) }))}
                            />
                        </div>
                    )}
                    {current.fixed ? (
                        <p className="dc-hint">
                            {current.basic ? t('discord.perm_commands_help_hint') : t('discord.perm_fixed_hint')}
                        </p>
                    ) : (
                        <>
                            <span className="muted">{t('discord.perm_from_level')}</span>
                            <fieldset className="dc-level-steps" aria-label={t('discord.perm_from_level')}>
                                {levels
                                    .filter((l) => l !== L.BANNED)
                                    .map((l) => (
                                        <button
                                            key={l}
                                            type="button"
                                            className={clsx('dc-level-step', {
                                                'is-on': !current.levelMixed && l === current.level,
                                                'is-up': levels.indexOf(l) > levels.indexOf(current.level),
                                            })}
                                            aria-pressed={!current.levelMixed && l === current.level}
                                            disabled={busy}
                                            onClick={() => move(current, l)}
                                        >
                                            {L.levelTitle(t, perm, l)}
                                        </button>
                                    ))}
                            </fieldset>
                            {current.controllers && (
                                <p className="dc-hint">
                                    {[
                                        t('discord.perm_controllers_tag'),
                                        L.controllerLine(board)?.level === current.level && t('discord.perm_line_follow'),
                                    ]
                                        .filter(Boolean)
                                        .join(' · ')}
                                </p>
                            )}
                            {shownView !== L.ALL_INSTANCES && current.items.some((i) => !i.instance) && (
                                <p className="dc-hint">{t('discord.perm_server_wide')}</p>
                            )}
                            {shownView === L.ALL_INSTANCES && !current.instance && current.items.length > 1 && (
                                <p className="dc-hint">
                                    {t('discord.perm_applies_to', { count: current.items.length })}
                                </p>
                            )}
                            {current.items.some((i) => !i.fixed && i.level !== i.preset) && (
                                <button
                                    type="button"
                                    className="btn btn-ghost btn-sm"
                                    disabled={busy}
                                    onClick={() => draftTo(current.items, (it) => it.preset as number)}
                                >
                                    {t('discord.perm_back_to_preset')}
                                </button>
                            )}
                        </>
                    )}
                </section>
            )}

            <div className="dc-legend">
                <span>
                    <span className="dc-dot tone-ok" /> {t('discord.risk_safe')}
                </span>
                <span>
                    <span className="dc-dot tone-warn" /> {t('discord.risk_caution')}
                </span>
                <span>
                    <span className="dc-dot tone-stop" /> {t('discord.risk_danger')}
                </span>
                <span>
                    <span className="dc-dot tone-info" /> {t('discord.kind_admin')}
                </span>
            </div>

            <div className="dc-field-row">
                <span className="dc-field-label">
                    <strong>{t('discord.perm_chatbot_cap')}</strong>
                    <small>{t('discord.perm_chatbot_cap_hint')}</small>
                </span>
                <LevelSelect
                    value={typeof perm.chatbotCap === 'number' ? perm.chatbotCap : 0}
                    levels={L.grantable(perm)}
                    perm={perm}
                    label={t('discord.perm_chatbot_cap')}
                    disabled={busy}
                    onChange={(level) => update({ chatbotCap: level })}
                />
            </div>
            <SaveBar dirty={dirty} saving={busy} onRevert={() => setMoves({})} onSave={() => update({ commandLevels: patch })} />
        </div>
    );
}

interface PermissionPanelProps {
    node: DiscordNode;
    instance?: string;
}

/** 인스턴스 메뉴: 명령마다 쓸 수 있는 레벨. 멤버 레벨은 서버 전체 값이라 여기서 고치지 않는다 (멤버 탭) */
export function PermissionPanel({ node, instance = L.ALL_INSTANCES }: PermissionPanelProps) {
    const permissions = usePermissions(node.guildId);
    return (
        <div className="dc-perm">
            <CommandStairs node={node} permissions={permissions} instance={instance} />
        </div>
    );
}

interface MembersPanelProps {
    node: DiscordNode;
    members?: DiscordMember[] | null;
    membersLoading?: boolean;
    onRefreshMembers?: () => void;
}

/**
 * 디스코드 서버의 멤버 탭: 멤버마다 레벨과 레벨 편집. 멤버 레벨은 서버에 하나이고 모든 인스턴스가 이것을 따른다
 */
export function MembersPanel({ node, members, membersLoading, onRefreshMembers }: MembersPanelProps) {
    const permissions = usePermissions(node.guildId);
    const [editing, setEditing] = useState(false);
    const { perm, loading, error, load } = permissions;

    if (editing) {
        if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
        return <LadderTab node={node} permissions={permissions} onClose={() => setEditing(false)} />;
    }
    return (
        <div className="dc-perm">
            <PeopleView
                node={node}
                instance={L.ALL_INSTANCES}
                members={members}
                membersLoading={membersLoading}
                onRefreshMembers={onRefreshMembers}
                permissions={permissions}
                onEditLevels={() => setEditing(true)}
            />
        </div>
    );
}
