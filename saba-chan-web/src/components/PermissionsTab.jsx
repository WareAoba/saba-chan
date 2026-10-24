import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Empty, errorText, failed, Loading, Section } from './ui';

/** 길드에서 줄 수 있는 레벨 (주인 3은 고정). 0 은 말하는 사바쨩만 */
const LEVELS = [2, 1, 0, -1];

function LevelSelect({ value, onChange, withDefault, t, disabled }) {
    return (
        <select
            value={value === null ? 'default' : String(value)}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value === 'default' ? null : Number(e.target.value))}
        >
            {withDefault && <option value="default">{t('perm.use_default')}</option>}
            {LEVELS.map((l) => (
                <option key={l} value={l}>
                    {t(`perm.level.${l}`)}
                </option>
            ))}
        </select>
    );
}

function GuildPermissions({ api, guildId }) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const [perm, setPerm] = useState(null);
    const [members, setMembers] = useState(null);
    const [query, setQuery] = useState('');
    const [busy, setBusy] = useState(false);

    const load = useCallback(async () => {
        const [p, m] = await Promise.all([api.permissionsGet(guildId), api.relayListNodeMembers(guildId)]);
        if (failed(p)) toast(errorText(t, p), 'error');
        else setPerm(p);
        setMembers(Array.isArray(m?.members) ? m.members : []);
    }, [api, guildId, t, toast]);

    useEffect(() => {
        setPerm(null);
        setMembers(null);
        load();
    }, [load]);

    const write = async (fn) => {
        setBusy(true);
        const res = await fn();
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        toast(t('perm.saved'), 'ok');
        load();
    };

    if (!perm || !members) return <Loading />;
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const levelOf = (id) => (Object.hasOwn(perm.members || {}, id) ? perm.members[id] : null);
    const q = query.trim().toLowerCase();
    const shown = members.filter(
        (m) =>
            !q ||
            [m.displayName, m.username, m.id].some((v) =>
                String(v ?? '')
                    .toLowerCase()
                    .includes(q),
            ),
    );

    return (
        <>
            <div className="field">
                <span>{t('perm.default')}</span>
                <LevelSelect
                    value={defaultLevel}
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
                            <li key={m.id} className="row">
                                <div className="row-main">
                                    <div className="row-title">{m.displayName || m.username || m.id}</div>
                                    {m.username && <div className="row-sub">@{m.username}</div>}
                                </div>
                                {owner ? (
                                    <span className="pill">{t('perm.owner')}</span>
                                ) : (
                                    <LevelSelect
                                        value={levelOf(String(m.id))}
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
        </>
    );
}

export function PermissionsTab({ api }) {
    const { t } = useTranslation();
    const hostId = useApp((s) => s.session.hostId);
    const [guilds, setGuilds] = useState(null);
    const [guildId, setGuildId] = useState(null);

    useEffect(() => {
        if (!hostId) return setGuilds([]);
        api.relayListHostNodes(hostId).then((res) => {
            const list = Array.isArray(res) ? res : [];
            setGuilds(list);
            setGuildId((g) => g ?? list[0]?.guildId ?? null);
        });
    }, [api, hostId]);

    return (
        <Section title={t('tabs.permissions')}>
            {guilds === null ? (
                <Loading />
            ) : guilds.length === 0 ? (
                <Empty>{t('perm.no_guilds')}</Empty>
            ) : (
                <>
                    {guilds.length > 1 && (
                        <label className="field">
                            <span>{t('perm.guild')}</span>
                            <select value={guildId ?? ''} onChange={(e) => setGuildId(e.target.value)}>
                                {guilds.map((g) => (
                                    <option key={g.guildId} value={g.guildId}>
                                        {g.guildName || g.guildId}
                                    </option>
                                ))}
                            </select>
                        </label>
                    )}
                    {guildId && <GuildPermissions api={api} guildId={guildId} />}
                </>
            )}
        </Section>
    );
}
