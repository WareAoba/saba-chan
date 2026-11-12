import type { DropdownOption } from '@gui/components/CustomDropdown/CustomDropdown';
import { AUTO_CLEAR_MINUTES, durationText, LOCK_MINUTES, SLOW_SECONDS } from '@gui/components/discord/modDuration';
import { DEFAULT_BEDTIME, HALF_HOURS, SLEEP_FEATURES } from '@gui/components/discord/sleep';
import type { DiscordMember, DiscordNode } from '@gui/components/discord/types';
import { LANGUAGE_OPTIONS } from '@gui/utils/languages';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { createRelay } from '../relay';
import { useApp } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { GuildPermissions } from './PermissionsTab';
import { Avatar, Button, Card, Empty, errorText, failed, Loading, Section, Sheet } from './ui';
import type {
    ApiBody,
    RelayChannels,
    RelayChannelsChange,
    RelayChatbot,
    RelayChatbotPatch,
    RelayClearTarget,
    RelayGuild as RelayGuildInfo,
    RelayGuildCan,
    RelayGuildSettings,
    RelayModChannel,
    RelayModChannelEdit,
    RelayModClear,
    RelayModeration,
    RelayModMember,
    RelayModSanction,
    RelayModSanctionBody,
    RelayMusicSettings,
    RelayResult,
    RelaySanctionKind,
    RelaySettingsPatch,
    RelayTtsSettings,
    TFn,
    WebApi,
} from '../types';

type Relay = ReturnType<typeof createRelay>;

/** 설정 화면이 읽는 서버 설정: 릴레이 값에, 이 PC 의 데몬이 더하는 asleep */
type GuildSettings = Partial<RelayGuildSettings> & { asleep?: boolean };

/** 데몬을 거친 쓰기가 실패했을 때: 릴레이 답과 같은 자리에 데몬 본문을 싣는다 */
interface DaemonFailure {
    ok: false;
    daemon: ApiBody;
}

/** 릴레이 답 또는 데몬을 거친 답 */
type StoreResult<T> = RelayResult<T> | DaemonFailure;

/** 서버 설정·말하는 사바쨩은 이 PC 의 서버면 데몬 경로이고 나머지는 릴레이다 (hereStore) */
export interface GuildStore extends Omit<Relay, 'settings' | 'saveSettings' | 'chatbot' | 'saveChatbot'> {
    settings(g: string): Promise<StoreResult<GuildSettings>>;
    saveSettings(g: string, patch: RelaySettingsPatch): Promise<StoreResult<GuildSettings>>;
    chatbot(g: string): Promise<StoreResult<RelayChatbot>>;
    saveChatbot(g: string, body: RelayChatbotPatch): Promise<StoreResult<RelayChatbot>>;
}

type GuildKey = 'settings' | 'channels' | 'chatbot' | 'moderation';

/** 쓰기 하나: key 는 답을 넣을 칸 (null 이면 아무것도 바꾸지 않는다). 성공하면 true */
type Write = (call: () => Promise<StoreResult<unknown>>, key: GuildKey | null) => Promise<boolean>;

/** 읽지 못한 칸 */
interface Failed {
    failed: true;
}

/** 한 서버의 릴레이 값 (읽는 중이면 칸이 없다) */
interface GuildData {
    settings?: GuildSettings | Failed;
    channels?: RelayChannels | Failed;
    chatbot?: RelayChatbot | Failed;
    moderation?: RelayModeration | Failed;
}

/** 탭에 나오는 디스코드 서버 한 칸: 이 PC 의 서버(here) 또는 릴레이가 알려 준 서버 */
interface GuildEntry {
    id: string;
    name: string;
    here: boolean;
    icon?: string | null;
    hasPc?: boolean;
    can?: RelayGuildCan;
}

/** 섹션들이 함께 받는 것 */
interface SectionProps {
    relay: GuildStore;
    g: string;
    busy: boolean;
    write: Write;
    here: boolean;
}

const isFailed = (x: unknown): x is Failed => !!x && !!(x as Failed).failed;
const ready = <T,>(x: T | Failed | undefined): x is T => !!x && !isFailed(x);

/**
 * 디스코드 탭 (2026-10-31 사용자 지정 시안). 이 PC 에 붙은 서버는 권한·보일 서버를 PC 를 거쳐 바꾸고, 설정은
 * 데몬이 원본인 값(언어·코코낸내·노래·읽어주기·말하는 사바쨩·API 키)을 데몬 경로로, 배정·기억은 릴레이로 바꾼다 (`hereStore`, 2026-11-03).
 * PC 없는 서버는 릴레이 API(src/relay.js)로 바로 바꾼다. 다른 PC 에 붙은 서버는 그 PC 의 링크에서 바꾼다.
 * 보이는 칸은 요청한 사람의 디스코드 권한(can)을 따른다: 설정은 manage, 서버 관리는 channels·messages·members.
 */

/** 이 PC 의 서버(PC 경유)와 릴레이가 알려 준 관리할 수 있는 서버를 합친다. 이 PC 의 것이 먼저 */
export function mergeGuilds(pcNodes: DiscordNode[], relayGuilds: RelayGuildInfo[]) {
    // 서버 사진: 이 PC 의 서버는 데몬이 준 주소(GUI 와 같다), 없을 때만 릴레이 것
    const out: GuildEntry[] = pcNodes.map((n) => ({
        id: String(n.guildId),
        name: n.guildName || String(n.guildId),
        here: true,
        icon: n.iconUrl || null,
    }));
    for (const g of relayGuilds) {
        const hit = out.find((x) => x.id === g.id);
        if (hit) Object.assign(hit, { icon: hit.icon || g.icon, hasPc: true, can: g.can });
        else out.push({ ...g, here: false });
    }
    return out;
}

const modAllowed = (can: RelayGuildCan | undefined) => !!(can?.channels || can?.messages || can?.members);

/** 데몬 답 → 릴레이 답 모양 ({ok, data} | {ok:false, daemon}) */
const fromDaemon = <T = ApiBody>(res: ApiBody, pick: (x: ApiBody) => T = (x) => x as T): StoreResult<T> =>
    failed(res) ? { ok: false, daemon: res } : { ok: true, data: pick(res) };

/**
 * 이 PC 에 붙은 서버의 쓰기 길: 데몬이 원본인 설정과 말하는 사바쨩은 데몬(이모지 업스케일은 데몬이 릴레이로 넘긴다),
 * 채널 배정·기억 지우기는 릴레이 (릴레이 API 는 PC 가 붙은 서버의 데몬 값을 409 PC_OWNED 로 거절한다).
 * API 키는 이 PC 의 키 하나다 (이 PC 에 붙은 모든 디스코드 서버가 같이 쓴다)
 */
export function hereStore(api: WebApi, relay: Relay): GuildStore {
    const chatbotView = (res: ApiBody): RelayChatbot => ({ settings: res.settings || {}, hasApiKey: !!res.hasApiKey });
    return {
        ...relay,
        settings: async (g: string) => fromDaemon<GuildSettings>(await api.discordSettingsGet(g)),
        saveSettings: async (g: string, patch: RelaySettingsPatch) =>
            fromDaemon<GuildSettings>(await api.discordSettingsSet(g, patch), (r) => r.settings),
        chatbot: async (g: string) => fromDaemon(await api.chatbotGuildGet(g), chatbotView),
        saveChatbot: async (g: string, { settings, apiKey }: RelayChatbotPatch): Promise<StoreResult<RelayChatbot>> => {
            if (apiKey !== undefined) {
                const res = await api.chatbotSave({ apiKey });
                if (failed(res)) return { ok: false, daemon: res };
            }
            const res = settings ? await api.chatbotGuildSave(g, settings) : await api.chatbotGuildGet(g);
            return fromDaemon(res, chatbotView);
        },
        // 멤버(사진 포함)는 데몬에서 받는다 (GUI 멤버 탭과 같은 목록). 데몬이 못 주면 릴레이
        modMembers: async (g: string) => {
            // 데몬 답은 {members, partial} (권한 탭과 같은 요청)
            const res = await api.relayListNodeMembers(g, '').catch(() => null);
            const list = res?.members;
            if (!Array.isArray(list)) return relay.modMembers(g);
            return {
                ok: true,
                data: (list as DiscordMember[]).map((m) => ({
                    id: String(m.id),
                    name: m.displayName || m.username || String(m.id),
                    username: m.username || '',
                    avatarUrl: typeof m.avatarUrl === 'string' ? m.avatarUrl : '',
                })),
            };
        },
    };
}

/** 릴레이 오류 → 한 줄 */
function relayError(t: TFn, res: StoreResult<unknown> & { ok: false }) {
    if ('daemon' in res) return errorText(t, res.daemon);
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
        'SANCTION_SELF',
        'SANCTION_BOT',
        'SANCTION_NOT_MEMBER',
        'SANCTION_CANNOT',
        'SANCTION_TOO_LONG',
        'CLEAR_RUNNING',
        'NO_CLEAR_JOB',
    ];
    return known.includes(res.error) ? t(`relay.err.${res.error}`) : t('relay.err.generic');
}

const minutesUntil = (iso: string) => Math.max(1, Math.ceil((Date.parse(iso) - Date.now()) / 60_000));

interface ToggleProps {
    label: React.ReactNode;
    hint?: React.ReactNode;
    checked?: boolean;
    disabled?: boolean;
    onChange: (checked: boolean) => void;
}

function Toggle({ label, hint, checked, disabled, onChange }: ToggleProps) {
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

interface ChoiceProps {
    label: string;
    value: string | number | null | undefined;
    options: DropdownOption[];
    disabled?: boolean;
    onChange: (value: string) => void;
}

function Choice({ label, value, options, disabled, onChange }: ChoiceProps) {
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
const withValue = (values: number[], value: number) => (values.includes(value) ? values : [...values, value].sort((a, b) => a - b));
const range = (from: number, to: number, step: number) => Array.from({ length: Math.floor((to - from) / step) + 1 }, (_, i) => from + i * step);

/** 한 서버의 릴레이 값과 쓰기 — 쓰기는 하나씩, 답으로 받은 값으로 화면을 맞춘다 */
function useRelayGuild(relay: GuildStore, guild: GuildEntry) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const [data, setData] = useState<GuildData>({});
    // 읽기 실패 뒤 "다시 읽기"를 누를 때마다 늘린다
    const [round, setRound] = useState(0);
    const [busy, setBusy] = useState(false);
    const g = guild.id;
    const manage = !!guild.can?.manage;
    const mod = modAllowed(guild.can);

    /** quiet: 진행을 다시 읽을 때 — 한 번 실패해도 화면을 그대로 둔다 */
    const loadModeration = useCallback(
        async (quiet = false) => {
            const res = await relay.moderation(g);
            if (!res.ok && quiet) return;
            setData((d) => ({ ...d, moderation: res.ok ? res.data : { failed: true } }));
        },
        [relay, g],
    );

    // biome-ignore lint/correctness/useExhaustiveDependencies: round 는 다시 읽기 신호다
    useEffect(() => {
        let alive = true;
        const put =
            <K extends GuildKey>(key: K) =>
            (res: StoreResult<NonNullable<GuildData[K]>>) =>
                alive && setData((d) => ({ ...d, [key]: res.ok ? res.data : { failed: true } }));
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
    const write: Write = async (call, key) => {
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
    return { data, busy, write, reload, refresh: loadModeration };
}

function BasicsSection({ relay, g, settings, busy, write, here }: SectionProps & { settings: GuildSettings }) {
    const { t } = useTranslation();
    const save = (patch: RelaySettingsPatch) => write(() => relay.saveSettings(g, patch), 'settings');
    return (
        <Card icon="settings" tone="7" title={t('relay.basics')} desc={t('relay.basics_desc')}>
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
            {/* 게임 채팅 채널 옵션 (원본은 릴레이, 데몬이 넘긴다). 게임 채팅은 PC 가 보내는 것이라 이 PC 의 서버만 */}
            {here &&
                (
                    [
                        ['gameChatGuard', 'relay.game_chat_guard'],
                        ['gameChatClearOnStop', 'relay.game_chat_clear_on_stop'],
                    ] as const
                ).map(
                    ([key, label]) =>
                        typeof settings[key] === 'boolean' && (
                            <Toggle
                                key={key}
                                label={t(label)}
                                hint={t(`${label}_hint`)}
                                checked={settings[key]}
                                disabled={busy}
                                onChange={(on) => save({ [key]: on })}
                            />
                        ),
                )}
        </Card>
    );
}

function MusicSection({ relay, g, settings, busy, write }: SectionProps & { settings: GuildSettings }) {
    const { t } = useTranslation();
    const save = (music: Partial<RelayMusicSettings>) => write(() => relay.saveSettings(g, { music }), 'settings');
    return (
        <Card icon="musicNote" tone="5" title={t('relay.music')} desc={t('relay.music_desc')}>
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
        </Card>
    );
}

function ReadingSection({ relay, g, settings, busy, write }: SectionProps & { settings: GuildSettings }) {
    const { t } = useTranslation();
    const tts = settings.tts || { volume: 100, speed: 100, pitch: 0 };
    const save = (patch: Partial<RelayTtsSettings>) => write(() => relay.saveSettings(g, { tts: patch }), 'settings');
    const speed = (v: number) => `${(v / 100).toFixed(2).replace(/0$/, '')}x`;
    return (
        <Card icon="sabaReading" tone="1" title={t('relay.reading')} desc={t('relay.reading_desc')}>
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
        </Card>
    );
}

function ApiKeySheet({
    onSave,
    onClose,
    busy,
}: {
    onSave: (apiKey: string) => void;
    onClose: () => void;
    busy: boolean;
}) {
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

function TalkingSection({ relay, g, chatbot, busy, write, here }: SectionProps & { chatbot: RelayChatbot }) {
    const { t } = useTranslation();
    const ask = useApp((s) => s.ask);
    const [editing, setEditing] = useState(false);
    const save = (body: RelayChatbotPatch) => write(() => relay.saveChatbot(g, body), 'chatbot');
    return (
        <Card icon="sabaTalking" tone="3" title={t('relay.talking')} desc={t('relay.talking_hint')}>
            <Toggle
                label={t('relay.enabled')}
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
                    {here && <small className="hint">{t('relay.api_key_pc_hint')}</small>}
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
        </Card>
    );
}

/** 여러 채널을 고르는 일 (말하는 사바쨩·게임 채팅·상황판, 사용자 2026-11-04) — 누른 채널을 더하거나 뺀다 */
interface ChannelChipsProps {
    label: string;
    list: { id: string; name: string }[];
    ids: string[];
    busy: boolean;
    onChange: (ids: string[]) => void;
}

function ChannelChips({ label, list, ids, busy, onChange }: ChannelChipsProps) {
    return (
        <fieldset className="field" aria-label={label}>
            <span>{label}</span>
            <div className="chips">
                {list.map((c) => (
                    <button
                        key={c.id}
                        type="button"
                        className={ids.includes(c.id) ? 'chip on' : 'chip'}
                        aria-pressed={ids.includes(c.id)}
                        disabled={busy}
                        onClick={() => onChange(ids.includes(c.id) ? ids.filter((x) => x !== c.id) : [...ids, c.id])}
                    >
                        # {c.name}
                    </button>
                ))}
            </div>
        </fieldset>
    );
}

function ChannelsSection({ relay, g, channels, busy, write, here }: SectionProps & { channels: RelayChannels }) {
    const { t } = useTranslation();
    const save = (change: RelayChannelsChange) => write(() => relay.saveChannels(g, change), 'channels');
    const list = channels.channels || [];
    if (!channels.botAvailable) return <Empty>{t('relay.err.BOT_UNAVAILABLE')}</Empty>;
    return (
        <Card icon="hash" tone="4" title={t('relay.channels')} desc={t('relay.channels_desc')}>
            <ChannelChips
                label={t('relay.talking')}
                list={list}
                ids={channels.chatbot || []}
                busy={busy}
                onChange={(chatbot) => save({ chatbot })}
            />
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
            {here ? (
                <>
                    <Choice
                        label={t('relay.channel_notify')}
                        value={channels.notify ?? ''}
                        disabled={busy}
                        onChange={(v) => save({ notify: v || null })}
                        options={[
                            { value: '', label: t('relay.none') },
                            ...list.map((c) => ({ value: c.id, label: `# ${c.name}` })),
                        ]}
                    />
                    {(['gameChat', 'board'] as const).map((kind) => (
                        <ChannelChips
                            key={kind}
                            label={t(`relay.channel_${kind}`)}
                            list={list}
                            ids={channels[kind]}
                            busy={busy}
                            onChange={(ids) => save({ [kind]: ids })}
                        />
                    ))}
                </>
            ) : (
                <div className="field field-locked">
                    <span>{t('relay.pc_channels')}</span>
                    <p className="locked">
                        <Icon name="lock" size={14} /> {t('relay.pc_channels_hint')}
                    </p>
                </div>
            )}
        </Card>
    );
}

const SLOW_FOR_MINUTES = [10, 60, 360, 1440];
const DAILY_CLEAR_AT = '04:00';
// 릴레이 CLEAR_TARGETS·SANCTION_KINDS 와 같은 값 (server-chan bot/clear/service.ts, bot/guildMod/sanction.ts)
const CLEAR_TARGETS: RelayClearTarget[] = ['all', 'humans', 'self', 'other_bots', 'bots'];
const CLEAR_RECENT_MINUTES = [60, 360, 1440, 10080];
const SANCTION_KINDS: RelaySanctionKind[] = ['chat', 'voice', 'ban', 'all'];
const SANCTION_MINUTES: (number | 'forever')[] = [10, 60, 360, 1440, 4320, 10080, 'forever'];
const PURGE_SECONDS: (0 | 3600 | 86400 | 604800)[] = [0, 3600, 86400, 604800];
const UNIT_MINUTES = [1, 60, 1440, 10080];

/** "2w"·"3h"·"30" → 분. words: 언어의 분·시간·일·주 낱말 (relay.mod.duration_units, 쉼표로). 못 읽으면 null */
export function parseDuration(text: string, words = '') {
    const m = /^\s*(\d+)\s*(\S*)\s*$/u.exec(text);
    if (!m) return null;
    const unit = m[2].toLowerCase();
    const at = (list: string[]) => list.findIndex((w) => w && w === unit);
    let i = unit ? at(['m', 'h', 'd', 'w']) : 0;
    if (i < 0) i = at(words.split(',').map((w) => w.trim().toLowerCase()));
    if (i < 0 && unit === 'min') i = 0;
    const minutes = i < 0 ? 0 : Number(m[1]) * UNIT_MINUTES[i];
    return minutes > 0 ? minutes : null;
}

const minutesSince = (iso: string) => Math.max(1, Math.floor((Date.now() - Date.parse(iso)) / 60_000));

function lockLine(t: TFn, tg: TFn, locked: { until: string | null }) {
    return locked.until
        ? t('relay.mod.locked_until', { time: durationText(tg, { minutes: minutesUntil(locked.until) }) })
        : t('relay.mod.locked');
}

function channelLine(t: TFn, tg: TFn, c: RelayModChannel) {
    if (c.clearing)
        return c.clearing.oldestAt
            ? t('relay.mod.clearing_back', { time: durationText(tg, { minutes: minutesSince(c.clearing.oldestAt) }) })
            : t('relay.mod.clearing');
    if (c.locked) return lockLine(t, tg, c.locked);
    const slow = !c.slowmode
        ? t('relay.mod.slow_off')
        : c.slowUntil
          ? t('relay.mod.slow_until', {
                time: durationText(tg, { seconds: c.slowmode }),
                left: durationText(tg, { minutes: minutesUntil(c.slowUntil) }),
            })
          : t('relay.mod.slow', { time: durationText(tg, { seconds: c.slowmode }) });
    const auto = c.autoClearDaily
        ? t('relay.mod.auto_daily', { time: c.autoClearDaily })
        : c.autoClear
          ? t('relay.mod.auto', { time: durationText(tg, { minutes: c.autoClear }) })
          : t('relay.mod.auto_off');
    const target = (c.autoClearDaily || c.autoClear) && c.autoClearTarget && c.autoClearTarget !== 'all';
    return [slow, auto, ...(target ? [t(`relay.mod.only_${c.autoClearTarget}`)] : [])].join(' · ');
}

/** 칩 하나를 고른다. options: [{ value, label }] */
interface ChipsProps<V> {
    label: string;
    value: V | null;
    options: { value: V; label: React.ReactNode }[];
    disabled?: boolean;
    onChange: (value: V) => void;
}

function Chips<V extends string | number | null>({ label, value, options, disabled, onChange }: ChipsProps<V>) {
    return (
        <div className="field">
            <span>{label}</span>
            <div className="chips">
                {options.map((o) => (
                    <button
                        key={String(o.value)}
                        type="button"
                        className={o.value === value ? 'chip on' : 'chip'}
                        aria-pressed={o.value === value}
                        disabled={disabled}
                        onClick={() => onChange(o.value)}
                    >
                        {o.label}
                    </button>
                ))}
            </div>
        </div>
    );
}

interface SegmentsProps {
    label: string;
    value: string;
    options: { value: string; label: React.ReactNode }[];
    disabled?: boolean;
    onChange: (value: string) => void;
}

function Segments({ label, value, options, disabled, onChange }: SegmentsProps) {
    return (
        <div className="field">
            <span>{label}</span>
            <div className="segments">
                {options.map((o) => (
                    <button
                        key={o.value}
                        type="button"
                        className={o.value === value ? 'on' : ''}
                        aria-pressed={o.value === value}
                        disabled={disabled}
                        onClick={() => onChange(o.value)}
                    >
                        {o.label}
                    </button>
                ))}
            </div>
        </div>
    );
}

interface ChannelSheetProps {
    channel: RelayModChannel;
    can: RelayGuildCan;
    busy: boolean;
    onEdit: (edit: RelayModChannelEdit) => void;
    onClearNow: () => void;
    onClose: () => void;
}

function ChannelSheet({ channel, can, busy, onEdit, onClearNow, onClose }: ChannelSheetProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    // 슬로우 기간은 다음에 켤 슬로우에도 쓴다 (null: 계속)
    const [slowFor, setSlowFor] = useState<number | null>(null);
    const lock = channel.locked ? 'keep' : 'off';
    const autoMode = channel.autoClearDaily ? 'daily' : channel.autoClear ? 'keep' : 'off';
    // 기간을 고르지 않았으면 지금 걸린 기간(남은 시간)을 지킨다
    const keepFor = channel.slowUntil ? minutesUntil(channel.slowUntil) : null;
    const slowEdit = (slowmode: number, minutes: number | null): RelayModChannelEdit => {
        const span = minutes ?? keepFor;
        return slowmode && span ? { slowmode, slowFor: span } : { slowmode };
    };
    return (
        <Sheet title={`# ${channel.name}`} onClose={onClose}>
            <div className="form">
                {can.channels && (
                    <>
                        <Choice
                            label={t('relay.mod.slowmode')}
                            value={channel.slowmode}
                            disabled={busy}
                            onChange={(v) => onEdit(slowEdit(Number(v), slowFor))}
                            options={withValue(SLOW_SECONDS, channel.slowmode).map((s) => ({
                                value: String(s),
                                label: s ? durationText(tg, { seconds: s }) : t('relay.off'),
                            }))}
                        />
                        <Chips
                            label={t('relay.mod.slow_for')}
                            value={slowFor}
                            disabled={busy}
                            onChange={(m) => {
                                setSlowFor(m);
                                if (channel.slowmode)
                                    onEdit(
                                        m ? { slowmode: channel.slowmode, slowFor: m } : { slowmode: channel.slowmode },
                                    );
                            }}
                            options={[
                                { value: null, label: t('relay.mod.forever') },
                                ...SLOW_FOR_MINUTES.map((m) => ({ value: m, label: durationText(tg, { minutes: m }) })),
                            ]}
                        />
                        <small className="hint">
                            {channel.slowmode && channel.slowUntil
                                ? t('relay.mod.slow_ends', { time: durationText(tg, { minutes: minutesUntil(channel.slowUntil) }) })
                                : t('relay.mod.slow_for_hint')}
                        </small>
                        <Choice
                            label={t('relay.mod.lock')}
                            value={lock}
                            disabled={busy}
                            onChange={(v) =>
                                v !== 'keep' && onEdit({ lock: v === 'off' || v === 'forever' ? v : Number(v) })
                            }
                            options={[
                                ...(channel.locked ? [{ value: 'keep', label: lockLine(t, tg, channel.locked) }] : []),
                                { value: 'off', label: t('relay.off') },
                                ...LOCK_MINUTES.map((m) => ({ value: String(m), label: durationText(tg, { minutes: m }) })),
                                { value: 'forever', label: t('relay.mod.forever') },
                            ]}
                        />
                    </>
                )}
                {can.messages && (
                    <>
                        <Segments
                            label={t('relay.mod.auto_clear')}
                            value={autoMode}
                            disabled={busy}
                            onChange={(mode) =>
                                mode !== autoMode &&
                                onEdit(
                                    mode === 'off'
                                        ? { autoClear: null }
                                        : mode === 'keep'
                                          ? { autoClear: 1440 }
                                          : { autoClearDaily: DAILY_CLEAR_AT },
                                )
                            }
                            options={[
                                { value: 'off', label: t('relay.off') },
                                { value: 'keep', label: t('relay.mod.auto_keep') },
                                { value: 'daily', label: t('relay.mod.auto_daily_mode') },
                            ]}
                        />
                        {autoMode === 'keep' && (
                            <Choice
                                label={t('relay.mod.auto_keep')}
                                value={channel.autoClear}
                                disabled={busy}
                                onChange={(v) => onEdit({ autoClear: Number(v) })}
                                options={withValue(AUTO_CLEAR_MINUTES, channel.autoClear as number).map((m) => ({
                                    value: String(m),
                                    label: durationText(tg, { minutes: m }),
                                }))}
                            />
                        )}
                        {autoMode === 'daily' && (
                            <Choice
                                label={t('relay.mod.auto_daily_at')}
                                value={channel.autoClearDaily}
                                disabled={busy}
                                onChange={(v) => onEdit({ autoClearDaily: v })}
                                options={[...new Set([...HALF_HOURS, channel.autoClearDaily as string])]
                                    .sort()
                                    .map((h) => ({ value: h, label: h }))}
                            />
                        )}
                        {autoMode !== 'off' && (
                            <Choice
                                label={t('relay.mod.auto_target')}
                                value={channel.autoClearTarget || 'all'}
                                disabled={busy}
                                onChange={(v) => onEdit({ autoClearTarget: v as RelayClearTarget })}
                                options={CLEAR_TARGETS.map((k) => ({ value: k, label: t(`relay.mod.target_${k}`) }))}
                            />
                        )}
                        {autoMode === 'daily' && (
                            <small className="hint">
                                {t('relay.mod.auto_daily_hint', { time: channel.autoClearDaily })}
                            </small>
                        )}
                        <Button onClick={onClearNow}>{t('relay.mod.clear_now')}</Button>
                    </>
                )}
            </div>
        </Sheet>
    );
}

/** datetime-local 값 → ms (브라우저 시간대) */
const localMs = (value: string) => (value ? Date.parse(value) : undefined);

/** 지금 청소 — /청소 와 같은 칸. 이어 지우는 중이면 진행과 중지를 보인다 */
interface ClearSheetProps {
    channel: RelayModChannel;
    members: RelayModMember[] | null;
    busy: boolean;
    onClear: (body: RelayModClear) => void;
    onStop: () => void;
    onClose: () => void;
}

function ClearSheet({ channel, members, busy, onClear, onStop, onClose }: ClearSheetProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [target, setTarget] = useState('all');
    const [author, setAuthor] = useState('');
    const [range, setRange] = useState('all');
    const [recent, setRecent] = useState(60);
    const [from, setFrom] = useState('');
    const [to, setTo] = useState('');
    const [count, setCount] = useState('');
    const n = Number(count);
    const countOk = count === '' || (Number.isInteger(n) && n >= 1 && n <= 1000);
    // 시각 지정은 하나 이상, 부터 < 까지 — 비우면 채널 전체를 지우게 되므로 막는다
    const rangeOk = range !== 'custom' || ((localMs(from) || localMs(to)) && !((localMs(from) as number) >= (localMs(to) as number)));
    const submit = () => {
        const body: RelayModClear = {};
        if (target !== 'all') body.target = target as RelayClearTarget;
        if (author) body.authorId = author;
        if (range === 'recent') body.sinceMinutes = recent;
        if (range === 'custom') {
            if (localMs(from)) body.from = localMs(from);
            if (localMs(to)) body.to = localMs(to);
        }
        if (count !== '') body.count = n;
        onClear(body);
    };
    const { clearing } = channel;
    return (
        <Sheet title={t('relay.mod.clear_title', { name: channel.name })} onClose={onClose}>
            <div className="form">
                {clearing && (
                    <output className="field">
                        <span>
                            {clearing.oldestAt
                                ? t('relay.mod.clearing_back_long', {
                                      time: durationText(tg, { minutes: minutesSince(clearing.oldestAt) }),
                                  })
                                : t('relay.mod.clearing')}
                        </span>
                        <small className="hint">
                            {t('relay.mod.clear_deleted', { n: clearing.deleted.toLocaleString() })}
                        </small>
                        <Button tone="danger" disabled={busy} onClick={onStop}>
                            {t('relay.mod.stop')}
                        </Button>
                    </output>
                )}
                <Chips
                    label={t('relay.mod.clear_target')}
                    value={target}
                    disabled={busy}
                    onChange={setTarget}
                    options={CLEAR_TARGETS.map((k) => ({ value: k, label: t(`relay.mod.target_${k}`) }))}
                />
                <Choice
                    label={t('relay.mod.clear_author')}
                    value={author}
                    disabled={busy || !members}
                    onChange={setAuthor}
                    options={[
                        { value: '', label: t('relay.none') },
                        ...(members || []).map((m) => ({ value: m.id, label: m.name })),
                    ]}
                />
                <Segments
                    label={t('relay.mod.clear_range')}
                    value={range}
                    disabled={busy}
                    onChange={setRange}
                    options={['all', 'recent', 'custom'].map((k) => ({ value: k, label: t(`relay.mod.range_${k}`) }))}
                />
                {range === 'recent' && (
                    <Choice
                        label={t('relay.mod.range_recent')}
                        value={recent}
                        disabled={busy}
                        onChange={(v) => setRecent(Number(v))}
                        options={CLEAR_RECENT_MINUTES.map((m) => ({ value: String(m), label: durationText(tg, { minutes: m }) }))}
                    />
                )}
                {range === 'custom' && (
                    <>
                        <label className="field">
                            <span>{t('relay.mod.range_from')}</span>
                            <input type="datetime-local" value={from} onChange={(e) => setFrom(e.target.value)} />
                        </label>
                        <label className="field">
                            <span>{t('relay.mod.range_to')}</span>
                            <input type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} />
                        </label>
                    </>
                )}
                <label className="field">
                    <span>{t('relay.mod.clear_count')}</span>
                    <input
                        type="number"
                        inputMode="numeric"
                        min={1}
                        max={1000}
                        value={count}
                        onChange={(e) => setCount(e.target.value)}
                    />
                    <small className="hint">{t('relay.mod.clear_count_hint')}</small>
                </label>
                <Button tone="accent" busy={busy} disabled={!!clearing || !countOk || !rangeOk} onClick={submit}>
                    {t('relay.mod.clear_start')}
                </Button>
                <small className="hint">{t('relay.mod.clear_hint')}</small>
            </div>
        </Sheet>
    );
}

/** 제재하기 — /제재 와 같은 칸. 결과는 디스코드에도 알린다 (릴레이가 서버의 시스템 채널에, 멘션 없이) */
interface SanctionSheetProps {
    members: RelayModMember[] | null;
    busy: boolean;
    onSanction: (userId: string, body: RelayModSanctionBody) => Promise<boolean>;
    onClose: () => void;
}

/** 제재할 사람으로 고른 한 명 (서버에 없는 사람은 ID 만) */
interface PickedMember {
    id: string;
    name: string;
    username?: string;
    avatarUrl?: string;
}

function SanctionSheet({ members, busy, onSanction, onClose }: SanctionSheetProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [query, setQuery] = useState('');
    const [picked, setPicked] = useState<PickedMember | null>(null);
    const [kind, setKind] = useState('chat');
    const [minutes, setMinutes] = useState<number | 'forever'>(60);
    const [custom, setCustom] = useState('');
    const [reason, setReason] = useState('');
    const [purge, setPurge] = useState(0);
    const q = query.trim().toLowerCase();
    const found = q
        ? (members || [])
              .filter((m) => m.name.toLowerCase().includes(q) || m.username?.toLowerCase().includes(q))
              .slice(0, 8)
        : [];
    // 밴은 서버에 없는 사람도 ID 로 한다
    const byId: PickedMember[] = /^\d{17,20}$/.test(q) && !found.some((m) => m.id === q) ? [{ id: q, name: q, username: '' }] : [];
    const duration = custom.trim() ? parseDuration(custom, t('relay.mod.duration_units')) : minutes;
    const submit = async () => {
        const body: RelayModSanctionBody = { kind: kind as RelaySanctionKind, duration: duration as number | 'forever' };
        if (reason.trim()) body.reason = reason.trim();
        if (kind === 'ban' && purge) body.purgeSeconds = purge as RelayModSanctionBody['purgeSeconds'];
        if (await onSanction((picked as PickedMember).id, body)) onClose();
    };
    return (
        <Sheet title={t('relay.mod.sanction_title')} onClose={onClose}>
            <div className="form">
                <label className="field">
                    <span>{t('relay.mod.member')}</span>
                    <input
                        type="search"
                        value={query}
                        placeholder={t('relay.mod.member_search')}
                        disabled={!members}
                        onChange={(e) => {
                            setQuery(e.target.value);
                            setPicked(null);
                        }}
                    />
                    <small className="hint">{t('relay.mod.member_hint')}</small>
                </label>
                {[...found, ...byId].length > 0 && (
                    <div className="chips">
                        {[...found, ...byId].map((m) => (
                            <button
                                key={m.id}
                                type="button"
                                className={picked?.id === m.id ? 'chip on' : 'chip'}
                                aria-pressed={picked?.id === m.id}
                                onClick={() => setPicked(m)}
                            >
                                {m.username && <Avatar url={m.avatarUrl} name={m.name} className="avatar avatar-sm" />}
                                {m.username ? `${m.name} (@${m.username})` : t('relay.mod.member_id', { id: m.id })}
                            </button>
                        ))}
                    </div>
                )}
                <Segments
                    label={t('relay.mod.kind')}
                    value={kind}
                    disabled={busy}
                    onChange={setKind}
                    options={SANCTION_KINDS.map((k) => ({ value: k, label: t(`relay.mod.kind_${k}`) }))}
                />
                <small className="hint">{t(`relay.mod.kind_${kind}_hint`)}</small>
                <Chips
                    label={t('relay.mod.duration')}
                    value={custom.trim() ? null : minutes}
                    disabled={busy}
                    onChange={(m) => {
                        setMinutes(m);
                        setCustom('');
                    }}
                    options={SANCTION_MINUTES.map((m) => ({
                        value: m,
                        label: m === 'forever' ? t('relay.mod.forever') : durationText(tg, { minutes: m }),
                    }))}
                />
                <label className="field">
                    <span>{t('relay.mod.duration_custom')}</span>
                    <input
                        type="text"
                        value={custom}
                        placeholder={t('relay.mod.duration_custom_hint')}
                        onChange={(e) => setCustom(e.target.value)}
                    />
                </label>
                <label className="field">
                    <span>{t('relay.mod.reason')}</span>
                    <input type="text" value={reason} maxLength={200} onChange={(e) => setReason(e.target.value)} />
                </label>
                {kind === 'ban' && (
                    <Choice
                        label={t('relay.mod.purge')}
                        value={purge}
                        disabled={busy}
                        onChange={(v) => setPurge(Number(v))}
                        options={PURGE_SECONDS.map((s) => ({
                            value: String(s),
                            label: s ? durationText(tg, { seconds: s }) : t('relay.none'),
                        }))}
                    />
                )}
                <small className="hint">{t('relay.mod.announce_hint')}</small>
                <Button tone="accent" busy={busy} disabled={!picked || !duration} onClick={submit}>
                    {t('relay.mod.sanction')}
                </Button>
            </div>
        </Sheet>
    );
}

function SanctionList({
    sanctions,
    members,
    busy,
    onRelease,
}: {
    sanctions: RelayModSanction[];
    members: RelayModMember[] | null;
    busy: boolean;
    onRelease: (sanction: RelayModSanction) => void;
}) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    if (sanctions.length === 0) return <small className="hint">{t('relay.mod.no_sanctions')}</small>;
    return (
        <ul className="rows" aria-label={t('relay.mod.sanctions')}>
            {sanctions.map((s) => (
                <li key={`${s.userId}-${s.kind}`} className="row member-row">
                    <Avatar url={members?.find((m) => m.id === s.userId)?.avatarUrl} name={s.name} />
                    <div className="row-main">
                        <div className="row-title row-title-one">
                            {s.name}
                            <span className="pill">{t(`relay.mod.kind_${s.kind}`)}</span>
                        </div>
                        <div className="row-sub">
                            {[
                                s.until
                                    ? t('relay.mod.left', { time: durationText(tg, { minutes: minutesUntil(s.until) }) })
                                    : t('relay.mod.forever'),
                                ...(s.inGuild ? [] : [t('relay.mod.not_in_guild')]),
                            ].join(' · ')}
                        </div>
                    </div>
                    <Button
                        disabled={busy}
                        aria-label={t('relay.mod.release_named', { name: s.name })}
                        onClick={() => onRelease(s)}
                    >
                        {t('relay.mod.release')}
                    </Button>
                </li>
            ))}
        </ul>
    );
}

// 이어 지우는 동안 진행을 다시 읽는 간격
const CLEARING_POLL_MS = 5000;

interface ModerationSectionProps extends SectionProps {
    can: RelayGuildCan;
    moderation: RelayModeration;
    refresh: (quiet?: boolean) => Promise<void>;
}

function ModerationSection({ relay, g, can, moderation, busy, write, refresh }: ModerationSectionProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    // { id, sheet: 'channel' | 'clear' } | { sheet: 'sanction' }
    const [open, setOpen] = useState<{ id?: string; sheet: 'channel' | 'clear' | 'sanction' } | null>(null);
    const [members, setMembers] = useState<RelayModMember[] | null>(null);
    const clearing = !!moderation.channels?.some((c) => c.clearing);
    useEffect(() => {
        if (!clearing) return;
        const timer = setInterval(() => refresh(true), CLEARING_POLL_MS);
        return () => clearInterval(timer);
    }, [clearing, refresh]);
    // 제재 중인 사람이 있으면 그 사진도 멤버 목록에서 찾는다
    const needsMembers = open?.sheet === 'clear' || open?.sheet === 'sanction' || !!moderation.sanctions?.length;
    useEffect(() => {
        if (!needsMembers || members) return;
        let alive = true;
        relay.modMembers(g).then((res) => alive && setMembers(res.ok && Array.isArray(res.data) ? res.data : []));
        return () => {
            alive = false;
        };
    }, [needsMembers, members, relay, g]);
    if (!moderation.botAvailable) return <Empty>{t('relay.err.BOT_UNAVAILABLE')}</Empty>;
    const channel = open?.id && moderation.channels.find((c) => c.id === open.id);
    const close = () => setOpen(null);
    return (
        <Card icon="tool" tone="2" title={t('relay.mod.title')} desc={t('relay.mod.desc')}>
            {(can.channels || can.messages) && (
                <ul className="rows">
                    {moderation.channels.map((c) => (
                        <li key={c.id} className="row">
                            <button
                                type="button"
                                className="row-open"
                                onClick={() => setOpen({ id: c.id, sheet: 'channel' })}
                            >
                                <span className="row-main">
                                    <span className="row-title row-title-one"># {c.name}</span>
                                    <span className="row-sub">{channelLine(t, tg, c)}</span>
                                </span>
                                <Icon name="chevronRight" size={16} />
                            </button>
                        </li>
                    ))}
                </ul>
            )}
            {can.members && (
                <div className="field">
                    <span>{t('relay.mod.sanctions')}</span>
                    <SanctionList
                        sanctions={moderation.sanctions || []}
                        members={members}
                        busy={busy}
                        onRelease={(s) =>
                            write(() => relay.modSanction(g, s.userId, { kind: s.kind, duration: 'off' }), 'moderation')
                        }
                    />
                    <Button onClick={() => setOpen({ sheet: 'sanction' })}>{t('relay.mod.sanction_open')}</Button>
                </div>
            )}
            {channel && open?.sheet === 'channel' && (
                <ChannelSheet
                    channel={channel}
                    can={can}
                    busy={busy}
                    onClose={close}
                    onClearNow={() => setOpen({ id: channel.id, sheet: 'clear' })}
                    onEdit={(edit) => write(() => relay.modChannel(g, channel.id, edit), 'moderation')}
                />
            )}
            {channel && open?.sheet === 'clear' && (
                <ClearSheet
                    channel={channel}
                    members={members}
                    busy={busy}
                    onClose={close}
                    onClear={(body) => write(() => relay.modClear(g, channel.id, body), 'moderation')}
                    onStop={() => write(() => relay.modClearStop(g, channel.id), 'moderation')}
                />
            )}
            {open?.sheet === 'sanction' && (
                <SanctionSheet
                    members={members}
                    busy={busy}
                    onClose={close}
                    onSanction={(userId, body) => write(() => relay.modSanction(g, userId, body), 'moderation')}
                />
            )}
        </Card>
    );
}

/**
 * 코코낸내 (GUI 봇 설정의 코코낸내 칸과 같은 값): 지금 재우기, 잠든 동안 끌 기능, 착한 어린이 시각.
 * PC 없는 서버도 릴레이 기능(음악·읽어주기·말하는 사바쨩)을 재우므로 보인다 — 서버 조작·알림만 PC 가 있어야 한다 (사용자 11-12)
 */
function SleepSection({ relay, g, settings, busy, write, here }: SectionProps & { settings: GuildSettings }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const save = (patch: RelaySettingsPatch) => write(() => relay.saveSettings(g, patch), 'settings');
    const blocks = Array.isArray(settings.sleepBlocks) ? settings.sleepBlocks : SLEEP_FEATURES;
    const bedtime = settings.bedtime || null;
    const time = (key: 'start' | 'end', other: 'start' | 'end') => {
        const value = bedtime?.[key] ?? DEFAULT_BEDTIME[key];
        const avoid = bedtime?.[other] ?? DEFAULT_BEDTIME[other];
        const times = [...new Set([...HALF_HOURS, value])].sort().filter((h) => h !== avoid);
        return (
            <Choice
                label={tg(`discord.bot_settings.bedtime_${key}`)}
                value={value}
                disabled={busy || !bedtime}
                onChange={(v) => v !== value && save({ bedtime: { ...bedtime, [key]: v } as { start: string; end: string } })}
                options={times.map((h) => ({ value: h, label: h }))}
            />
        );
    };
    return (
        <Card icon="zzz" tone="7" title={tg('discord.bot_settings.sleep_title')}>
            <Toggle
                label={t('relay.sleep_now')}
                hint={settings.asleep ? tg('discord.sleep_corner_asleep') : t('relay.awake')}
                checked={settings.sleeping}
                disabled={busy}
                onChange={(sleeping) => save({ sleeping })}
            />
            <p className="hint">{tg('discord.bot_settings.sleep_desc')}</p>
            {SLEEP_FEATURES.map((f) => {
                // PC 가 없으면 끌 서버 조작·알림도 없다 — 잠가 꺼진 채로 보인다 (저장된 값은 그대로 둔다)
                const locked = f === 'commands' && !here;
                return (
                    <Toggle
                        key={f}
                        label={tg(`discord.bot_settings.sleep_${f}`)}
                        hint={locked ? t('relay.sleep_needs_pc') : tg(`discord.bot_settings.sleep_${f}_hint`)}
                        checked={!locked && blocks.includes(f)}
                        disabled={busy || locked}
                        onChange={(on) =>
                            save({ sleepBlocks: SLEEP_FEATURES.filter((x) => (x === f ? on : blocks.includes(x))) })
                        }
                    />
                );
            })}
            <Toggle
                label={tg('discord.bot_settings.bedtime')}
                hint={tg('discord.bot_settings.bedtime_hint')}
                checked={!!bedtime}
                disabled={busy}
                onChange={(on) => save({ bedtime: on ? DEFAULT_BEDTIME : null })}
            />
            {time('start', 'end')}
            {time('end', 'start')}
            <p className="hint">{tg('discord.bot_settings.bedtime_zone')}</p>
        </Card>
    );
}

/** PC 없는 서버 — 릴레이에서 바로 바꾼다. here: 이 PC 의 서버 (relay 는 hereStore, 코코낸내를 더한다. 서버 관리는 늘 릴레이) */
function RelayGuild({ relay, guild, here = false }: { relay: GuildStore; guild: GuildEntry; here?: boolean }) {
    const { t } = useTranslation();
    const { data, busy, write, reload, refresh } = useRelayGuild(relay, guild);
    const g = guild.id;
    const props = { relay, g, busy, write, here };
    const manage = !!guild.can?.manage;
    // 연결이 끊겼거나 릴레이가 답하지 못했다 — 숨기지 않고 알리고 다시 읽게 한다
    const broken = [data.settings, data.chatbot, data.channels, data.moderation].some((x) => isFailed(x));
    if (broken)
        return (
            <div className="empty">
                <p>{t('relay.load_failed')}</p>
                <Button onClick={reload}>{t('relay.retry')}</Button>
            </div>
        );
    if (manage && !data.settings) return <Loading />;
    return (
        <div className="set-card-grid">
            {manage && ready(data.settings) && (
                <>
                    <BasicsSection {...props} settings={data.settings} />
                    <SleepSection {...props} settings={data.settings} />
                    <MusicSection {...props} settings={data.settings} />
                    <ReadingSection {...props} settings={data.settings} />
                </>
            )}
            {manage && ready(data.chatbot) && <TalkingSection {...props} chatbot={data.chatbot} />}
            {manage && ready(data.channels) && <ChannelsSection {...props} channels={data.channels} />}
            {modAllowed(guild.can) &&
                (ready(data.moderation) ? (
                    <ModerationSection {...props} can={guild.can as RelayGuildCan} moderation={data.moderation} refresh={refresh} />
                ) : (
                    <Loading />
                ))}
        </div>
    );
}

function GuildChips({
    guilds,
    value,
    onChange,
}: {
    guilds: GuildEntry[];
    value: string | null;
    onChange: (id: string) => void;
}) {
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
                    <Avatar url={g.icon} name={g.name} className="guild-initial" />
                    <span className="guild-chip-text">
                        <span className="guild-chip-name">{g.name}</span>
                        {/* 넓은 화면 왼쪽 목록에서만 보이는 줄: 이 서버가 어디에 붙어 있는지 */}
                        <span className="guild-chip-sub">
                            {t(g.here ? 'relay.guild_here' : g.hasPc ? 'relay.guild_other_pc' : 'relay.guild_no_pc')}
                        </span>
                    </span>
                    {(g.here || g.hasPc) && <span className="pill">{t('relay.pc')}</span>}
                </button>
            ))}
        </div>
    );
}

export function DiscordTab({ api, relay }: { api: WebApi; relay: Relay | null }) {
    const { t } = useTranslation();
    const hostId = useApp((s) => s.session.hostId);
    const [guilds, setGuilds] = useState<GuildEntry[] | null>(null);
    const [guildId, setGuildId] = useState<string | null>(null);
    // 릴레이가 서버 목록을 주지 못했다 (끊김·5xx)
    const [relayFailed, setRelayFailed] = useState(false);
    const [round, setRound] = useState(0);

    // biome-ignore lint/correctness/useExhaustiveDependencies: round 는 다시 읽기 신호다
    useEffect(() => {
        let alive = true;
        // 릴레이 세션이 없으면 이 PC 의 서버만 보인다
        Promise.all([
            hostId ? api.relayListHostNodes(hostId, '').catch(() => []) : [],
            relay ? relay.guilds() : null,
        ]).then(([nodes, res]) => {
            if (!alive) return;
            const list = mergeGuilds(
                Array.isArray(nodes) ? (nodes as DiscordNode[]) : [],
                res?.ok && Array.isArray(res.data) ? res.data : [],
            );
            setRelayFailed(!!res && !res.ok);
            setGuilds(list);
            setGuildId((g) => (list.some((x) => x.id === g) ? g : (list[0]?.id ?? null)));
        });
        return () => {
            alive = false;
        };
    }, [api, relay, hostId, round]);

    const guild = guilds?.find((g) => g.id === guildId);
    const here = useMemo(() => relay && hereStore(api, relay), [api, relay]);
    return (
        <Section title={t('tabs.discord')}>
            {relayFailed && (
                <div className="empty">
                    <p>{t('relay.guilds_failed')}</p>
                    <Button
                        onClick={() => {
                            setGuilds(null);
                            setRound((r) => r + 1);
                        }}
                    >
                        {t('relay.retry')}
                    </Button>
                </div>
            )}
            {guilds === null ? (
                <Loading />
            ) : guilds.length === 0 ? (
                !relayFailed && <Empty>{t('relay.no_guilds')}</Empty>
            ) : (
                <div className={guilds.length > 1 ? 'side-layout' : undefined}>
                    {guilds.length > 1 && <GuildChips guilds={guilds} value={guildId} onChange={setGuildId} />}
                    <div className="side-main">
                        {guild?.here && relay && (guild.can?.manage || modAllowed(guild.can)) && (
                            <RelayGuild key={`here-${guild.id}`} relay={here as GuildStore} guild={guild} here />
                        )}
                        {guild?.here && (
                            <GuildPermissions
                                key={guild.id}
                                api={api}
                                guildId={guild.id}
                                siblings={guilds.filter((x) => x.here && x.id !== guild.id).map((x) => x.id)}
                            />
                        )}
                        {guild && !guild.here && guild.hasPc && <Empty>{t('relay.other_pc')}</Empty>}
                        {guild && !guild.here && !guild.hasPc && <RelayGuild key={guild.id} relay={relay as GuildStore} guild={guild} />}
                    </div>
                </div>
            )}
        </Section>
    );
}
