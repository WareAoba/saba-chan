import * as L from '@gui/components/discord/levels';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Button, Empty, errorText, failed, Loading, Section } from './ui';

/** 레벨 이름: 길드에 더한 단계는 그 이름, 기본 단계는 웹 문구 */
const levelName = (t, perm, id) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);

function LevelSelect({ value, perm, onChange, withDefault, t, disabled }) {
    return (
        <select
            value={value === null ? 'default' : String(value)}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value === 'default' ? null : Number(e.target.value))}
        >
            {withDefault && <option value="default">{t('perm.use_default')}</option>}
            {L.grantable(perm).map((l) => (
                <option key={l} value={l}>
                    {levelName(t, perm, l)}
                </option>
            ))}
        </select>
    );
}

/** 단계 사다리 (위가 높은 단계). 휴대폰이라 끌기 대신 위·아래 버튼으로 옮긴다. 소유자·차단은 고정 */
function LadderEditor({ perm, busy, onSave, t }) {
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const [name, setName] = useState('');
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    const patch = L.ladderPatch(draft);
    const changed = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const label = (l) => (L.isCustomLevel(l) ? l.name : levelName(t, perm, l.id));
    const last = draft.length - 1;
    return (
        <div className="field">
            <span>{t('perm.ladder')}</span>
            <small className="hint">{t('perm.ladder_hint')}</small>
            <ul className="rows">
                {draft.map((l, i) => {
                    const fixed = i === 0 || i === last;
                    const custom = L.isCustomLevel(l);
                    return (
                        <li key={l.id ?? l.key} className="row" aria-label={label(l)}>
                            <div className="row-main">
                                {custom ? (
                                    <input
                                        value={l.name}
                                        maxLength={L.MAX_NAME}
                                        aria-label={t('perm.ladder_name')}
                                        onChange={(e) => setDraft((d) => L.renameLevel(d, i, e.target.value))}
                                    />
                                ) : (
                                    <div className="row-title">{label(l)}</div>
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
                                        ↑
                                    </Button>
                                    <Button
                                        disabled={i >= last - 1}
                                        aria-label={t('perm.ladder_down', { name: label(l) })}
                                        onClick={() => setDraft((d) => L.moveLevel(d, i, i + 1))}
                                    >
                                        ↓
                                    </Button>
                                    <Button
                                        tone="ghost-danger"
                                        disabled={draft.length <= 3}
                                        aria-label={t('perm.ladder_delete', { name: label(l) })}
                                        onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                    >
                                        ✕
                                    </Button>
                                </>
                            )}
                        </li>
                    );
                })}
            </ul>
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
                <Button disabled={busy || !changed} onClick={() => setDraft(saved)}>
                    {t('perm.ladder_revert')}
                </Button>
                <Button tone="accent" busy={busy} disabled={!changed || !valid} onClick={() => onSave(patch)}>
                    {t('perm.ladder_save')}
                </Button>
            </div>
        </div>
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
                    perm={perm}
                    t={t}
                    disabled={busy}
                    onChange={(l) => l !== null && write(() => api.permissionsUpdate(guildId, { defaultLevel: l }))}
                />
                <small className="hint">{t('perm.default_hint')}</small>
            </div>
            <LadderEditor
                perm={perm}
                busy={busy}
                t={t}
                onSave={(ladder) => write(() => api.permissionsUpdate(guildId, { ladder }))}
            />
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
                                        perm={perm}
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
