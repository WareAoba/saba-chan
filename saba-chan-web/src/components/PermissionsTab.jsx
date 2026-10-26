import * as L from '@gui/components/discord/levels';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { Button, Empty, errorText, failed, Loading, Section } from './ui';

/** 레벨 이름: 길드에 더한 단계는 그 이름, 기본 단계는 웹 문구 */
const levelName = (t, perm, id) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);

const nameOf = (m) => m.displayName || m.globalName || m.username || String(m.id);
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** 실제 레벨 높은 순(길드 사다리 순서, 주인 먼저, 따로 정하지 않았으면 기본 레벨), 같은 레벨은 이름순 */
export function orderMembers(members, perm) {
    const own = perm?.members || {};
    const fallback = typeof perm?.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const ladder = L.ladderOf(perm || {});
    const rankOf = (m) => {
        const id = String(m.id);
        const level = id === String(perm?.ownerDiscordId ?? '') ? 3 : Object.hasOwn(own, id) ? own[id] : fallback;
        const at = ladder.findIndex((l) => l.id === level);
        return at < 0 ? ladder.length : at;
    };
    return [...members].sort((a, b) => rankOf(a) - rankOf(b) || collator.compare(nameOf(a), nameOf(b)));
}

/** 디스코드 프로필 사진. 없거나 못 읽으면 이름 첫 글자 */
function Avatar({ member }) {
    const [broken, setBroken] = useState(false);
    const url = member.avatarUrl || member.avatar;
    return (
        <span className="avatar" aria-hidden="true">
            {typeof url === 'string' && url.startsWith('https://') && !broken ? (
                <img src={url} alt="" loading="lazy" referrerPolicy="no-referrer" onError={() => setBroken(true)} />
            ) : (
                nameOf(member).trim().charAt(0).toUpperCase() || '?'
            )}
        </span>
    );
}

function LevelSelect({ value, perm, onChange, withDefault, t, disabled, label }) {
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
                                        disabled={draft.length <= 3}
                                        aria-label={t('perm.ladder_delete', { name: label(l) })}
                                        onClick={() => setDraft((d) => L.removeLevel(d, i))}
                                    >
                                        <Icon name="close" size={16} />
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

/** 켜짐 알림 줄: 이 디스코드 서버가 쓰기로 한 서버만 (allowedInstances 가 null 이면 전부), 켠 것은 on */
export function noticeRows(servers, notice) {
    if (!notice) return [];
    const on = new Set(Array.isArray(notice.instances) ? notice.instances : []);
    const allowed = Array.isArray(notice.allowedInstances) ? new Set(notice.allowedInstances) : null;
    return servers
        .filter((s) => !allowed || allowed.has(s.id))
        .map((s) => ({ id: s.id, name: s.name, on: on.has(s.id) }));
}

/** 켜짐 알림: 사바쨩이 켠 서버를 이 디스코드 서버의 알림 채널에 알린다 (서버마다, 기본 꺼짐) */
function StartNotice({ api, guildId }) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const servers = useData((s) => s.servers);
    const serversLoaded = useData((s) => s.serversLoaded);
    // null = 읽는 중, false = 읽지 못함
    const [notice, setNotice] = useState(null);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        setNotice(null);
        api.guildStartNoticeGet(guildId).then((res) => {
            if (failed(res)) {
                toast(errorText(t, res), 'error');
                setNotice(false);
            } else setNotice(res);
        });
    }, [api, guildId, t, toast]);

    // 한 번에 하나씩: 응답의 목록이 그때의 전체라 겹치면 늦게 온 것이 앞 변경을 덮는다
    const toggle = async (id, enabled) => {
        setBusy(true);
        const res = await api.guildStartNoticeSave(id, guildId, enabled);
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        setNotice((n) => ({ ...n, instances: res.instances }));
        toast(enabled ? t('perm.notice_on') : t('perm.notice_off'), 'ok');
    };

    const rows = noticeRows(servers, notice);
    return (
        <div className="field">
            <span>{t('perm.notice')}</span>
            <small className="hint">{t('perm.notice_hint')}</small>
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
            <LadderEditor
                perm={perm}
                busy={busy}
                t={t}
                onSave={(ladder) => write(() => api.permissionsUpdate(guildId, { ladder }))}
            />
            <StartNotice api={api} guildId={guildId} />
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
                                <Avatar member={m} />
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
                        <div className="field">
                            <span>{t('perm.guild')}</span>
                            <Dropdown
                                value={guildId ?? ''}
                                onChange={setGuildId}
                                ariaLabel={t('perm.guild')}
                                options={guilds.map((g) => ({ value: g.guildId, label: g.guildName || g.guildId }))}
                            />
                        </div>
                    )}
                    {guildId && <GuildPermissions api={api} guildId={guildId} />}
                </>
            )}
        </Section>
    );
}
