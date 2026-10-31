import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';
import { Avatar } from './Avatar';
import { CHATBOT_MAX, channelsErrorText, channelsPatch } from './ChannelsTab';

/**
 * 디스코드 서버 화면 › 서버 관리 (2026-10-31): 디스코드 명령 /청소·/자동청소·/슬로우·/잠금·/음소거 와 채널 정보 수정,
 * 사바쨩 채널 배정(예전 "채널" 탭)을 한 화면에서 한다. 앱은 데몬 API 만 부르고, 데몬이 서버쨩에 넘긴다.
 * 채널 상태·음소거: GET/PATCH/POST/PUT /api/relay/node/:guild/moderation… , 배정: GET/PUT /api/relay/node/:guild/channels
 */

export const moderationAvailable = () =>
    Boolean(window.api?.relayNodeModeration && window.api?.relayNodeChannels && window.api?.relayNodeChannelsSave);

/** 채널 하나가 맡는 일 — 한 채널은 한 가지만 (릴레이 setAssignments) */
export const ASSIGN_KINDS = ['chatbot', 'music', 'notify', 'gameChat', 'board'];

const assignOf = (res) => ({
    chatbot: Array.isArray(res?.chatbot) ? res.chatbot.map(String) : [],
    music: res?.music ? String(res.music) : null,
    notify: res?.notify ? String(res.notify) : null,
    gameChat: res?.gameChat ? String(res.gameChat) : null,
    board: res?.board ? String(res.board) : null,
});

export function kindOf(assign, id) {
    if (assign.chatbot.includes(id)) return 'chatbot';
    return ASSIGN_KINDS.find((k) => k !== 'chatbot' && assign[k] === id) || null;
}

/** 이 채널의 일을 kind 로 (null 이면 뺀다). 다른 채널이 맡던 한 채널짜리 일은 이 채널로 옮긴다. 바뀐 키만 */
export function assignPatch(saved, id, kind) {
    const draft = {
        ...saved,
        chatbot: saved.chatbot.filter((x) => x !== id),
    };
    for (const k of ASSIGN_KINDS) if (k !== 'chatbot' && draft[k] === id) draft[k] = null;
    if (kind === 'chatbot') draft.chatbot = [...draft.chatbot, id];
    else if (kind) draft[kind] = id;
    return channelsPatch(saved, draft);
}

/** 분 → "1시간 30분" 같은 말 (초는 seconds 로) */
export function durationText(t, { minutes = 0, seconds = 0 }) {
    const total = minutes * 60 + seconds;
    if (total < 60) return t('discord.mod.dur_seconds', { count: total });
    const m = Math.round(total / 60);
    const d = Math.floor(m / 1440);
    const h = Math.floor((m % 1440) / 60);
    const rest = m % 60;
    return [
        d && t('discord.mod.dur_days', { count: d }),
        h && t('discord.mod.dur_hours', { count: h }),
        rest && t('discord.mod.dur_minutes', { count: rest }),
    ]
        .filter(Boolean)
        .join(' ');
}

const SLOW_SECONDS = [0, 5, 10, 30, 60, 300, 900, 3600, 21600];
const LOCK_MINUTES = [30, 60, 360, 1440, 10080];
const AUTO_CLEAR_MINUTES = [60, 360, 720, 1440, 4320, 10080];
const CLEAR_PERIODS = [10, 60, 360, 1440, 10080];
export const MUTE_MINUTES = [10, 60, 360, 1440, 10080, 40320];

/** 데몬·릴레이 오류 본문 → 한 줄 */
export function modErrorText(res, t) {
    const code = res?.error_code || res?.code || res?.error;
    if (code === 'BOT_MISSING_PERMISSION') {
        return t('discord.mod.err_BOT_MISSING_PERMISSION', {
            permission: t(`discord.mod.perm_${res.permission}`, { defaultValue: res.permission }),
        });
    }
    if (code === 'RENAME_LIMIT') {
        return t('discord.mod.err_RENAME_LIMIT', { minutes: Math.max(1, Math.ceil((res.retryAfter || 0) / 60)) });
    }
    const known = [
        'NOT_LOCKED',
        'UNKNOWN_CHANNEL',
        'BOT_UNAVAILABLE',
        'NODE_MOVED',
        'MUTE_NOT_MEMBER',
        'MUTE_CANNOT',
        'MUTE_BOT',
        'MUTE_SELF',
        'MUTE_TOO_LONG',
    ];
    return known.includes(code) ? t(`discord.mod.err_${code}`) : t('discord.mod.err_generic');
}

const isError = (res) => !res || res.error || res.error_code || res.ok === false;

/**
 * 서버 관리 탭 — 채널 / 멤버 두 화면.
 * members: 권한 화면과 같은 멤버 목록 (음소거할 사람을 고른다)
 */
export function ModerationTab({ node, members = [], membersLoading, onRefreshMembers }) {
    const { t } = useTranslation('gui');
    const guildId = node.guildId;
    const [view, setView] = useState('channels');
    const [mod, setMod] = useState(null);
    const [assign, setAssign] = useState(null);
    const [loading, setLoading] = useState(true);
    const [error, setError] = useState(null);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const [m, a] = await Promise.all([
                window.api.relayNodeModeration(guildId),
                window.api.relayNodeChannels(guildId),
            ]);
            if (isError(m)) setError(m || {});
            else setMod(m);
            if (!isError(a)) setAssign(a);
        } catch (e) {
            setError({ message: e.message });
        }
        setLoading(false);
    }, [guildId]);

    useEffect(() => {
        load();
    }, [load]);

    if (loading && !mod) {
        return (
            <p className="muted pad">
                <SabaSpinner size="sm" /> {t('discord.mod.loading')}
            </p>
        );
    }

    return (
        <div className="dc-tab dc-mod">
            <div className="dc-mod-head">
                <Segmented
                    ariaLabel={t('discord.tab_moderation')}
                    value={view}
                    onChange={setView}
                    items={[
                        { id: 'channels', label: t('discord.mod.view_channels'), icon: <Icon name="hash" size="sm" /> },
                        { id: 'members', label: t('discord.mod.view_members'), icon: <Icon name="user" size="sm" /> },
                    ]}
                />
                <button type="button" className="btn btn-ghost btn-sm" onClick={load} disabled={loading}>
                    <Icon name="refresh" size="sm" /> {t('discord.mod.reload')}
                </button>
            </div>
            {error && (
                <p className="dc-field-error" role="alert">
                    <Icon name="alertTriangle" size="xs" /> {modErrorText(error, t)}
                </p>
            )}
            {mod && !mod.botAvailable && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('discord.mod.no_bot')}</span>
                </div>
            )}
            {mod?.botAvailable && view === 'channels' && (
                <ChannelsView
                    guildId={guildId}
                    mod={mod}
                    setMod={setMod}
                    assignRes={assign}
                    setAssignRes={setAssign}
                    members={members}
                />
            )}
            {mod?.botAvailable && view === 'members' && (
                <MembersView
                    guildId={guildId}
                    mod={mod}
                    reload={load}
                    members={members}
                    membersLoading={membersLoading}
                    onRefreshMembers={onRefreshMembers}
                />
            )}
        </div>
    );
}

// ── 채널 ──

function ChannelsView({ guildId, mod, setMod, assignRes, setAssignRes, members }) {
    const { t } = useTranslation('gui');
    const channels = mod.channels || [];
    const [selected, setSelected] = useState(channels[0]?.id || null);
    const assign = assignOf(assignRes);
    // 배정은 봇이 글을 쓸 수 있는 채널만 (GET …/channels 의 channels)
    const assignable = new Set((assignRes?.channels || []).map((c) => String(c.id)));
    const nameOf = useCallback(
        (id) => {
            const c = channels.find((ch) => String(ch.id) === String(id));
            return c ? `#${c.name}` : id;
        },
        [channels],
    );
    const groups = useMemo(() => {
        const out = [];
        for (const c of channels) {
            const key = c.category || '';
            let g = out.find((x) => x.key === key);
            if (!g) out.push((g = { key, items: [] }));
            g.items.push(c);
        }
        return out;
    }, [channels]);

    const replaceChannel = (next) =>
        setMod((m) => ({ ...m, channels: m.channels.map((c) => (c.id === next.id ? next : c)) }));

    const current = channels.find((c) => c.id === selected) || null;

    if (channels.length === 0) return <p className="muted pad">{t('discord.mod.no_channels')}</p>;

    return (
        <>
            <div className="dc-mod-summary">
                {ASSIGN_KINDS.filter((k) => k === 'chatbot' || (assignRes && k in assignRes)).map((k) => {
                    const ids = k === 'chatbot' ? assign.chatbot : assign[k] ? [assign[k]] : [];
                    return (
                        <span key={k} className="dc-mod-summary-item">
                            <span className="muted">{t(`discord.mod.kind_${k}`)}</span>{' '}
                            {ids.length === 0
                                ? t('discord.mod.summary_none')
                                : ids.length === 1
                                  ? nameOf(ids[0])
                                  : t('discord.mod.summary_more', { first: nameOf(ids[0]), count: ids.length - 1 })}
                        </span>
                    );
                })}
            </div>
            <div className="dc-mod-split">
                <div className="dc-mod-list" role="listbox" aria-label={t('discord.mod.view_channels')}>
                    {groups.map((g) => (
                        <div key={g.key || '_'} className="dc-channel-group">
                            {g.key && <div className="dc-channel-cat">{g.key}</div>}
                            {g.items.map((c) => (
                                <ChannelRow
                                    key={c.id}
                                    channel={c}
                                    kind={kindOf(assign, c.id)}
                                    active={c.id === selected}
                                    onSelect={() => setSelected(c.id)}
                                />
                            ))}
                        </div>
                    ))}
                </div>
                {current ? (
                    <ChannelPanel
                        key={current.id}
                        guildId={guildId}
                        channel={current}
                        can={mod.can || {}}
                        onChanged={replaceChannel}
                        assign={assign}
                        assignRes={assignRes}
                        assignable={assignable.has(current.id)}
                        setAssignRes={setAssignRes}
                        nameOf={nameOf}
                        members={members}
                    />
                ) : (
                    <p className="muted pad">{t('discord.mod.pick_channel')}</p>
                )}
            </div>
        </>
    );
}

function ChannelRow({ channel, kind, active, onSelect }) {
    const { t } = useTranslation('gui');
    const badges = [];
    if (kind) badges.push({ key: 'kind', tone: 'assign', text: t(`discord.mod.kind_${kind}`) });
    if (channel.slowmode) {
        badges.push({
            key: 'slow',
            text: t('discord.mod.badge_slow', { label: durationText(t, { seconds: channel.slowmode }) }),
        });
    }
    if (channel.locked) badges.push({ key: 'lock', tone: 'warn', text: t('discord.mod.badge_locked') });
    if (channel.autoClear) {
        badges.push({
            key: 'auto',
            text: t('discord.mod.badge_auto', { label: durationText(t, { minutes: channel.autoClear }) }),
        });
    }
    return (
        <button
            type="button"
            role="option"
            aria-selected={active}
            className={clsx('dc-mod-row', { active })}
            onClick={onSelect}
        >
            <span className="dc-mod-row-name">#{channel.name}</span>
            <span className="dc-mod-badges">
                {badges.length === 0 && <span className="muted">{t('discord.mod.badge_none')}</span>}
                {badges.map((b) => (
                    <span key={b.key} className={clsx('dc-badge', 'dc-mod-badge', b.tone && `tone-${b.tone}`)}>
                        {b.text}
                    </span>
                ))}
            </span>
        </button>
    );
}

/** 한 칸에서 부르는 일 하나 — 진행·오류·결과 한 줄을 그 칸 안에 보인다 */
function useAction() {
    const [busy, setBusy] = useState(false);
    const [note, setNote] = useState(null); // {tone, text}
    const run = async (call, onOk) => {
        setBusy(true);
        setNote(null);
        try {
            const res = await call();
            if (isError(res)) setNote({ tone: 'error', res });
            else setNote(onOk ? onOk(res) : null);
        } catch (e) {
            setNote({ tone: 'error', res: { message: e.message } });
        }
        setBusy(false);
    };
    return { busy, note, setNote, run };
}

function ActionNote({ note, errorText }) {
    if (!note) return null;
    if (note.tone === 'error') {
        return (
            <p className="dc-field-error" role="alert">
                <Icon name="alertTriangle" size="xs" /> {errorText(note.res)}
            </p>
        );
    }
    return (
        <p className="dc-hint dc-mod-ok" role="status">
            <Icon name="checkCircle" size="xs" /> {note.text}
        </p>
    );
}

function Section({ icon, title, desc, children, aside }) {
    return (
        <section className="dc-card dc-mod-section">
            <div className="dc-card-head">
                <h2>
                    <Icon name={icon} size="sm" /> {title}
                </h2>
                {aside}
            </div>
            {desc && <p className="dc-card-desc">{desc}</p>}
            {children}
        </section>
    );
}

function ChannelPanel({
    guildId,
    channel,
    can,
    onChanged,
    assign,
    assignRes,
    assignable,
    setAssignRes,
    nameOf,
    members,
}) {
    const { t } = useTranslation('gui');
    const errorText = (res) => modErrorText(res, t);
    const patch = (body) => window.api.relayModChannel(guildId, channel.id, body);
    const applied = (res) => {
        onChanged(res);
        return { tone: 'ok', text: t('discord.mod.saved') };
    };

    // ── 채널 정보 ──
    const [name, setName] = useState(channel.name);
    const [topic, setTopic] = useState(channel.topic || '');
    const info = useAction();
    const infoDirty = name.trim() !== channel.name || topic !== (channel.topic || '');
    const saveInfo = () => {
        const body = {};
        if (name.trim() !== channel.name) body.name = name.trim();
        if (topic !== (channel.topic || '')) body.topic = topic || null;
        info.run(() => patch(body), applied);
    };

    // ── 배정 ──
    const assigning = useAction();
    const [musicArmed, setMusicArmed] = useState(false);
    const kind = kindOf(assign, channel.id);
    const pick = (next) => {
        const target = kind === next ? null : next;
        if (target === 'chatbot' && assign.chatbot.length >= CHATBOT_MAX) {
            assigning.setNote({ tone: 'error', res: { error: 'TOO_MANY' } });
            return;
        }
        // 노래 채널은 사바쨩이 일반 메시지를 지운다: 한 번 더 눌러 확인
        if (target === 'music' && !musicArmed) {
            setMusicArmed(true);
            return;
        }
        setMusicArmed(false);
        const body = assignPatch(assign, channel.id, target);
        if (Object.keys(body).length === 0) return;
        assigning.run(
            () => window.api.relayNodeChannelsSave(guildId, body),
            (res) => {
                setAssignRes(res);
                return { tone: 'ok', text: t('discord.mod.saved') };
            },
        );
    };
    const assignError = (res) =>
        res?.error === 'TOO_MANY'
            ? t('discord.channels_too_many', { max: CHATBOT_MAX })
            : channelsErrorText(res, t, nameOf);

    // ── 슬로우·잠금·자동 청소 ──
    const slow = useAction();
    const lock = useAction();
    const auto = useAction();
    const lockValue = channel.locked ? (channel.locked.until ? 'timed' : 'forever') : 'off';

    return (
        <div className="dc-mod-panel">
            <h2 className="dc-mod-panel-title">
                <Icon name="hash" size="sm" /> {channel.name}
            </h2>

            <Section
                icon="edit"
                title={t('discord.mod.info_title')}
                desc={t('discord.mod.info_desc')}
                aside={
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={!infoDirty || !name.trim() || info.busy || !can.manageChannels}
                        onClick={saveInfo}
                    >
                        {info.busy ? <SabaSpinner size="xs" /> : <Icon name="save" size="sm" />} {t('discord.mod.save')}
                    </button>
                }
            >
                <label className="dc-field">
                    <span className="dc-field-label">{t('discord.mod.name')}</span>
                    <input className="input" value={name} maxLength={100} onChange={(e) => setName(e.target.value)} />
                </label>
                <label className="dc-field">
                    <span className="dc-field-label">{t('discord.mod.topic')}</span>
                    <textarea
                        className="input"
                        rows={2}
                        value={topic}
                        maxLength={1024}
                        onChange={(e) => setTopic(e.target.value)}
                    />
                </label>
                <p className="dc-hint">{t('discord.mod.rename_limit_hint')}</p>
                <ActionNote note={info.note} errorText={errorText} />
            </Section>

            <Section icon="sabaTalking" title={t('discord.mod.assign_title')} desc={t('discord.mod.assign_desc')}>
                {!assignable && kind === null ? (
                    <p className="dc-hint">{t('discord.mod.assign_unavailable')}</p>
                ) : (
                    <fieldset className="dc-mod-chips" aria-label={t('discord.mod.assign_title')}>
                        {ASSIGN_KINDS.filter((k) => k === 'chatbot' || (assignRes && k in assignRes)).map((k) => {
                            const elsewhere =
                                k !== 'chatbot' && assign[k] && assign[k] !== channel.id ? assign[k] : null;
                            return (
                                <button
                                    key={k}
                                    type="button"
                                    className={clsx('dc-chip-toggle', { active: kind === k })}
                                    aria-pressed={kind === k}
                                    disabled={assigning.busy || (!assignable && kind !== k)}
                                    onClick={() => pick(k)}
                                >
                                    {t(`discord.mod.assign_${k}`)}
                                    {elsewhere && (
                                        <span className="muted">
                                            {' · '}
                                            {t('discord.mod.assign_now', { channel: nameOf(elsewhere) })}
                                        </span>
                                    )}
                                </button>
                            );
                        })}
                    </fieldset>
                )}
                {musicArmed && (
                    <div className="notice tone-warn" role="alert">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.channels_music_warn', { channel: `#${channel.name}` })}</span>
                        <button type="button" className="btn btn-danger btn-sm" onClick={() => pick('music')}>
                            {t('discord.channels_save_confirm')}
                        </button>
                    </div>
                )}
                <p className="dc-hint">{t('discord.mod.assign_rule', { max: CHATBOT_MAX })}</p>
                <ActionNote note={assigning.note} errorText={assignError} />
            </Section>

            <Section
                icon="clock"
                title={t('discord.mod.slow_title')}
                desc={t('discord.mod.slow_desc')}
                aside={
                    <CustomDropdown
                        value={String(channel.slowmode || 0)}
                        ariaLabel={t('discord.mod.slow_title')}
                        disabled={slow.busy || !can.manageChannels}
                        onChange={(v) => slow.run(() => patch({ slowmode: Number(v) }), applied)}
                        options={SLOW_SECONDS.map((s) => ({
                            value: String(s),
                            label: s === 0 ? t('discord.mod.off') : durationText(t, { seconds: s }),
                        }))}
                    />
                }
            >
                <ActionNote note={slow.note} errorText={errorText} />
            </Section>

            <Section
                icon="key"
                title={t('discord.mod.lock_title')}
                desc={
                    channel.locked?.until
                        ? t('discord.mod.lock_until', { time: new Date(channel.locked.until).toLocaleString() })
                        : t('discord.mod.lock_desc')
                }
                aside={
                    <CustomDropdown
                        value={lockValue}
                        ariaLabel={t('discord.mod.lock_title')}
                        disabled={lock.busy || !can.manageChannels || !can.manageRoles}
                        onChange={(v) => {
                            if (v === lockValue || v === 'timed') return;
                            lock.run(() => patch({ lock: v === 'off' || v === 'forever' ? v : Number(v) }), applied);
                        }}
                        options={[
                            { value: 'off', label: t('discord.mod.off') },
                            ...(lockValue === 'timed' ? [{ value: 'timed', label: t('discord.mod.lock_timed') }] : []),
                            ...LOCK_MINUTES.map((m) => ({ value: String(m), label: durationText(t, { minutes: m }) })),
                            { value: 'forever', label: t('discord.mod.lock_forever') },
                        ]}
                    />
                }
            >
                <ActionNote note={lock.note} errorText={errorText} />
            </Section>

            <Section
                icon="refresh"
                title={t('discord.mod.auto_title')}
                desc={t('discord.mod.auto_desc')}
                aside={
                    <CustomDropdown
                        value={String(channel.autoClear || 0)}
                        ariaLabel={t('discord.mod.auto_title')}
                        disabled={auto.busy || !can.manageMessages}
                        onChange={(v) => auto.run(() => patch({ autoClear: Number(v) || null }), applied)}
                        options={[
                            { value: '0', label: t('discord.mod.off') },
                            ...AUTO_CLEAR_MINUTES.map((m) => ({
                                value: String(m),
                                label: t('discord.mod.keep_for', { label: durationText(t, { minutes: m }) }),
                            })),
                        ]}
                    />
                }
            >
                <ActionNote note={auto.note} errorText={errorText} />
            </Section>

            <ClearSection guildId={guildId} channel={channel} can={can} members={members} />
        </div>
    );
}

function ClearSection({ guildId, channel, can, members }) {
    const { t } = useTranslation('gui');
    const [count, setCount] = useState(100);
    const [author, setAuthor] = useState('');
    const [period, setPeriod] = useState('0');
    const [confirming, setConfirming] = useState(false);
    const action = useAction();
    const go = () => {
        setConfirming(false);
        const body = { count: Math.min(1000, Math.max(1, Number(count) || 1)) };
        if (author) body.authorId = author;
        if (Number(period)) body.sinceMinutes = Number(period);
        action.run(
            () => window.api.relayModClear(guildId, channel.id, body),
            (res) => ({
                tone: 'ok',
                text: [
                    res.deleted
                        ? t('discord.mod.clear_done', { deleted: res.deleted, kept: res.kept })
                        : t('discord.mod.clear_nothing'),
                    res.olderLeft ? t('discord.mod.clear_older_left') : '',
                ]
                    .filter(Boolean)
                    .join(' '),
            }),
        );
    };
    return (
        <Section
            icon="trash"
            title={t('discord.mod.clear_title')}
            desc={t('discord.mod.clear_desc')}
            aside={
                <button
                    type="button"
                    className="btn btn-danger btn-sm"
                    disabled={action.busy || !can.manageMessages}
                    onClick={() => setConfirming(true)}
                >
                    {action.busy ? <SabaSpinner size="xs" /> : <Icon name="trash" size="sm" />}{' '}
                    {t('discord.mod.clear_button')}
                </button>
            }
        >
            <div className="dc-mod-fields">
                <label className="dc-field">
                    <span className="dc-field-label">{t('discord.mod.count')}</span>
                    <input
                        className="input"
                        type="number"
                        min={1}
                        max={1000}
                        value={count}
                        onChange={(e) => setCount(e.target.value)}
                    />
                </label>
                <div className="dc-field">
                    <span className="dc-field-label">{t('discord.mod.author')}</span>
                    <CustomDropdown
                        value={author}
                        ariaLabel={t('discord.mod.author')}
                        onChange={(v) => setAuthor(String(v))}
                        options={[
                            { value: '', label: t('discord.mod.author_all') },
                            ...members.map((m) => ({ value: String(m.id), label: memberName(m) })),
                        ]}
                    />
                </div>
                <div className="dc-field">
                    <span className="dc-field-label">{t('discord.mod.period')}</span>
                    <CustomDropdown
                        value={period}
                        ariaLabel={t('discord.mod.period')}
                        onChange={(v) => setPeriod(String(v))}
                        options={[
                            { value: '0', label: t('discord.mod.period_all') },
                            ...CLEAR_PERIODS.map((m) => ({
                                value: String(m),
                                label: t('discord.mod.period_recent', { label: durationText(t, { minutes: m }) }),
                            })),
                        ]}
                    />
                </div>
            </div>
            {confirming && (
                <div className="notice tone-warn dc-mod-confirm" role="alertdialog">
                    <Icon name="alertTriangle" size="sm" />
                    <span>
                        <strong>
                            {t('discord.mod.clear_confirm', {
                                channel: channel.name,
                                count: Math.min(1000, Math.max(1, Number(count) || 1)),
                            })}
                        </strong>{' '}
                        {t('discord.mod.clear_confirm_hint')}
                    </span>
                    <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirming(false)}>
                        {t('discord.mod.cancel')}
                    </button>
                    <button type="button" className="btn btn-danger btn-sm" onClick={go}>
                        {t('discord.mod.clear_go')}
                    </button>
                </div>
            )}
            <ActionNote note={action.note} errorText={(res) => modErrorText(res, t)} />
        </Section>
    );
}

// ── 멤버 ──

const memberName = (m) => m?.displayName || m?.globalName || m?.username || m?.id;

function MembersView({ guildId, mod, reload, members, membersLoading, onRefreshMembers }) {
    const { t } = useTranslation('gui');
    const [target, setTarget] = useState('');
    const [duration, setDuration] = useState('60');
    const [reason, setReason] = useState('');
    const action = useAction();
    const can = mod.can || {};
    const now = Date.now();
    const mutes = [...(mod.mutes || [])].sort(
        (a, b) => (a.forever ? Infinity : Date.parse(a.until) || 0) - (b.forever ? Infinity : Date.parse(b.until) || 0),
    );
    const send = (userId, body, done) =>
        action
            .run(
                () => window.api.relayModMute(guildId, userId, body),
                (res) => {
                    reload();
                    return {
                        tone: 'ok',
                        text: t(res.result === 'unmuted' ? 'discord.mod.unmuted' : 'discord.mod.muted'),
                    };
                },
            )
            .then(done);

    return (
        <div className="dc-mod-members">
            <Section
                icon="bellOff"
                title={t('discord.mod.mute_title')}
                desc={t('discord.mod.mute_desc')}
                aside={
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={!target || action.busy || !can.moderateMembers}
                        onClick={() =>
                            send(
                                target,
                                {
                                    duration: duration === 'forever' ? 'forever' : Number(duration),
                                    ...(reason.trim() ? { reason: reason.trim() } : {}),
                                },
                                () => setReason(''),
                            )
                        }
                    >
                        {action.busy ? <SabaSpinner size="xs" /> : <Icon name="bellOff" size="sm" />}{' '}
                        {t('discord.mod.mute_go')}
                    </button>
                }
            >
                {!can.moderateMembers && (
                    <p className="dc-hint">
                        {t('discord.mod.err_BOT_MISSING_PERMISSION', {
                            permission: t('discord.mod.perm_ModerateMembers'),
                        })}
                    </p>
                )}
                <div className="dc-mod-fields">
                    <div className="dc-field">
                        <span className="dc-field-label">
                            {t('discord.mod.member')}
                            {onRefreshMembers && (
                                <button
                                    type="button"
                                    className="icon-btn"
                                    aria-label={t('discord.mod.reload')}
                                    disabled={membersLoading}
                                    onClick={onRefreshMembers}
                                >
                                    <Icon name="refresh" size="xs" />
                                </button>
                            )}
                        </span>
                        <CustomDropdown
                            value={target}
                            ariaLabel={t('discord.mod.member')}
                            placeholder={t('discord.mod.member_pick')}
                            onChange={(v) => setTarget(String(v))}
                            options={members.map((m) => ({ value: String(m.id), label: memberName(m) }))}
                        />
                    </div>
                    <div className="dc-field">
                        <span className="dc-field-label">{t('discord.mod.duration')}</span>
                        <CustomDropdown
                            value={duration}
                            ariaLabel={t('discord.mod.duration')}
                            onChange={(v) => setDuration(String(v))}
                            options={[
                                ...MUTE_MINUTES.map((m) => ({
                                    value: String(m),
                                    label: durationText(t, { minutes: m }),
                                })),
                                { value: 'forever', label: t('discord.mod.mute_forever') },
                            ]}
                        />
                    </div>
                    <label className="dc-field">
                        <span className="dc-field-label">{t('discord.mod.reason')}</span>
                        <input
                            className="input"
                            value={reason}
                            maxLength={200}
                            onChange={(e) => setReason(e.target.value)}
                        />
                    </label>
                </div>
                <ActionNote note={action.note} errorText={(res) => modErrorText(res, t)} />
            </Section>

            <Section icon="user" title={t('discord.mod.muted_title', { count: mutes.length })}>
                {mutes.length === 0 ? (
                    <p className="muted">{t('discord.mod.muted_none')}</p>
                ) : (
                    <ul className="dc-list">
                        {mutes.map((m) => (
                            <li key={m.userId} className="dc-list-row">
                                <Avatar name={m.name} url={m.avatarUrl} size={32} />
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{m.name}</span>
                                    <span className="dc-list-sub">
                                        {m.forever
                                            ? t('discord.mod.forever_note')
                                            : t('discord.mod.left', {
                                                  time: durationText(t, {
                                                      minutes: Math.max(
                                                          1,
                                                          Math.ceil((Date.parse(m.until) - now) / 60000),
                                                      ),
                                                  }),
                                              })}
                                    </span>
                                </span>
                                <button
                                    type="button"
                                    className="btn btn-ghost btn-sm"
                                    disabled={action.busy || !can.moderateMembers}
                                    onClick={() => send(m.userId, { duration: 'off' })}
                                >
                                    {t('discord.mod.unmute')}
                                </button>
                            </li>
                        ))}
                    </ul>
                )}
            </Section>
        </div>
    );
}
