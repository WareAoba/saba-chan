import clsx from 'clsx';
import type { Dispatch, SetStateAction } from 'react';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { CHATBOT_MAX, type ChannelAssign, channelsErrorText } from './ChannelsTab';
import { AUTO_CLEAR_MINUTES, durationText, LOCK_MINUTES, SLOW_SECONDS } from './modDuration';
import {
    ActionNote,
    type ActionResult,
    ASSIGN_KINDS,
    assignOf,
    assignPatch,
    idsOf,
    isList,
    kindOf,
    memberName,
    type ModChannel,
    modErrorText,
    type ModState,
    Section,
    useAction,
} from './ModerationParts';
import type { DiscordMember } from './types';

/* 서버 관리 탭 › 채널 화면: 남은 배정 정리, 채널 목록, 고른 채널의 정보·배정·슬로우·잠금·자동 청소·지금 청소 */

const CLEAR_PERIODS = [10, 60, 360, 1440, 10080];

interface StaleAssignmentsProps {
    known: ModChannel[];
    assignRes: ApiBody;
    saveAssign: (body: ChannelAssign) => Promise<ApiBody>;
    assignSaving: boolean;
}

/** 지워졌거나 사바쨩이 볼 수 없는 채널에 남은 배정 — 채널 목록에서 고를 수 없으니 여기서 뺀다 */
export function StaleAssignments({ known, assignRes, saveAssign, assignSaving }: StaleAssignmentsProps) {
    const { t } = useTranslation('gui');
    const action = useAction();
    const assign = assignOf(assignRes);
    const seen = new Set(known.map((c) => String(c.id)));
    const stale: { kind: string; id: string }[] = [];
    for (const k of ASSIGN_KINDS) {
        for (const id of idsOf(assign, k)) if (!seen.has(id)) stale.push({ kind: k, id });
    }
    if (stale.length === 0) return null;
    const label = (id: string) => {
        const c = (assignRes.channels || []).find((ch: ModChannel) => String(ch.id) === id);
        return c ? `#${c.name}` : id;
    };
    const remove = (id: string) => {
        const body = assignPatch(assign, id, null);
        if (Object.keys(body).length === 0) return;
        action.run(
            () => saveAssign(body),
            () => ({ tone: 'ok', text: t('discord.mod.saved') }),
        );
    };
    return (
        <div className="notice tone-warn dc-mod-stale">
            <Icon name="alertTriangle" size="sm" />
            <div>
                <p>{t('discord.mod.stale_hint')}</p>
                <ul aria-label={t('discord.mod.stale_title')}>
                    {stale.map(({ kind, id }) => (
                        <li key={`${kind}:${id}`}>
                            <span className="muted">{t(`discord.mod.kind_${kind}`)}</span> {label(id)}{' '}
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm"
                                disabled={assignSaving}
                                onClick={() => remove(id)}
                            >
                                {t('discord.mod.stale_remove')}
                            </button>
                        </li>
                    ))}
                </ul>
                <ActionNote note={action.note} errorText={(res) => channelsErrorText(res, t, label)} />
            </div>
        </div>
    );
}

interface ChannelsViewProps {
    guildId: string;
    mod: ModState;
    setMod: Dispatch<SetStateAction<ModState | null>>;
    assignRes: ApiBody | null;
    saveAssign: (body: ChannelAssign) => Promise<ApiBody>;
    assignSaving: boolean;
    members: DiscordMember[];
}

export function ChannelsView({ guildId, mod, setMod, assignRes, saveAssign, assignSaving, members }: ChannelsViewProps) {
    const { t } = useTranslation('gui');
    const channels = mod.channels || [];
    const [selected, setSelected] = useState<string | null>(channels[0]?.id || null);
    const assign = assignOf(assignRes);
    // 배정은 봇이 글을 쓸 수 있는 채널만 (GET …/channels 의 channels)
    const assignable = new Set((assignRes?.channels || []).map((c: ModChannel) => String(c.id)));
    const nameOf = useCallback(
        (id: string) => {
            const c = channels.find((ch) => String(ch.id) === String(id));
            return c ? `#${c.name}` : id;
        },
        [channels],
    );
    const groups = useMemo(() => {
        const out: { key: string; items: ModChannel[] }[] = [];
        for (const c of channels) {
            const key = c.category || '';
            let g = out.find((x) => x.key === key);
            if (!g) out.push((g = { key, items: [] }));
            g.items.push(c);
        }
        return out;
    }, [channels]);

    const replaceChannel = (next: ModChannel) =>
        setMod((m) => ({
            ...m,
            channels: ((m as ModState).channels as ModChannel[]).map((c) => (c.id === next.id ? next : c)),
        }));

    const current = channels.find((c) => c.id === selected) || null;

    if (channels.length === 0) return <p className="muted pad">{t('discord.mod.no_channels')}</p>;

    return (
        <>
            <div className="dc-mod-summary">
                {ASSIGN_KINDS.filter((k) => k === 'chatbot' || (assignRes && k in assignRes)).map((k) => {
                    const ids = idsOf(assign, k);
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
                <div
                    data-spot="mod.channels"
                    className="dc-mod-list"
                    role="listbox"
                    aria-label={t('discord.mod.view_channels')}
                >
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
                        saveAssign={saveAssign}
                        assignSaving={assignSaving}
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

interface ChannelRowProps {
    channel: ModChannel;
    kind: string | null;
    active: boolean;
    onSelect: () => void;
}

function ChannelRow({ channel, kind, active, onSelect }: ChannelRowProps) {
    const { t } = useTranslation('gui');
    const badges: { key: string; tone?: string; text: string }[] = [];
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

interface ChannelPanelProps {
    guildId: string;
    channel: ModChannel;
    can: Record<string, boolean>;
    onChanged: (channel: ModChannel) => void;
    assign: ChannelAssign;
    assignRes: ApiBody | null;
    assignable: boolean;
    saveAssign: (body: ChannelAssign) => Promise<ApiBody>;
    assignSaving: boolean;
    nameOf: (id: string) => string;
    members: DiscordMember[];
}

function ChannelPanel({
    guildId,
    channel,
    can,
    onChanged,
    assign,
    assignRes,
    assignable,
    saveAssign,
    assignSaving,
    nameOf,
    members,
}: ChannelPanelProps) {
    const { t } = useTranslation('gui');
    const errorText = (res: ApiBody) => modErrorText(res, t);
    const patch = (body: ApiBody) => window.api.relayModChannel(guildId, channel.id, body);
    const applied = (res: ApiBody): ActionResult => {
        onChanged(res as ModChannel);
        return { tone: 'ok', text: t('discord.mod.saved') };
    };

    // ── 채널 정보 ──
    const [name, setName] = useState(channel.name);
    const [topic, setTopic] = useState(channel.topic || '');
    // 새로 고침이나 저장으로 바뀐 이름·주제를 입력에 반영 (오래된 값으로 되돌리지 않게)
    useEffect(() => setName(channel.name), [channel.name]);
    useEffect(() => setTopic(channel.topic || ''), [channel.topic]);
    const info = useAction();
    const infoDirty = name.trim() !== channel.name || topic !== (channel.topic || '');
    const saveInfo = () => {
        const body: ApiBody = {};
        if (name.trim() !== channel.name) body.name = name.trim();
        if (topic !== (channel.topic || '')) body.topic = topic || null;
        info.run(() => patch(body), applied);
    };

    // ── 배정 ──
    const assigning = useAction();
    const [musicArmed, setMusicArmed] = useState(false);
    const kind = kindOf(assign, channel.id);
    const pick = (next: string) => {
        const target = kind === next ? null : next;
        if (target && isList(target) && idsOf(assign, target).length >= CHATBOT_MAX) {
            assigning.setNote({ tone: 'error', res: { error: 'TOO_MANY', kind: target } });
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
            () => saveAssign(body),
            () => ({ tone: 'ok', text: t('discord.mod.saved') }),
        );
    };
    const assignError = (res: ApiBody) =>
        res?.error === 'TOO_MANY'
            ? t('discord.mod.assign_too_many', { kind: t(`discord.mod.kind_${res.kind}`), max: CHATBOT_MAX })
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
                        className="input dc-mod-topic"
                        rows={3}
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
                    <fieldset
                        data-spot="mod.assign"
                        className="dc-mod-chips"
                        aria-label={t('discord.mod.assign_title')}
                    >
                        {ASSIGN_KINDS.filter((k) => k === 'chatbot' || (assignRes && k in assignRes)).map((k) => {
                            const elsewhere = !isList(k) && assign[k] && assign[k] !== channel.id ? assign[k] : null;
                            return (
                                <button
                                    key={k}
                                    type="button"
                                    className={clsx('dc-chip-toggle', { active: kind === k })}
                                    aria-pressed={kind === k}
                                    disabled={assignSaving || (!assignable && kind !== k)}
                                    onClick={() => pick(k)}
                                >
                                    {t(`discord.mod.assign_${k}`)}
                                    {elsewhere && (
                                        <span className="muted">
                                            {' · '}
                                            {t('discord.mod.assign_now', { channel: nameOf(elsewhere as string) })}
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

interface ClearSectionProps {
    guildId: string;
    channel: ModChannel;
    can: Record<string, boolean>;
    members: DiscordMember[];
}

function ClearSection({ guildId, channel, can, members }: ClearSectionProps) {
    const { t } = useTranslation('gui');
    const [count, setCount] = useState<number | string>(100);
    const [author, setAuthor] = useState('');
    const [period, setPeriod] = useState('0');
    const [confirming, setConfirming] = useState(false);
    const action = useAction();
    const bodyOf = () => {
        const body: ApiBody = { count: Math.min(1000, Math.max(1, Number(count) || 1)) };
        if (author) body.authorId = author;
        if (Number(period)) body.sinceMinutes = Number(period);
        return body;
    };
    // 권한을 먼저 본다: 지울 수 없으면 확인 창 대신 거절을 보인다
    const ask = () =>
        action.run(
            () => window.api.relayModClearCheck(guildId, channel.id, bodyOf()),
            () => {
                setConfirming(true);
                return null;
            },
        );
    const go = () => {
        setConfirming(false);
        action.run(
            () => window.api.relayModClear(guildId, channel.id, bodyOf()),
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
                    onClick={ask}
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
