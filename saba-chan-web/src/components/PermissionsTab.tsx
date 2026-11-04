import * as L from '@gui/components/discord/levels';
import * as P from '@gui/components/discord/nodePermissions';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { PermissionStairs } from './PermissionStairs';
import { Button, Empty, errorText, failed, Loading, Segments } from './ui';
import type { Loose } from '../types';

/** 레벨 이름: 길드에 더한 단계는 그 이름, 기본 단계는 웹 문구 */
const levelName = (t: Loose, perm: Loose, id: Loose) => L.ladderOf(perm).find((l) => l.id === id)?.name || t(`perm.level.${id}`);

const nameOf = (m: Loose) => m.displayName || m.globalName || m.username || String(m.id);
const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

/** 실제 레벨 높은 순(길드 사다리 순서, 주인 먼저, 따로 정하지 않았으면 기본 레벨), 같은 레벨은 이름순 */
export function orderMembers(members: Loose, perm: Loose) {
    const own = perm?.members || {};
    const fallback = typeof perm?.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const ladder = L.ladderOf(perm || {});
    const rankOf = (m: Loose) => {
        const id = String(m.id);
        const level = id === String(perm?.ownerDiscordId ?? '') ? 3 : Object.hasOwn(own, id) ? own[id] : fallback;
        const at = ladder.findIndex((l) => l.id === level);
        return at < 0 ? ladder.length : at;
    };
    return [...members].sort((a, b) => rankOf(a) - rankOf(b) || collator.compare(nameOf(a), nameOf(b)));
}

/** 디스코드 프로필 사진. 없거나 못 읽으면 이름 첫 글자 */
function Avatar({ member }: Loose) {
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

function LevelSelect({ value, perm, onChange, withDefault, t, disabled, label }: Loose) {
    return (
        <Dropdown
            className="level-select"
            value={value === null ? 'default' : String(value)}
            disabled={disabled}
            ariaLabel={label}
            onChange={(v: Loose) => onChange(v === 'default' ? null : Number(v))}
            options={[
                ...(withDefault ? [{ value: 'default', label: t('perm.use_default') }] : []),
                ...L.grantable(perm).map((l) => ({ value: String(l), label: levelName(t, perm, l) })),
            ]}
        />
    );
}

/** 단계 사다리 (위가 높은 단계). 휴대폰이라 끌기 대신 위·아래 버튼으로 옮긴다. 소유자·차단은 고정 */
function LadderEditor({ perm, busy, onSave, t }: Loose) {
    const saved = L.ladderOf(perm);
    const [draft, setDraft] = useState(saved);
    const [name, setName] = useState('');
    useEffect(() => setDraft(L.ladderOf(perm)), [perm]);
    const patch = L.ladderPatch(draft);
    const changed = JSON.stringify(patch) !== JSON.stringify(L.ladderPatch(saved));
    const removed = saved.some((s) => !draft.some((d) => d.id === s.id));
    const valid = L.ladderValid(draft);
    const label = (l: Loose) => (L.isCustomLevel(l) ? l.name : levelName(t, perm, l.id));
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
export function noticeRows(servers: Loose, notice: Loose) {
    if (!notice) return [];
    const on = new Set(Array.isArray(notice.instances) ? notice.instances : []);
    const allowed = Array.isArray(notice.allowedInstances) ? new Set(notice.allowedInstances) : null;
    return servers
        .filter((s: Loose) => !allowed || allowed.has(s.id))
        .map((s: Loose) => ({ id: s.id, name: s.name, on: on.has(s.id) }));
}

/**
 * 이 디스코드 서버에서 보일 서버 (2026-11-03 사용자 지정 시안, GUI 노드의 인스턴스 탭과 같은 규칙):
 * 모두 보이기(제한 없음, 새 서버도 바로 보인다) 또는 고른 것만. 서버 하나는 디스코드 서버 하나만 고를 수 있다.
 * `allowed`: 이 길드의 allowedInstances (null 이면 제한 없음), `others`: 이 PC 의 다른 길드 {guildId: allowedInstances}
 */
export function VisibleServers({ api, guildId, allowed, others, onSaved }: Loose) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const toast = useApp((s) => s.toast);
    const servers = useData((s) => s.servers);
    const [busy, setBusy] = useState(false);
    const settings: Record<string, { allowedInstances: Loose }> = {
        ...Object.fromEntries(Object.entries(others || {}).map(([g, list]) => [g, { allowedInstances: list }])),
    };
    if (Array.isArray(allowed)) settings[guildId] = { allowedInstances: allowed };
    const nodeIds = [guildId, ...Object.keys(others || {})];
    const mode = P.instanceMode(settings, guildId);
    const ownerOf = (id: Loose) => P.instanceOwner(settings, nodeIds, id, guildId);

    const save = async (next: Loose) => {
        const list = P.nodeConfig(next, guildId).allowedInstances;
        setBusy(true);
        const res = await api.guildAllowedInstancesSave(guildId, Array.isArray(list) ? list : null);
        setBusy(false);
        if (failed(res)) return toast(tg('discord.allowed_save_failed', { error: errorText(t, res) }), 'error');
        onSaved(res.allowedInstances ?? null);
        toast(t('perm.saved'), 'ok');
    };
    const visible = servers.filter((s: Loose) => P.isInstanceAllowed(settings, guildId, s.id) && !ownerOf(s.id)).length;
    return (
        <div className="field">
            <span>
                {t('perm.visible')} <small className="pill">{`${visible}/${servers.length}`}</small>
            </span>
            <small className="hint">{t('perm.visible_hint')}</small>
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
                                        servers.map((x: Loose) => x.id),
                                        nodeIds,
                                    ),
                                )
                            }
                        />
                    </label>
                </li>
                {servers.map((srv: Loose) => {
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
        </div>
    );
}

/**
 * 서버별 알림 목록 (서버마다, 기본 꺼짐): 켜짐·꺼짐 알림(`start`)과 자리 꽉 참 알림(`full`).
 * 켠 서버만 이 디스코드 서버의 알림 채널에 알린다
 */
function StartNotice({ api, guildId, notice, setNotice, kind = 'start' }: Loose) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const servers = useData((s) => s.servers);
    const serversLoaded = useData((s) => s.serversLoaded);
    const [busy, setBusy] = useState(false);

    // 한 번에 하나씩: 응답의 목록이 그때의 전체라 겹치면 늦게 온 것이 앞 변경을 덮는다
    const toggle = async (id: Loose, enabled: Loose) => {
        setBusy(true);
        const save = kind === 'full' ? api.guildFullNoticeSave : api.guildStartNoticeSave;
        const res = await save(id, guildId, enabled);
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        setNotice((n: Loose) => ({ ...n, instances: res.instances }));
        toast(enabled ? t(`perm.${prefix}_on`) : t(`perm.${prefix}_off`), 'ok');
    };
    const prefix = kind === 'full' ? 'full_notice' : 'notice';

    const rows = noticeRows(servers, notice);
    return (
        <div className="field">
            <span>{t(`perm.${prefix}`)}</span>
            <small className="hint">{t(`perm.${prefix}_hint`)}</small>
            {notice === false ? null : !notice || !serversLoaded ? (
                <Loading />
            ) : rows.length === 0 ? (
                <Empty>{t('perm.notice_empty')}</Empty>
            ) : (
                <ul className="rows">
                    {rows.map((r: Loose) => (
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

/** 길드 하나의 권한과 명령 계단. 늦게 온 앞선 읽기는 버린다 */
function useGuildPermissions(api: Loose, guildId: Loose) {
    const { t, i18n } = useTranslation();
    const locale = i18n.language;
    const toast = useApp((s) => s.toast);
    const [perm, setPerm] = useState<Loose>(null);
    const [board, setBoard] = useState<Loose>(null);
    const [members, setMembers] = useState<Loose>(null);
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
        api.relayListNodeMembers(guildId).then((m: Loose) => live && setMembers(Array.isArray(m?.members) ? m.members : []));
        return () => {
            live = false;
        };
    }, [api, guildId]);

    const write = async (fn: Loose) => {
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
function MemberLevels({ api, guildId, perm, members, busy, write }: Loose) {
    const { t } = useTranslation();
    const [query, setQuery] = useState('');
    const defaultLevel = typeof perm.defaultLevel === 'number' ? perm.defaultLevel : 1;
    const levelOf = (id: Loose) => (Object.hasOwn(perm.members || {}, id) ? perm.members[id] : null);
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
                    onChange={(l: Loose) => l !== null && write(() => api.permissionsUpdate(guildId, { defaultLevel: l }))}
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
                                        onChange={(l: Loose) =>
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
                busy={busy}
                t={t}
                onSave={(ladder: Loose) => write(() => api.permissionsUpdate(guildId, { ladder }))}
            />
        </>
    );
}

/** 인스턴스 메뉴: 레벨별 명령과 멤버별 레벨을 따로 보인다 (GUI와 같은 동선) */
function InstanceMenu({ api, guildId, data, instance, onBack }: Loose) {
    const { t } = useTranslation();
    const [view, setView] = useState('commands');
    const { perm, board, members, busy, write } = data;
    const srv = board?.instances?.find((i: Loose) => i.id === instance);
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
                    <PermissionStairs
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
                <MemberLevels api={api} guildId={guildId} perm={perm} members={members} busy={busy} write={write} />
            )}
        </div>
    );
}

/** 디스코드 서버 하나: 인스턴스 목록 → 인스턴스 메뉴. 켜짐·꺼짐 알림·자리 꽉 참 알림은 목록 아래 */
export function GuildPermissions({ api, guildId, siblings = [] }: Loose) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const data = useGuildPermissions(api, guildId);
    const [instance, setInstance] = useState<Loose>(null);
    // 켜짐 알림과 보일 서버: {guildId, instances, allowedInstances}. null = 읽는 중, false = 읽지 못함
    const [notice, setNotice] = useState<Loose>(null);
    // 자리 꽉 참 알림 {guildId, instances, allowedInstances} (모양은 위와 같다)
    const [full, setFull] = useState<Loose>(null);
    // 이 PC 의 다른 디스코드 서버가 고른 서버 (서버 하나는 디스코드 서버 하나만)
    const [others, setOthers] = useState<Loose>(null);
    const siblingKey = siblings.join(',');

    useEffect(() => {
        let alive = true;
        setNotice(null);
        setFull(null);
        api.guildStartNoticeGet(guildId).then((res: Loose) => {
            if (!alive) return;
            if (failed(res)) {
                toast(errorText(t, res), 'error');
                setNotice(false);
            } else setNotice(res);
        });
        api.guildFullNoticeGet(guildId).then((res: Loose) => {
            if (alive) setFull(failed(res) ? false : res);
        });
        return () => {
            alive = false;
        };
    }, [api, guildId, t, toast]);

    // biome-ignore lint/correctness/useExhaustiveDependencies: siblingKey 가 siblings 의 값이다
    useEffect(() => {
        let alive = true;
        setOthers(null);
        Promise.all(siblings.map((g: Loose) => api.guildStartNoticeGet(g))).then((list) => {
            if (!alive) return;
            const out: Record<string, Loose> = {};
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
                data={data}
                instance={instance}
                onBack={() => setInstance(null)}
            />
        );
    const instances = data.board?.instances || [];
    return (
        <>
            <div className="field">
                <span>{t('perm.instances')}</span>
                <small className="hint">{t('perm.instances_hint')}</small>
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
            </div>
            {notice && others && (
                <VisibleServers
                    api={api}
                    guildId={guildId}
                    allowed={notice.allowedInstances ?? null}
                    others={others}
                    onSaved={(allowedInstances: Loose) => setNotice((n: Loose) => ({ ...n, allowedInstances }))}
                />
            )}
            <StartNotice api={api} guildId={guildId} notice={notice} setNotice={setNotice} />
            <StartNotice api={api} guildId={guildId} notice={full} setNotice={setFull} kind="full" />
        </>
    );
}
