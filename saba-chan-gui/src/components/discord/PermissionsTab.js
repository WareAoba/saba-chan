import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaSpinner } from '../ui/SabaUI';
import { SabaFail } from '../ui/SabaArt';
import { Segmented } from '../ui/StatusChip';
import { LadderTab } from './LadderTab';
import * as L from './levels';

/** 데몬 P1 권한 API가 preload에 있는지 (없으면 예전 명령별 차단 화면을 쓴다) */
export const permissionsAvailable = () =>
    Boolean(
        window.api?.permissionsGet &&
            window.api?.permissionsUpdate &&
            window.api?.permissionsSetMember &&
            window.api?.permissionsRemoveMember,
    );

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

/** 길드 하나의 권한: 읽기와, 쓴 뒤 다시 읽기 */
export function usePermissions(guildId) {
    const { t } = useTranslation('gui');
    const [perm, setPerm] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);
    const [busy, setBusy] = useState(false);

    const load = useCallback(async () => {
        try {
            const res = await window.api.permissionsGet(guildId);
            if (failed(res)) setError(res || {});
            else {
                setPerm(res);
                setError(null);
            }
        } catch (e) {
            setError({ error: e.message });
        }
        setLoading(false);
    }, [guildId]);

    useEffect(() => {
        setLoading(true);
        setPerm(null);
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

    return { perm, loading, error, busy, load, write };
}

const memberName = (m) => m?.displayName || m?.globalName || m?.username;

function LevelSelect({ value, levels, perm, onChange, label, disabled }) {
    const { t } = useTranslation('gui');
    return (
        <CustomDropdown
            className="dc-level-select"
            value={String(value)}
            ariaLabel={label}
            disabled={disabled}
            onChange={(v) => onChange(Number(v))}
            options={levels.map((l) => ({ value: String(l), label: L.levelLabel(t, perm, l) }))}
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

// ── 멤버: 길드 기본값 + 레벨별 멤버 ──
export function MemberLevelsTab({ node, members, membersLoading, onRefreshMembers, permissions }) {
    const { t } = useTranslation('gui');
    const { perm, loading, error, busy, load, write } = permissions;
    const guildId = node.guildId;
    const [query, setQuery] = useState('');
    const [newId, setNewId] = useState('');
    const [newLevel, setNewLevel] = useState(2);
    const [picked, setPicked] = useState([]);
    const [ownerDraft, setOwnerDraft] = useState(null); // null = 편집 안 함
    const [ownerArmed, setOwnerArmed] = useState(false); // 릴레이 값은 덜 믿으므로 한 번 더 누르게 한다
    const [presetDraft, setPresetDraft] = useState(null); // 명령별 레벨을 덮을 프리셋 (확인 전)

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
    const q = query.trim().toLowerCase();
    const matches = (id) => !q || `${nameOf(id)} ${byId.get(id)?.username || ''} ${id}`.toLowerCase().includes(q);
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    // 디스코드 멤버 중 레벨을 따로 받지 않은 사람: 여기서 바로 레벨을 줄 수 있다
    const shownOthers = others.filter((m) => matches(String(m.id)));
    const preset = L.PRESETS.includes(perm.preset) ? perm.preset : 'default';
    const levelName = (l) => L.levelLabel(t, perm, l);
    const grantable = L.grantable(perm);
    // 고른 단계를 지웠으면 맨 위 단계로
    const addLevel = grantable.includes(newLevel) ? newLevel : grantable[0];
    const update = (patch) => write(() => window.api.permissionsUpdate(guildId, patch));
    // 프리셋은 한 번 적용하는 틀이다: 명령별 레벨이 있으면 다시 맞춘다고 먼저 묻는다
    const overrides = Object.keys(perm.commandLevels || {}).length;
    const pickPreset = (p) => {
        if (busy || p === preset) return;
        if (overrides) setPresetDraft(p);
        else update({ preset: p });
    };
    const setMember = (id, level) => write(() => window.api.permissionsSetMember(guildId, id, level));
    const trimmedId = newId.trim();
    const owner = perm.ownerDiscordId ? String(perm.ownerDiscordId) : null;
    // 데몬은 처음 받은 소유자를 고정하고, 릴레이가 다른 값을 보내면 무시한 채 알려 준다
    const relayOwner = L.ownerMismatch(perm);
    const isId = (v) => /^\d{17,20}$/.test(v); // 디스코드 snowflake
    const saveOwner = async (id) => {
        if (await update({ ownerDiscordId: id })) setOwnerDraft(null);
    };

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

            <section className="dc-card">
                <h3>{t('discord.perm_defaults')}</h3>
                <div className="dc-field-row">
                    <span className="dc-field-label">
                        <strong>{t('discord.perm_owner')}</strong>
                        <small>{t('discord.perm_owner_hint')}</small>
                    </span>
                    {ownerDraft === null ? (
                        <span className="dc-owner">
                            <span>{owner ? nameOf(owner) : t('discord.perm_owner_none')}</span>
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
                                disabled={busy || !isId(ownerDraft.trim()) || ownerDraft.trim() === owner}
                                onClick={() => saveOwner(ownerDraft.trim())}
                            >
                                {t('discord.perm_owner_save')}
                            </button>
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setOwnerDraft(null)}>
                                {t('modals.cancel')}
                            </button>
                        </span>
                    )}
                </div>
                <div className="dc-field-row">
                    <span className="dc-field-label">
                        <strong>{t('discord.perm_default_level')}</strong>
                        <small>{t('discord.perm_default_level_hint')}</small>
                    </span>
                    <LevelSelect
                        value={defaultLevel}
                        levels={grantable}
                        perm={perm}
                        label={t('discord.perm_default_level')}
                        disabled={busy}
                        onChange={(level) => update({ defaultLevel: level })}
                    />
                </div>
                <div className="dc-field-row">
                    <span className="dc-field-label">
                        <strong>{t('discord.perm_preset')}</strong>
                        <small>
                            {t('discord.perm_preset_hint', {
                                safe: levelName(L.presetLevel(perm, 'safe')),
                                caution: levelName(L.presetLevel(perm, 'caution')),
                                danger: levelName(L.presetLevel(perm, 'danger')),
                            })}
                        </small>
                    </span>
                    <Segmented
                        ariaLabel={t('discord.perm_preset')}
                        value={preset}
                        onChange={pickPreset}
                        items={L.PRESETS.map((p) => ({ id: p, label: t(`discord.preset_${p}`) }))}
                    />
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
                <div className="dc-field-row">
                    <span className="dc-field-label">
                        <strong>{t('discord.perm_chatbot_cap')}</strong>
                        <small>{t('discord.perm_chatbot_cap_hint')}</small>
                    </span>
                    <LevelSelect
                        value={typeof perm.chatbotCap === 'number' ? perm.chatbotCap : 0}
                        levels={grantable}
                        perm={perm}
                        label={t('discord.perm_chatbot_cap')}
                        disabled={busy}
                        onChange={(level) => update({ chatbotCap: level })}
                    />
                </div>
            </section>

            <div className="dc-toolbar">
                <input
                    type="search"
                    className="input"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder={t('discord.members_search')}
                    aria-label={t('discord.members_search')}
                />
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
            </div>

            {L.membersByLevel(perm).map(({ level, ids }) => {
                const shown = ids.filter(matches);
                if (!shown.length) return null;
                return (
                    <section key={level} className="dc-level-group" aria-label={levelName(level)}>
                        <h4 className={clsx('dc-level-title', `level-${L.levelKey(level)}`)}>
                            {levelName(level)} <span className="muted">{ids.length}</span>
                        </h4>
                        <ul className="dc-list">
                            {shown.map((id) => (
                                <li key={id} className="dc-list-row">
                                    <span className="dc-list-text">
                                        <span className="dc-list-name">{nameOf(id)}</span>
                                        {nameOf(id) !== id && (
                                            <span className="dc-list-sub">
                                                {byId.get(id)?.username ? `@${byId.get(id).username}` : id}
                                            </span>
                                        )}
                                    </span>
                                    {level === L.OWNER ? (
                                        <span className="dc-badge tone-info">{t('discord.perm_owner_fixed')}</span>
                                    ) : (
                                        <>
                                            <LevelSelect
                                                value={level}
                                                levels={grantable}
                                                perm={perm}
                                                label={t('discord.perm_member_level', { name: nameOf(id) })}
                                                disabled={busy}
                                                onChange={(l) => setMember(id, l)}
                                            />
                                            <button
                                                type="button"
                                                className="btn btn-ghost btn-sm"
                                                disabled={busy}
                                                title={t('discord.perm_reset_member')}
                                                aria-label={t('discord.perm_reset_member_named', {
                                                    name: nameOf(id),
                                                })}
                                                onClick={() =>
                                                    write(() => window.api.permissionsRemoveMember(guildId, id))
                                                }
                                            >
                                                <Icon name="x" size="sm" />
                                            </button>
                                        </>
                                    )}
                                </li>
                            ))}
                        </ul>
                    </section>
                );
            })}
            {membersLoading && (
                <div className="dc-pair-wait pad">
                    <SabaSpinner size="sm" /> {t('discord.members_loading')}
                </div>
            )}
            {!membersLoading && !members?.length && <p className="muted pad">{t('discord.members_empty')}</p>}
            {shownOthers.length > 0 && (
                <section className="dc-level-group" aria-label={t('discord.perm_default_members')}>
                    <h4 className={clsx('dc-level-title', `level-${L.levelKey(defaultLevel)}`)}>
                        {t('discord.perm_default_members')} <span className="muted">{others.length}</span>
                    </h4>
                    <ul className="dc-list">
                        {shownOthers.map((m) => {
                            const id = String(m.id);
                            return (
                                <li key={id} className="dc-list-row">
                                    <span className="dc-list-text">
                                        <span className="dc-list-name">{nameOf(id)}</span>
                                        {m.username && <span className="dc-list-sub">@{m.username}</span>}
                                    </span>
                                    <LevelSelect
                                        value={defaultLevel}
                                        levels={grantable}
                                        perm={perm}
                                        label={t('discord.perm_member_level', { name: nameOf(id) })}
                                        disabled={busy}
                                        onChange={(l) => l !== defaultLevel && setMember(id, l)}
                                    />
                                </li>
                            );
                        })}
                    </ul>
                </section>
            )}
            <p className="dc-hint">{t('discord.perm_others', { level: levelName(defaultLevel) })}</p>

            <section className="dc-card">
                <h3>{t('discord.perm_add')}</h3>
                <div className="dc-toolbar">
                    <input
                        className="input"
                        value={newId}
                        inputMode="numeric"
                        onChange={(e) => setNewId(e.target.value)}
                        placeholder={t('discord.perm_add_id')}
                        aria-label={t('discord.perm_add_id')}
                    />
                    <LevelSelect
                        value={addLevel}
                        levels={grantable}
                        perm={perm}
                        label={t('discord.perm_add_level')}
                        onChange={setNewLevel}
                    />
                    <button
                        type="button"
                        className="btn btn-primary"
                        disabled={busy || !isId(trimmedId)}
                        onClick={async () => {
                            if (await setMember(trimmedId, addLevel)) setNewId('');
                        }}
                    >
                        {t('discord.perm_add_button')}
                    </button>
                </div>
            </section>

            {candidates.length > 0 && (
                <section className="dc-card">
                    <h3>{t('discord.perm_import')}</h3>
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
                                                        ? levelName(now)
                                                        : t('discord.perm_import_default'),
                                                to: levelName(m.level),
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
                </section>
            )}
        </div>
    );
}

// ── 명령: 명령마다 실제 요구 레벨 (프리셋 값이거나 따로 고른 값) ──
export function CommandLevelsTab({ node, servers, commandsFor, riskOf, permissions }) {
    const { t } = useTranslation('gui');
    const { perm, loading, error, busy, load, write } = permissions;
    if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;

    const setLevel = (key, level) =>
        write(() => window.api.permissionsUpdate(node.guildId, { commandLevels: { [key]: level } }));

    return (
        <div className="dc-tab">
            <p className="dc-hint">{t('discord.perm_commands_hint')}</p>
            <p className="dc-hint">{t('discord.perm_commands_help_hint')}</p>
            {servers.length === 0 && <p className="muted pad">{t('discord.member_no_instances')}</p>}
            {servers.map((srv) => {
                const cmds = [
                    ...L.BUILTIN_COMMANDS.map((c) => ({
                        name: c.name,
                        label: t(`discord.builtin_${c.name}`),
                        risk: c.risk,
                    })),
                    ...commandsFor(srv.module)
                        .filter((c) => !L.BUILTIN_COMMANDS.some((b) => b.name === c.name))
                        .map((c) => ({ ...c, risk: L.normalizeRisk(riskOf(srv.module, c.name)) })),
                ];
                return (
                    <section key={srv.id} className="dc-card dc-perm-card">
                        <div className="dc-card-head">
                            <div className="dc-list-text">
                                <span className="dc-list-name">{srv.name}</span>
                                <span className="dc-list-sub">{srv.module}</span>
                            </div>
                        </div>
                        <ul className="dc-list">
                            {cmds.map((c) => {
                                const key = L.commandKey(srv.id, c.name);
                                const { level, overridden } = L.requiredLevel(perm, srv.id, c.name, c.risk);
                                return (
                                    <li key={c.name} className={clsx('dc-list-row', { 'dc-overridden': overridden })}>
                                        <span className="dc-list-text">
                                            <span className="dc-list-name">{c.label}</span>
                                            {c.description && <span className="dc-list-sub">{c.description}</span>}
                                        </span>
                                        <span
                                            className={clsx(
                                                'dc-badge',
                                                c.risk === 'danger'
                                                    ? 'tone-stop'
                                                    : c.risk === 'caution'
                                                      ? 'tone-warn'
                                                      : 'tone-ok',
                                            )}
                                        >
                                            {t(`discord.risk_${c.risk}`)}
                                        </span>
                                        <LevelSelect
                                            value={level}
                                            levels={L.requirable(perm)}
                                            perm={perm}
                                            label={t('discord.perm_command_level', { command: c.label })}
                                            disabled={busy}
                                            onChange={(l) => l !== level && setLevel(key, l)}
                                        />
                                    </li>
                                );
                            })}
                        </ul>
                    </section>
                );
            })}
        </div>
    );
}

/** 멤버·명령·단계 탭이 같은 권한 상태를 나눠 쓴다 (탭을 바꿔도 다시 읽지 않는다) */
export function PermissionPanel({ tab, node, ...rest }) {
    const permissions = usePermissions(node.guildId);
    if (tab === 'commands') return <CommandLevelsTab node={node} permissions={permissions} {...rest} />;
    if (tab === 'levels') {
        const { perm, loading, error, load } = permissions;
        if (!perm) return <PermState loading={loading} error={error} onRetry={load} />;
        return <LadderTab node={node} permissions={permissions} />;
    }
    return <MemberLevelsTab node={node} permissions={permissions} {...rest} />;
}
