import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaSpinner } from '../ui/SabaUI';
import { SabaFail } from '../ui/SabaArt';
import { Segmented } from '../ui/StatusChip';
import { Avatar } from './Avatar';
import { LadderTab } from './LadderTab';
import * as L from './levels';

export const OWNER_CONFIRM_MS = 5000;

const failed = (res) => !res || res.success === false || Boolean(res.error || res.error_code);

export function permissionsErrorText(res, t) {
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
export function usePermissions(guildId) {
    const { t, i18n } = useTranslation('gui');
    const [perm, setPerm] = useState(null);
    const [board, setBoard] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    const [busy, setBusy] = useState(false);
    const locale = i18n.language;

    const load = useCallback(async () => {
        try {
            const [res, stairs] = await Promise.all([
                window.api.permissionsGet(guildId),
                window.api.permissionsCommands?.(guildId, locale),
            ]);
            if (failed(res)) setError(res || {});
            else {
                setPerm(res);
                setError(null);
            }
            // 계단을 못 읽어도 사람 화면은 쓴다
            setBoard(stairs && !failed(stairs) ? stairs : null);
        } catch (e) {
            setError({ error: e.message });
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
        async (...calls) => {
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
                    safeShowToast(e.message || t('discord.perm_err_generic'), 'error', 5000);
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

const memberName = (m) => m?.displayName || m?.globalName || m?.username;
const isId = (v) => /^\d{17,20}$/.test(v); // 디스코드 snowflake

function LevelSelect({ value, levels, perm, onChange, label, disabled }) {
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

function PermState({ loading, error, onRetry }) {
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
function gainsByLevel(board) {
    const gains = {};
    for (const chip of L.stairChips(board).filter((c) => !c.instance)) gains[chip.level] = (gains[chip.level] || 0) + 1;
    return gains;
}

/** "Lv 1의 모든 명령 + 5개" 같은 한 줄. 계단을 못 읽었으면 없다 */
function levelSummary(t, perm, board, id) {
    if (id === L.OWNER) return t('discord.perm_level_all');
    if (id === L.BANNED) return t('discord.perm_level_none');
    if (!board) return null;
    const up = L.ascending(perm);
    const at = up.indexOf(id);
    const below = up[at - 1];
    const gain = gainsByLevel(board)[id] || 0;
    // 금지 바로 위 레벨은 help·list부터 쓴다
    if (below === L.BANNED) return t('discord.perm_level_base', { count: gain + 1 });
    return t('discord.perm_level_gain', { below: L.levelTitle(t, perm, below), count: gain });
}

// ── 사람: 레벨마다 사람 칩 ──
export function PeopleView({ node, members, membersLoading, onRefreshMembers, permissions, onEditLevels }) {
    const { t } = useTranslation('gui');
    const { perm, board, loading, error, busy, load, write } = permissions;
    const guildId = node.guildId;
    const [open, setOpen] = useState(null); // 레벨을 고치는 사람 id
    const [adding, setAdding] = useState(null); // 사람을 더하는 레벨
    const [newId, setNewId] = useState('');
    const [showDefault, setShowDefault] = useState(false);
    const [picked, setPicked] = useState([]);
    const [ownerDraft, setOwnerDraft] = useState(null); // null = 편집 안 함
    const [ownerArmed, setOwnerArmed] = useState(false); // 릴레이 값은 덜 믿으므로 한 번 더 누르게 한다

    // 가져오기 후보는 릴레이에 남은 부여값(legacy-members)에서 고른다. 디스코드 멤버 목록에는 레벨이 없다
    // 새로고침 때도 다시 읽는다 (옛 레벨을 가져오지 않게). 늦게 온 옛 응답은 버린다
    const [legacy, setLegacy] = useState([]);
    const legacyGen = useRef(0);
    const loadLegacy = useCallback(async () => {
        if (!window.api?.relayListLegacyMembers) return;
        const gen = ++legacyGen.current;
        // 새 목록이 올 때까지 옛 후보와 고른 것을 비워 옛 레벨을 가져오지 못하게 한다
        setLegacy([]);
        setPicked([]);
        let res;
        try {
            res = await window.api.relayListLegacyMembers(guildId);
        } catch {
            res = null;
        }
        if (gen === legacyGen.current) setLegacy(Array.isArray(res) ? res : []);
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

    const nameOf = (id) => memberName(byId.get(id)) || id;
    const avatarOf = (id) => byId.get(id)?.avatarUrl || byId.get(id)?.avatar;
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const grantable = L.grantable(perm);
    const update = (patch) => write(() => window.api.permissionsUpdate(guildId, patch));
    const setMember = (id, level) => write(() => window.api.permissionsSetMember(guildId, id, level));
    const owner = perm.ownerDiscordId ? String(perm.ownerDiscordId) : null;
    // 데몬은 처음 받은 소유자를 고정하고, 릴레이가 다른 값을 보내면 무시한 채 알려 준다
    const relayOwner = L.ownerMismatch(perm);
    const saveOwner = async (id) => {
        if (await update({ ownerDiscordId: id })) setOwnerDraft(null);
    };
    const groups = new Map(L.membersByLevel(perm).map((g) => [g.level, g.ids]));
    const sortedOthers = L.sortByName(others, (m) => nameOf(String(m.id)));

    const chip = (id, level) => (
        <li key={id} className="dc-person">
            <button
                type="button"
                className={clsx('dc-chip', { 'is-open': open === id })}
                aria-expanded={open === id}
                aria-label={nameOf(id)}
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
    const addable = (level) =>
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
                {L.ladderOf(perm).map(({ id: level }) => {
                    const ids = L.sortByName(groups.get(level) || [], nameOf);
                    const summary = levelSummary(t, perm, board, level);
                    const isDefault = level === defaultLevel;
                    return (
                        <li
                            key={level}
                            className={clsx('dc-level-row', `level-${L.levelKey(level)}`)}
                            aria-label={L.levelLabel(t, perm, level)}
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
                                                onChange={async (id) => {
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
                                        (m) => () => window.api.permissionsSetMember(guildId, String(m.id), m.level),
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

const RISK_TONE = { safe: 'ok', caution: 'warn', danger: 'stop' };

// ── 명령: 계단. 칩은 "이 레벨부터 쓸 수 있다"는 자리에 있고, 위 계단은 아래 계단의 명령을 모두 쓴다 ──
export function CommandStairs({ node, permissions, instance = L.ALL_INSTANCES }) {
    const { t } = useTranslation('gui');
    const { perm, board, loading, error, busy, load, write } = permissions;
    const [view, setView] = useState(instance);
    const [selected, setSelected] = useState(null); // 칩 id
    const [dragging, setDragging] = useState(null);
    const [over, setOver] = useState(null);
    const [presetDraft, setPresetDraft] = useState(null); // 명령별 레벨을 덮을 프리셋 (확인 전)
    if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
    if (!board)
        return <SabaFail message={t('discord.perm_err_generic')} onRetry={load} retryLabel={t('discord.reload')} />;

    const guildId = node.guildId;
    const update = (patch) => write(() => window.api.permissionsUpdate(guildId, patch));
    const preset = L.PRESETS.includes(perm.preset) ? perm.preset : 'default';
    // 프리셋은 한 번 적용하는 틀이다: 명령별 레벨이 있으면 다시 맞춘다고 먼저 묻는다 (지금 프리셋을 다시 눌러도)
    const overrides = Object.keys(perm.commandLevels || {}).length;
    const pickPreset = (p) => {
        if (busy || (p === preset && !overrides)) return;
        if (overrides) setPresetDraft(p);
        else update({ preset: p });
    };
    const instances = board.instances || [];
    const shownView = view === L.ALL_INSTANCES || instances.some((i) => i.id === view) ? view : L.ALL_INSTANCES;
    const instanceName = (id) => instances.find((i) => i.id === id)?.name || id;
    const levels = L.ascending(perm);
    const lowest = levels.find((l) => l !== L.BANNED);
    const chips = [
        // help·list는 금지가 아니면 누구나 쓴다 (바꿀 수 없다)
        { id: 'basic', title: t('discord.perm_help_list'), level: lowest, fixed: true, basic: true, items: [] },
        ...L.stairChips(board, shownView),
    ];
    const current = chips.find((c) => c.id === selected);
    const move = async (chip, level) => {
        if (chip.fixed || level === L.BANNED || level === chip.level) return;
        await update({ commandLevels: L.levelPatch(chip.items, level) });
    };
    const label = (chip) => (chip.instance ? `${chip.title} · ${instanceName(chip.instance)}` : chip.title);
    // 계단 하나에 새로 오는 칩 수 (위 계단은 이것들을 모두 이어받는다)
    const gains = {};
    for (const c of chips.filter((c) => !c.instance)) gains[c.level] = (gains[c.level] || 0) + 1;
    let carried = 0;

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
                <ol className="dc-stairs" style={{ '--steps': levels.length }} aria-label={t('discord.perm_stairs')}>
                    {levels.map((level, i) => {
                        const here = chips.filter((c) => c.level === level);
                        const below = carried;
                        carried += gains[level] || 0;
                        const droppable = level !== L.BANNED && dragging !== null;
                        return (
                            <li
                                key={level}
                                className={clsx('dc-step', `level-${L.levelKey(level)}`, {
                                    'is-over': over === level && droppable,
                                })}
                                style={{ '--step': i }}
                                aria-label={L.levelTitle(t, perm, level)}
                                onDragOver={(e) => {
                                    if (!droppable) return;
                                    e.preventDefault();
                                    setOver(level);
                                }}
                                onDragLeave={() => setOver((o) => (o === level ? null : o))}
                                onDrop={(e) => {
                                    e.preventDefault();
                                    const chip = chips.find((c) => c.id === dragging);
                                    setDragging(null);
                                    setOver(null);
                                    if (chip && droppable) move(chip, level);
                                }}
                            >
                                <div className="dc-step-chips">
                                    {level !== L.BANNED && below > 0 && (
                                        <span className="dc-step-carry">
                                            {t('discord.perm_carry', {
                                                below: L.levelTitle(t, perm, levels[i - 1]),
                                                count: below,
                                            })}
                                        </span>
                                    )}
                                    {level === L.BANNED && (
                                        <span className="muted">{t('discord.perm_level_none')}</span>
                                    )}
                                    {here.map((c) => (
                                        <button
                                            key={c.id}
                                            type="button"
                                            className={clsx('dc-cmd-chip', `kind-${c.kind || 'basic'}`, {
                                                'is-moved': c.overridden,
                                                'is-fixed': c.fixed,
                                                'is-selected': selected === c.id,
                                            })}
                                            draggable={c.fixed ? undefined : 'true'}
                                            aria-pressed={selected === c.id}
                                            onDragStart={(e) => {
                                                e.dataTransfer?.setData('text/plain', c.id);
                                                setDragging(c.id);
                                            }}
                                            onDragEnd={() => {
                                                setDragging(null);
                                                setOver(null);
                                            }}
                                            onClick={() => setSelected(selected === c.id ? null : c.id)}
                                        >
                                            <span
                                                className={clsx(
                                                    'dc-dot',
                                                    c.kind === 'admin'
                                                        ? 'tone-info'
                                                        : `tone-${RISK_TONE[c.risk] || 'ok'}`,
                                                )}
                                                aria-hidden="true"
                                            />
                                            {c.fixed && <Icon name="pin" size="xs" />}
                                            <span>{label(c)}</span>
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
                            <span className={clsx('dc-badge', `tone-${RISK_TONE[current.risk] || 'ok'}`)}>
                                {current.kind === 'admin' ? t('discord.kind_admin') : t(`discord.risk_${current.risk}`)}
                            </span>
                        )}
                    </div>
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
                                                'is-on': l === current.level,
                                                'is-up': levels.indexOf(l) > levels.indexOf(current.level),
                                            })}
                                            aria-pressed={l === current.level}
                                            disabled={busy}
                                            onClick={() => move(current, l)}
                                        >
                                            {L.levelTitle(t, perm, l)}
                                        </button>
                                    ))}
                            </fieldset>
                            {shownView !== L.ALL_INSTANCES && current.items.some((i) => !i.instance) && (
                                <p className="dc-hint">{t('discord.perm_server_wide')}</p>
                            )}
                            {shownView === L.ALL_INSTANCES && !current.instance && current.items.length > 1 && (
                                <p className="dc-hint">
                                    {t('discord.perm_applies_to', { count: current.items.length })}
                                </p>
                            )}
                            {current.items.some((i) => i.overridden) && (
                                <button
                                    type="button"
                                    className="btn btn-ghost btn-sm"
                                    disabled={busy}
                                    onClick={() =>
                                        update({
                                            commandLevels: Object.fromEntries(
                                                current.items.flatMap((i) => (i.keys || [i.key]).map((k) => [k, null])),
                                            ),
                                        })
                                    }
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
                <span>
                    <span className="dc-cmd-chip is-moved is-legend">{t('discord.perm_legend_moved')}</span>
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
        </div>
    );
}

/** 권한 탭: 사람·명령 두 화면과 레벨 편집. 같은 권한 상태를 나눠 쓴다 (화면을 바꿔도 다시 읽지 않는다) */
export function PermissionPanel({ node, instance = L.ALL_INSTANCES, members, membersLoading, onRefreshMembers }) {
    const { t } = useTranslation('gui');
    const permissions = usePermissions(node.guildId);
    const [view, setView] = useState('commands');
    const [editing, setEditing] = useState(false);
    const { perm, loading, error, load } = permissions;

    if (editing) {
        if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
        return <LadderTab node={node} permissions={permissions} onClose={() => setEditing(false)} />;
    }
    return (
        <div className="dc-perm">
            <Segmented
                ariaLabel={t('discord.tab_permissions')}
                value={view}
                onChange={setView}
                items={[
                    {
                        id: 'commands',
                        label: t('discord.perm_view_commands'),
                        icon: <Icon name="terminal" size="sm" />,
                    },
                    { id: 'people', label: t('discord.perm_view_people'), icon: <Icon name="user" size="sm" /> },
                ]}
            />
            {view === 'commands' ? (
                <CommandStairs node={node} permissions={permissions} instance={instance} />
            ) : (
                <PeopleView
                    node={node}
                    members={members}
                    membersLoading={membersLoading}
                    onRefreshMembers={onRefreshMembers}
                    permissions={permissions}
                    onEditLevels={() => setEditing(true)}
                />
            )}
        </div>
    );
}
