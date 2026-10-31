import { LANGUAGE_OPTIONS } from '@gui/utils/languages';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { GuildPermissions } from './PermissionsTab';
import { Button, Empty, Loading, Section, Sheet } from './ui';

/**
 * 디스코드 탭 (2026-10-31 사용자 지정 시안). 이 PC 에 붙은 서버는 예전처럼 PC 를 거쳐 권한을 바꾸고,
 * PC 없는 서버는 릴레이 API(src/relay.js)로 바로 바꾼다. 다른 PC 에 붙은 서버는 그 PC 의 링크에서 바꾼다.
 * 보이는 칸은 요청한 사람의 디스코드 권한(can)을 따른다: 설정은 manage, 서버 관리는 channels·messages·members.
 */

/** 이 PC 의 서버(PC 경유)와 릴레이가 알려 준 관리할 수 있는 서버를 합친다. 이 PC 의 것이 먼저 */
export function mergeGuilds(pcNodes, relayGuilds) {
    const out = pcNodes.map((n) => ({ id: String(n.guildId), name: n.guildName || String(n.guildId), here: true }));
    for (const g of relayGuilds) {
        const hit = out.find((x) => x.id === g.id);
        if (hit) Object.assign(hit, { icon: g.icon, hasPc: true, can: g.can });
        else out.push({ ...g, here: false });
    }
    return out;
}

const modAllowed = (can) => !!(can?.channels || can?.messages || can?.members);

/** 릴레이 오류 → 한 줄 */
function relayError(t, res) {
    const known = [
        'NEEDS_PC',
        'PC_OWNED',
        'RATE_LIMITED',
        'FORBIDDEN',
        'BOT_UNAVAILABLE',
        'BOT_MISSING_PERMISSION',
        'GUILD_NOT_FOUND',
        'UNKNOWN_CHANNEL',
        'MUTE_CANNOT',
    ];
    return known.includes(res?.error) ? t(`relay.err.${res.error}`) : t('relay.err.generic');
}

/** 분·초 → "1시간 30분" */
export function durationText(t, minutes) {
    if (minutes < 1) return t('relay.dur.seconds', { count: Math.round(minutes * 60) });
    const m = Math.round(minutes);
    const parts = [
        [Math.floor(m / 1440), 'days'],
        [Math.floor((m % 1440) / 60), 'hours'],
        [m % 60, 'minutes'],
    ];
    return parts
        .filter(([n]) => n)
        .map(([n, unit]) => t(`relay.dur.${unit}`, { count: n }))
        .join(' ');
}

const minutesUntil = (iso) => Math.max(1, Math.ceil((Date.parse(iso) - Date.now()) / 60_000));

function Toggle({ label, hint, checked, disabled, onChange }) {
    return (
        <label className="field field-row">
            <span>
                {label}
                {hint && <small className="hint">{hint}</small>}
            </span>
            <input
                type="checkbox"
                className="switch"
                checked={!!checked}
                disabled={disabled}
                onChange={(e) => onChange(e.target.checked)}
            />
        </label>
    );
}

function Choice({ label, value, options, disabled, onChange }) {
    return (
        <div className="field field-row">
            <span>{label}</span>
            <Dropdown
                className="choice"
                value={String(value)}
                disabled={disabled}
                ariaLabel={label}
                onChange={onChange}
                options={options}
            />
        </div>
    );
}

/** 지금 값이 목록에 없어도 고를 수 있게 넣는다 */
const withValue = (values, value) => (values.includes(value) ? values : [...values, value].sort((a, b) => a - b));
const range = (from, to, step) => Array.from({ length: Math.floor((to - from) / step) + 1 }, (_, i) => from + i * step);

/** 한 서버의 릴레이 값과 쓰기 — 쓰기는 하나씩, 답으로 받은 값으로 화면을 맞춘다 */
function useRelayGuild(relay, guild) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const [data, setData] = useState({});
    // 읽기 실패 뒤 "다시 읽기"를 누를 때마다 늘린다
    const [round, setRound] = useState(0);
    const [busy, setBusy] = useState(false);
    const g = guild.id;
    const manage = !!guild.can?.manage;
    const mod = modAllowed(guild.can);

    const loadModeration = useCallback(async () => {
        const res = await relay.moderation(g);
        setData((d) => ({ ...d, moderation: res.ok ? res.data : { failed: true } }));
    }, [relay, g]);

    // biome-ignore lint/correctness/useExhaustiveDependencies: round 는 다시 읽기 신호다
    useEffect(() => {
        let alive = true;
        const put = (key) => (res) => alive && setData((d) => ({ ...d, [key]: res.ok ? res.data : { failed: true } }));
        if (manage) {
            relay.settings(g).then(put('settings'));
            relay.channels(g).then(put('channels'));
            relay.chatbot(g).then(put('chatbot'));
        }
        if (mod) relay.moderation(g).then(put('moderation'));
        return () => {
            alive = false;
        };
    }, [relay, g, manage, mod, round]);

    /** call: 릴레이 요청, key: 답을 넣을 칸. 'moderation' 이면 서버 관리를 다시 읽고, null 이면 아무것도 바꾸지 않는다 */
    const write = async (call, key) => {
        if (busy) return false;
        setBusy(true);
        try {
            const res = await call();
            if (!res.ok) {
                toast(relayError(t, res), 'error');
                return false;
            }
            if (key === 'moderation') await loadModeration();
            else if (key) setData((d) => ({ ...d, [key]: res.data }));
            toast(t('relay.saved'));
            return true;
        } finally {
            setBusy(false);
        }
    };
    const reload = () => {
        setData({});
        setRound((r) => r + 1);
    };
    return { data, busy, write, reload };
}

function BasicsSection({ relay, g, settings, busy, write }) {
    const { t } = useTranslation();
    const save = (patch) => write(() => relay.saveSettings(g, patch), 'settings');
    return (
        <Section title={t('relay.basics')}>
            <Choice
                label={t('relay.locale')}
                value={settings.locale}
                disabled={busy}
                onChange={(locale) => save({ locale })}
                options={[{ value: 'auto', label: t('relay.locale_auto') }, ...LANGUAGE_OPTIONS]}
            />
            <Toggle
                label={t('relay.big_emoji')}
                hint={t('relay.big_emoji_hint')}
                checked={settings.bigEmoji}
                disabled={busy}
                onChange={(bigEmoji) => save({ bigEmoji })}
            />
        </Section>
    );
}

function MusicSection({ relay, g, settings, busy, write }) {
    const { t } = useTranslation();
    const save = (music) => write(() => relay.saveSettings(g, { music }), 'settings');
    return (
        <Section title={t('relay.music')}>
            <Toggle
                label={t('relay.enabled')}
                checked={settings.music?.enabled}
                disabled={busy}
                onChange={(enabled) => save({ enabled })}
            />
            <Toggle
                label={t('relay.music_ui')}
                hint={t('relay.music_ui_hint')}
                checked={settings.music?.ui}
                disabled={busy}
                onChange={(ui) => save({ ui })}
            />
        </Section>
    );
}

function ReadingSection({ relay, g, settings, busy, write }) {
    const { t } = useTranslation();
    const tts = settings.tts || { volume: 100, speed: 100, pitch: 0 };
    const save = (patch) => write(() => relay.saveSettings(g, { tts: patch }), 'settings');
    const speed = (v) => `${(v / 100).toFixed(2).replace(/0$/, '')}x`;
    return (
        <Section title={t('relay.reading')}>
            <Choice
                label={t('relay.volume')}
                value={tts.volume}
                disabled={busy}
                onChange={(v) => save({ volume: Number(v) })}
                options={withValue(range(10, 150, 10), tts.volume).map((v) => ({ value: String(v), label: `${v}%` }))}
            />
            <Choice
                label={t('relay.speed')}
                value={tts.speed}
                disabled={busy}
                onChange={(v) => save({ speed: Number(v) })}
                options={withValue(range(80, 150, 10), tts.speed).map((v) => ({ value: String(v), label: speed(v) }))}
            />
            <Choice
                label={t('relay.pitch')}
                value={tts.pitch}
                disabled={busy}
                onChange={(v) => save({ pitch: Number(v) })}
                options={range(-6, 6, 1).map((v) => ({ value: String(v), label: v > 0 ? `+${v}` : String(v) }))}
            />
        </Section>
    );
}

function ApiKeySheet({ onSave, onClose, busy }) {
    const { t } = useTranslation();
    const [key, setKey] = useState('');
    return (
        <Sheet title={t('relay.api_key')} onClose={onClose}>
            <form
                className="form"
                onSubmit={(e) => {
                    e.preventDefault();
                    onSave(key.trim());
                }}
            >
                <label className="field">
                    <span>{t('relay.api_key')}</span>
                    <input
                        type="password"
                        autoComplete="off"
                        value={key}
                        maxLength={512}
                        onChange={(e) => setKey(e.target.value)}
                    />
                    <small className="hint">{t('relay.api_key_hint')}</small>
                </label>
                <Button type="submit" tone="accent" busy={busy} disabled={key.trim().length < 8}>
                    {t('relay.save')}
                </Button>
            </form>
        </Sheet>
    );
}

function TalkingSection({ relay, g, chatbot, busy, write }) {
    const { t } = useTranslation();
    const ask = useApp((s) => s.ask);
    const [editing, setEditing] = useState(false);
    const save = (body) => write(() => relay.saveChatbot(g, body), 'chatbot');
    return (
        <Section title={t('relay.talking')}>
            <Toggle
                label={t('relay.enabled')}
                hint={t('relay.talking_hint')}
                checked={chatbot.settings?.enabled}
                disabled={busy}
                onChange={(enabled) => save({ settings: { enabled } })}
            />
            <div className="field field-row">
                <span>
                    {t('relay.api_key')}
                    <small className="hint">
                        {chatbot.hasApiKey ? t('relay.api_key_saved') : t('relay.api_key_none')}
                    </small>
                </span>
                <div className="row-actions">
                    <Button onClick={() => setEditing(true)}>
                        {chatbot.hasApiKey ? t('relay.change') : t('relay.add')}
                    </Button>
                    {chatbot.hasApiKey && (
                        <Button
                            tone="ghost-danger"
                            disabled={busy}
                            onClick={async () => {
                                const ok = await ask({
                                    title: t('relay.api_key_remove_title'),
                                    confirmLabel: t('relay.remove'),
                                    danger: true,
                                });
                                if (ok) save({ apiKey: null });
                            }}
                        >
                            {t('relay.remove')}
                        </Button>
                    )}
                </div>
            </div>
            <div className="field field-row">
                <span>
                    {t('relay.memory_clear')}
                    <small className="hint">{t('relay.memory_clear_hint')}</small>
                </span>
                <Button
                    tone="ghost-danger"
                    disabled={busy}
                    onClick={async () => {
                        const ok = await ask({
                            title: t('relay.memory_clear_title'),
                            message: t('relay.memory_clear_hint'),
                            confirmLabel: t('relay.remove'),
                            danger: true,
                        });
                        if (ok) write(() => relay.clearMemory(g), null);
                    }}
                >
                    {t('relay.remove')}
                </Button>
            </div>
            {editing && (
                <ApiKeySheet
                    busy={busy}
                    onClose={() => setEditing(false)}
                    onSave={async (apiKey) => {
                        if (await save({ apiKey })) setEditing(false);
                    }}
                />
            )}
        </Section>
    );
}

function ChannelsSection({ relay, g, channels, busy, write }) {
    const { t } = useTranslation();
    const save = (change) => write(() => relay.saveChannels(g, change), 'channels');
    const list = channels.channels || [];
    const chatbot = channels.chatbot || [];
    const toggleChatbot = (id) =>
        save({ chatbot: chatbot.includes(id) ? chatbot.filter((x) => x !== id) : [...chatbot, id] });
    if (!channels.botAvailable) return <Empty>{t('relay.err.BOT_UNAVAILABLE')}</Empty>;
    return (
        <Section title={t('relay.channels')}>
            <div className="field">
                <span>{t('relay.talking')}</span>
                <div className="chips">
                    {list.map((c) => (
                        <button
                            key={c.id}
                            type="button"
                            className={chatbot.includes(c.id) ? 'chip on' : 'chip'}
                            aria-pressed={chatbot.includes(c.id)}
                            disabled={busy}
                            onClick={() => toggleChatbot(c.id)}
                        >
                            # {c.name}
                        </button>
                    ))}
                </div>
            </div>
            <Choice
                label={t('relay.music')}
                value={channels.music ?? ''}
                disabled={busy}
                onChange={(v) => save({ music: v || null })}
                options={[
                    { value: '', label: t('relay.none') },
                    ...list.map((c) => ({ value: c.id, label: `# ${c.name}` })),
                ]}
            />
            <div className="field field-locked">
                <span>{t('relay.pc_channels')}</span>
                <p className="locked">
                    <Icon name="lock" size={14} /> {t('relay.pc_channels_hint')}
                </p>
            </div>
        </Section>
    );
}

const SLOW_SECONDS = [0, 5, 10, 30, 60, 300, 900, 3600, 21600];
const LOCK_MINUTES = [30, 60, 360, 1440, 10080];
const AUTO_CLEAR_MINUTES = [60, 360, 720, 1440, 4320, 10080];

function channelLine(t, c) {
    if (c.locked)
        return c.locked.until
            ? t('relay.mod.locked_until', { time: durationText(t, minutesUntil(c.locked.until)) })
            : t('relay.mod.locked');
    return [
        c.slowmode ? t('relay.mod.slow', { time: durationText(t, c.slowmode / 60) }) : t('relay.mod.slow_off'),
        c.autoClear ? t('relay.mod.auto', { time: durationText(t, c.autoClear) }) : t('relay.mod.auto_off'),
    ].join(' · ');
}

function ChannelSheet({ channel, can, busy, onEdit, onClose }) {
    const { t } = useTranslation();
    const lock = channel.locked ? 'keep' : 'off';
    return (
        <Sheet title={`# ${channel.name}`} onClose={onClose}>
            <div className="form">
                {can.channels && (
                    <Choice
                        label={t('relay.mod.slowmode')}
                        value={channel.slowmode}
                        disabled={busy}
                        onChange={(v) => onEdit({ slowmode: Number(v) })}
                        options={withValue(SLOW_SECONDS, channel.slowmode).map((s) => ({
                            value: String(s),
                            label: s ? durationText(t, s / 60) : t('relay.off'),
                        }))}
                    />
                )}
                {can.channels && (
                    <Choice
                        label={t('relay.mod.lock')}
                        value={lock}
                        disabled={busy}
                        onChange={(v) =>
                            v !== 'keep' && onEdit({ lock: v === 'off' || v === 'forever' ? v : Number(v) })
                        }
                        options={[
                            ...(channel.locked ? [{ value: 'keep', label: channelLine(t, channel) }] : []),
                            { value: 'off', label: t('relay.off') },
                            ...LOCK_MINUTES.map((m) => ({ value: String(m), label: durationText(t, m) })),
                            { value: 'forever', label: t('relay.mod.forever') },
                        ]}
                    />
                )}
                {can.messages && (
                    <Choice
                        label={t('relay.mod.auto_clear')}
                        value={channel.autoClear ?? ''}
                        disabled={busy}
                        onChange={(v) => onEdit({ autoClear: v ? Number(v) : null })}
                        options={[
                            { value: '', label: t('relay.off') },
                            ...withValue(AUTO_CLEAR_MINUTES, channel.autoClear ?? 60).map((m) => ({
                                value: String(m),
                                label: durationText(t, m),
                            })),
                        ]}
                    />
                )}
            </div>
        </Sheet>
    );
}

function ModerationSection({ relay, g, can, moderation, busy, write }) {
    const { t } = useTranslation();
    const [open, setOpen] = useState(null);
    if (!moderation.botAvailable) return <Empty>{t('relay.err.BOT_UNAVAILABLE')}</Empty>;
    const channel = open && moderation.channels.find((c) => c.id === open);
    return (
        <Section title={t('relay.mod.title')}>
            {(can.channels || can.messages) && (
                <ul className="rows">
                    {moderation.channels.map((c) => (
                        <li key={c.id} className="row">
                            <button type="button" className="row-open" onClick={() => setOpen(c.id)}>
                                <span className="row-main">
                                    <span className="row-title row-title-one"># {c.name}</span>
                                    <span className="row-sub">{channelLine(t, c)}</span>
                                </span>
                                <Icon name="chevronRight" size={16} />
                            </button>
                        </li>
                    ))}
                </ul>
            )}
            {can.members && (
                <div className="field">
                    <span>{t('relay.mod.mutes')}</span>
                    {moderation.mutes.length === 0 ? (
                        <small className="hint">{t('relay.mod.no_mutes')}</small>
                    ) : (
                        <ul className="rows">
                            {moderation.mutes.map((m) => (
                                <li key={m.userId} className="row member-row">
                                    <div className="row-main">
                                        <div className="row-title row-title-one">{m.name}</div>
                                        <div className="row-sub">
                                            {m.forever || !m.until
                                                ? t('relay.mod.forever')
                                                : t('relay.mod.left', { time: durationText(t, minutesUntil(m.until)) })}
                                        </div>
                                    </div>
                                    <Button
                                        disabled={busy}
                                        aria-label={t('relay.mod.unmute_named', { name: m.name })}
                                        onClick={() =>
                                            write(() => relay.modMute(g, m.userId, { duration: 'off' }), 'moderation')
                                        }
                                    >
                                        {t('relay.mod.unmute')}
                                    </Button>
                                </li>
                            ))}
                        </ul>
                    )}
                </div>
            )}
            {channel && (
                <ChannelSheet
                    channel={channel}
                    can={can}
                    busy={busy}
                    onClose={() => setOpen(null)}
                    onEdit={(edit) => write(() => relay.modChannel(g, channel.id, edit), 'moderation')}
                />
            )}
        </Section>
    );
}

/** PC 없는 서버 — 릴레이에서 바로 바꾼다 */
function RelayGuild({ relay, guild }) {
    const { t } = useTranslation();
    const { data, busy, write, reload } = useRelayGuild(relay, guild);
    const g = guild.id;
    const props = { relay, g, busy, write };
    const manage = !!guild.can?.manage;
    const ready = (x) => x && !x.failed;
    // 연결이 끊겼거나 릴레이가 답하지 못했다 — 숨기지 않고 알리고 다시 읽게 한다
    const broken = [data.settings, data.chatbot, data.channels, data.moderation].some((x) => x?.failed);
    if (broken)
        return (
            <div className="empty">
                <p>{t('relay.load_failed')}</p>
                <Button onClick={reload}>{t('relay.retry')}</Button>
            </div>
        );
    if (manage && !data.settings) return <Loading />;
    return (
        <>
            {manage && ready(data.settings) && (
                <>
                    <BasicsSection {...props} settings={data.settings} />
                    <MusicSection {...props} settings={data.settings} />
                    <ReadingSection {...props} settings={data.settings} />
                </>
            )}
            {manage && ready(data.chatbot) && <TalkingSection {...props} chatbot={data.chatbot} />}
            {manage && ready(data.channels) && <ChannelsSection {...props} channels={data.channels} />}
            {modAllowed(guild.can) &&
                (data.moderation ? (
                    <ModerationSection {...props} can={guild.can} moderation={data.moderation} />
                ) : (
                    <Loading />
                ))}
        </>
    );
}

function GuildChips({ guilds, value, onChange }) {
    const { t } = useTranslation();
    return (
        <div className="chips guild-chips" role="tablist" aria-label={t('perm.guild')}>
            {guilds.map((g) => (
                <button
                    key={g.id}
                    type="button"
                    role="tab"
                    aria-selected={g.id === value}
                    className={g.id === value ? 'chip on' : 'chip'}
                    onClick={() => onChange(g.id)}
                >
                    <span className="guild-initial" aria-hidden="true">
                        {g.name.trim().charAt(0).toUpperCase() || '?'}
                    </span>
                    {g.name}
                    {(g.here || g.hasPc) && <span className="pill">{t('relay.pc')}</span>}
                </button>
            ))}
        </div>
    );
}

export function DiscordTab({ api, relay }) {
    const { t } = useTranslation();
    const hostId = useApp((s) => s.session.hostId);
    const [guilds, setGuilds] = useState(null);
    const [guildId, setGuildId] = useState(null);

    useEffect(() => {
        let alive = true;
        // 아직 /relay 를 모르는 릴레이도 있다 — 그때는 이 PC 의 서버만 보인다
        Promise.all([
            hostId ? api.relayListHostNodes(hostId).catch(() => []) : [],
            relay ? relay.guilds() : { ok: false },
        ]).then(([nodes, res]) => {
            if (!alive) return;
            const list = mergeGuilds(
                Array.isArray(nodes) ? nodes : [],
                res.ok && Array.isArray(res.data) ? res.data : [],
            );
            setGuilds(list);
            setGuildId((g) => g ?? list[0]?.id ?? null);
        });
        return () => {
            alive = false;
        };
    }, [api, relay, hostId]);

    const guild = guilds?.find((g) => g.id === guildId);
    return (
        <Section title={t('tabs.discord')}>
            {guilds === null ? (
                <Loading />
            ) : guilds.length === 0 ? (
                <Empty>{t('relay.no_guilds')}</Empty>
            ) : (
                <>
                    {guilds.length > 1 && <GuildChips guilds={guilds} value={guildId} onChange={setGuildId} />}
                    {guild?.here && <GuildPermissions key={guild.id} api={api} guildId={guild.id} />}
                    {guild && !guild.here && guild.hasPc && <Empty>{t('relay.other_pc')}</Empty>}
                    {guild && !guild.here && !guild.hasPc && <RelayGuild key={guild.id} relay={relay} guild={guild} />}
                </>
            )}
        </Section>
    );
}
