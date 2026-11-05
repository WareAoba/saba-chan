import clsx from 'clsx';
import type { TFunction } from 'i18next';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaSpinner } from '../ui/SabaUI';
import { SabaFail } from '../ui/SabaArt';
import { Segmented } from '../ui/StatusChip';
import { Avatar } from './Avatar';
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
    const [newId, setNewId] = useState('');
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
                <Avatar name={nameOf(id)} url={avatarOf(id)} size={20} />
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

            <div className="dc-toolbar">
                <span className="dc-field-inline">
                    <span className="muted">{t('discord.perm_default_level_hint')}</span>
                    <LevelSelect
                        value={defaultLevel}
                        levels={grantable}
                        perm={perm}
                        label={t('discord.perm_default_level')}
                        disabled={busy}
                        onChange={(level) => update({ defaultLevel: level })}
                    />
                </span>
                <span className="dc-mini-actions">
                    <button
                        type="button"
                        className="btn btn-secondary"
                        onClick={() => {
                            load();
                            loadLegacy();
                            onRefreshMembers?.();
                        }}
                        disabled={busy || membersLoading}
                    >
                        <Icon name="refresh" size="sm" /> {t('discord.reload')}
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
                                                <span className="dc-chip is-static">
                                                    <Avatar name={nameOf(owner)} url={avatarOf(owner)} size={20} />
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
                                                className="dc-chip is-add"
                                                aria-expanded={adding === level}
                                                aria-label={t('discord.perm_add_person_to', {
                                                    level: L.levelLabel(t, perm, level),
                                                })}
                                                onClick={() => {
                                                    setAdding(adding === level ? null : level);
                                                    setNewId('');
                                                }}
                                            >
                                                <Icon name="plus" size="xs" /> {t('discord.perm_add_person')}
                                            </button>
                                        </li>
                                    </ul>
                                )}
                                {adding === level && (
                                    <div className="dc-toolbar dc-add-person">
                                        {addable(level).length > 0 && (
                                            <CustomDropdown
                                                className="dc-member-pick"
                                                value=""
                                                placeholder={t('discord.perm_pick_member')}
                                                ariaLabel={t('discord.perm_pick_member')}
                                                disabled={busy}
                                                onChange={async (id: string) => {
                                                    if (await setMember(String(id), level)) setAdding(null);
                                                }}
                                                options={addable(level).map((m) => ({
                                                    value: String(m.id),
                                                    label: nameOf(String(m.id)),
                                                }))}
                                            />
                                        )}
                                        <input
                                            className="input"
                                            value={newId}
                                            inputMode="numeric"
                                            onChange={(e) => setNewId(e.target.value)}
                                            placeholder={t('discord.perm_add_id')}
                                            aria-label={t('discord.perm_add_id')}
                                        />
                                        <button
                                            type="button"
                                            className="btn btn-primary"
                                            disabled={busy || !isId(newId.trim())}
                                            onClick={async () => {
                                                if (await setMember(newId.trim(), level)) setAdding(null);
                                            }}
                                        >
                                            {t('discord.perm_add_button')}
                                        </button>
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

// ── 명령: 목록. 명령마다 "누구부터 쓸 수 있나"와 디스코드 되묻기를 그 줄에서 고른다 (위 레벨은 아래 레벨의 명령을 모두 쓴다) ──
/** 명령 묶음: 조회 · 게임 조작 · 관리 (도움말·목록은 조회) */
const GROUPS = ['query', 'command', 'admin'] as const;
const groupOf = (chip: L.StairChip) => (chip.kind === 'admin' ? 'admin' : chip.kind === 'command' ? 'command' : 'query');

interface CommandListProps {
    node: DiscordNode;
    permissions: PermissionsState;
    instance?: string;
}

export function CommandList({ node, permissions, instance = L.ALL_INSTANCES }: CommandListProps) {
    const { t } = useTranslation('gui');
    const { perm, board, loading, error, busy, load, write } = permissions;
    const [view, setView] = useState(instance);
    const [query, setQuery] = useState('');
    const [presetDraft, setPresetDraft] = useState<string | null>(null); // 명령별 레벨을 덮을 프리셋 (확인 전)
    if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
    if (!board)
        return <SabaFail message={t('discord.perm_err_generic')} onRetry={load} retryLabel={t('discord.reload')} />;

    const guildId = node.guildId;
    const update = (patch: ApiBody) => write(() => window.api.permissionsUpdate(guildId, patch));
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
    const usable = levels.filter((l) => l !== L.BANNED);
    const lowest = usable[0];
    const top = usable[usable.length - 1];
    const chips: L.StairChip[] = [
        // help·list는 금지가 아니면 누구나 쓴다 (바꿀 수 없다)
        { id: 'basic', title: t('discord.perm_help_list'), level: lowest, fixed: true, basic: true, items: [] },
        ...L.stairChips(board, shownView),
    ];
    const move = async (chip: L.StairChip, level: number) => {
        // 대표 레벨이 같아도 다른 레벨인 인스턴스가 있으면 맞춘다
        if (busy || chip.fixed || level === L.BANNED || chip.items.every((i) => i.fixed || i.level === level)) return;
        await update({ commandLevels: L.levelPatch(chip.items, level) });
    };
    const setRisk = async (chip: L.StairChip, risk: string) => {
        if (busy || chip.items.every((i) => (i.confirmRisk || i.risk) === risk)) return;
        await write(() => window.api.discordSettingsSet(guildId, { commandRisks: L.riskPatch(chip.items, risk) }));
    };
    const label = (chip: L.StairChip) =>
        chip.instance ? `${chip.title} · ${instanceName(chip.instance)}` : chip.title || '';
    const fromText = (l: number) =>
        t(l === top ? 'discord.perm_only' : 'discord.perm_from', { level: L.levelLabel(t, perm, l) });
    // 요약: 레벨마다 쓸 수 있는 명령 수 (아래 레벨 것을 이어받는다). 인스턴스 꼬리표 줄은 세지 않는다
    const counted = chips.filter((c) => !c.instance);
    const summary = usable
        .map((l) => `${L.levelLabel(t, perm, l)} ${counted.filter((c) => levels.indexOf(c.level) <= levels.indexOf(l)).length}`)
        .join(' · ');
    const q = query.trim().toLowerCase();
    const shown = q ? chips.filter((c) => label(c).toLowerCase().includes(q)) : chips;

    const row = (c: L.StairChip) => {
        const name = label(c);
        const tone = c.kind === 'admin' ? 'info' : RISK_TONE[c.confirmRisk as string] || 'ok';
        return (
            <li key={c.id} className={clsx('dc-cmd-row', { 'is-moved': c.overridden })}>
                <span className="dc-cmd-name">
                    <span className={clsx('dc-dot', `tone-${tone}`)} aria-hidden="true" />
                    {c.fixed && <Icon name="pin" size="xs" />}
                    <span className="dc-cmd-title">
                        <strong>{name}</strong>
                        <small className="muted">
                            {[
                                c.fixed && (c.basic ? t('discord.perm_commands_help_hint') : t('discord.perm_fixed_hint')),
                                !c.fixed && c.overridden && t('discord.perm_moved'),
                                shownView !== L.ALL_INSTANCES &&
                                    c.items.some((i) => !i.instance) &&
                                    t('discord.perm_server_wide_tag'),
                                shownView === L.ALL_INSTANCES &&
                                    !c.instance &&
                                    c.items.length > 1 &&
                                    t('discord.perm_instances_count', { count: c.items.length }),
                            ]
                                .filter(Boolean)
                                .join(' · ')}
                        </small>
                    </span>
                </span>
                <CustomDropdown
                    className="dc-level-select"
                    value={c.levelMixed ? '' : String(c.level)}
                    ariaLabel={t('discord.perm_who_for', { command: name })}
                    disabled={busy || c.fixed}
                    placeholder={t('discord.perm_confirm_tier_mixed')}
                    onChange={(v) => move(c, Number(v))}
                    options={usable.map((l) => ({ value: String(l), label: fromText(l) }))}
                />
                {c.basic ? (
                    <span />
                ) : (
                    <CustomDropdown
                        className="dc-risk-select"
                        value={c.confirmMixed ? '' : (c.confirmRisk as string)}
                        ariaLabel={t('discord.perm_risk_for', { command: name })}
                        disabled={busy}
                        placeholder={t('discord.perm_confirm_tier_mixed')}
                        onChange={(r) => setRisk(c, r)}
                        options={L.RISKS.map((r) => ({
                            value: r,
                            label:
                                r === c.risk || c.kind === 'admin'
                                    ? t(`discord.risk_${r}`)
                                    : `${t(`discord.risk_${r}`)} ${t('discord.perm_confirm_tier_default', { risk: t(`discord.risk_${c.risk}`) })}`,
                        }))}
                    />
                )}
                {c.overridden && !c.fixed ? (
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={busy}
                        onClick={() =>
                            update({
                                commandLevels: Object.fromEntries(
                                    c.items.flatMap((i) => (i.keys || [i.key]).map((k) => [k, null])),
                                ),
                            })
                        }
                    >
                        {t('discord.perm_back_to_preset')}
                    </button>
                ) : (
                    <span />
                )}
            </li>
        );
    };

    return (
        <div className="dc-tab">
            <div className="dc-toolbar">
                <span className="dc-field-inline">
                    <strong>{t('discord.perm_preset')}</strong>
                    <Segmented
                        ariaLabel={t('discord.perm_preset')}
                        value={preset}
                        onChange={pickPreset}
                        items={L.PRESETS.map((p) => ({ id: p, label: t(`discord.preset_${p}`) }))}
                    />
                </span>
                {L.overriddenCount(board) > 0 && (
                    <span className="muted">
                        {t('discord.perm_moved_count', { count: L.overriddenCount(board) })}{' '}
                        <button
                            type="button"
                            className="btn btn-ghost btn-sm"
                            disabled={busy}
                            onClick={() => pickPreset(preset)}
                        >
                            {t('discord.perm_reset_to_preset')}
                        </button>
                    </span>
                )}
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
            <p className="dc-hint">{t('discord.perm_list_hint')}</p>
            <div className="dc-cmd-tools">
                <input
                    className="input dc-cmd-search"
                    type="search"
                    value={query}
                    placeholder={t('discord.perm_search')}
                    aria-label={t('discord.perm_search')}
                    onChange={(e) => setQuery(e.target.value)}
                />
                {instance === L.ALL_INSTANCES && instances.length > 1 && (
                    <span className="dc-field-inline">
                        <span className="muted">{t('discord.perm_view')}</span>
                        <CustomDropdown
                            className="dc-level-select"
                            value={shownView}
                            ariaLabel={t('discord.perm_view')}
                            onChange={setView}
                            options={[
                                { value: L.ALL_INSTANCES, label: t('discord.perm_view_all') },
                                ...instances.map((i) => ({ value: i.id, label: i.name })),
                            ]}
                        />
                    </span>
                )}
            </div>
            <p className="dc-cmd-summary muted">{summary}</p>
            {GROUPS.map((g) => {
                const list = shown.filter((c) => groupOf(c) === g);
                if (!list.length) return null;
                return (
                    <section key={g} className="dc-cmd-group" aria-label={t(`discord.perm_group_${g}`)}>
                        <h3 className="dc-cmd-group-title">{t(`discord.perm_group_${g}`)}</h3>
                        <ul className="dc-cmd-list">{list.map(row)}</ul>
                    </section>
                );
            })}
            {!shown.length && <p className="muted">{t('discord.perm_search_none')}</p>}

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
            <CommandList node={node} permissions={permissions} instance={instance} />
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
