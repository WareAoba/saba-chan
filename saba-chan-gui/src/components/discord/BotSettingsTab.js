import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { LANGUAGE_OPTIONS } from '../../utils/languages';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';
import { defaultPrefix, prefixProblems } from './prefixes';
import { TagInput } from './TagInput';

export const botSettingsAvailable = () => Boolean(window.api?.discordSettingsGet && window.api?.discordSettingsSet);

/** relay nodeSettingsSchema와 같은 범위 (I2 계약) */
export const QUEUE_LINES = { min: 1, max: 20 };
export const REFRESH_SECONDS = { min: 2, max: 10 };
/** 읽어주는 사바쨩 (데몬 `TTS_*`). 속도는 화면에서 배, 저장은 % */
export const TTS_VOLUME = { min: 10, max: 150 };
export const TTS_SPEED = { min: 0.8, max: 1.5 };
export const TTS_PITCH = { min: -6, max: 6 };
/** 이름 부르기 — 데몬 discord.rs TTS_NAMES 와 같다 */
const TTS_NAMES = ['always', 'multi', 'never'];

/** 고정 제한 — 릴레이 bot/tts/speechText.ts MAX_SPEECH_CHARS, speakerRate.ts SPEAKER_RATE_CHARS 를 보여 주기만 한다 */
export const TTS_LIMITS = { chars: 40, perMinute: 400 };

/** 되묻기 단계 (데몬 `CONFIRM_LEVELS`): 모두 / 주의 이상 / 위험만 / 묻지 않음 */
export const CONFIRM_LEVELS = ['all', 'caution', 'danger', 'none'];

const failed = (res) => !res || res.success === false || Boolean(res.error_code);

/** 데몬 오류 → 한 줄 */
export function botSettingsErrorText(res, t) {
    if (res?.error_code === 'not_paired') return t('discord.bot_settings.err_not_paired');
    if (res?.error_code === 'invalid_input') return t('discord.bot_settings.err_invalid', { field: res.field || '' });
    return res?.error || res?.message || t('discord.bot_settings.err_generic');
}

/**
 * 숫자 칸: 범위 안의 정수만 저장하고, 벗어나면 그 자리에 알린다.
 * scale 이 있으면 화면 값 × scale 이 정수인 값만 받는다 (속도 1.25배 → 125). value·onCommit 은 저장 값이다
 */
function NumberField({ label, unit, hint, value, range, scale = 1, disabled, onCommit }) {
    const { t } = useTranslation('gui');
    const shown = String(value / scale);
    const [draft, setDraft] = useState(shown);
    useEffect(() => setDraft(shown), [shown]);
    const n = Number(draft);
    const stored = Math.round(n * scale);
    const valid =
        draft !== '' && Number.isFinite(n) && Math.abs(n * scale - stored) < 1e-9 && n >= range.min && n <= range.max;
    const commit = () => {
        if (valid && stored !== value) onCommit(stored);
    };
    return (
        <label className="mt-field">
            <span>{label}</span>
            <span className="mt-inline">
                <input
                    className="input mt-num"
                    type="number"
                    min={range.min}
                    max={range.max}
                    step={1 / scale}
                    value={draft}
                    disabled={disabled}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={commit}
                    onKeyDown={(e) => e.key === 'Enter' && commit()}
                />
                <span className="muted">{unit}</span>
            </span>
            {hint && <span className="cb-hint">{hint}</span>}
            {!valid && (
                <span className="dc-field-error">
                    {t(scale === 1 ? 'discord.bot_settings.range' : 'discord.bot_settings.range_decimal', {
                        min: range.min,
                        max: range.max,
                    })}
                </span>
            )}
        </label>
    );
}

/**
 * 접두사 칸: 고치는 즉시 저장한다. 저장할 수 없는 목록은 저장하지 않고 화면에만 두고 이유를 보인다.
 * 저장이 끝나기 전에 또 고치면, 앞 저장의 답이 화면의 목록을 되돌리지 않게 저장 중에는 받은 값으로 덮지 않는다
 */
function PrefixField({ value, fallback, disabled, onSave }) {
    const { t } = useTranslation('gui');
    const [draft, setDraft] = useState(value);
    const pending = useRef(0);
    const valueKey = JSON.stringify(value);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 저장된 목록이 바뀔 때만 (valueKey)
    useEffect(() => {
        if (pending.current === 0) setDraft(value);
    }, [valueKey]);
    const problems = prefixProblems(draft, t);
    return (
        <>
            <TagInput
                value={draft}
                onChange={(next) => {
                    if (disabled) return;
                    setDraft(next);
                    if (prefixProblems(next, t).length > 0) return;
                    pending.current += 1;
                    Promise.resolve(onSave(next)).finally(() => {
                        pending.current -= 1;
                    });
                }}
                placeholder={t('discord.prefix_default_placeholder', { prefix: fallback })}
                ariaLabel={t('discord.prefix_title')}
            />
            {draft.length === 0 && <p className="dc-hint">{t('discord.prefix_default_hint', { prefix: fallback })}</p>}
            {problems.length > 0 && (
                <p className="dc-field-error">
                    <Icon name="alertTriangle" size="xs" /> {problems.join(' · ')}
                </p>
            )}
        </>
    );
}

/**
 * 디스코드 서버 하나의 봇 설정 (데몬 GET/PATCH /api/discord/:guild/settings). 원본은 데몬 config.json이고,
 * 데몬이 relay로 보낸다. 바꾸는 즉시 저장한다.
 * - section 'bot': 언어·접두사·명령 이름 번역·되묻기·사용법
 * - section 'music': 노래하는 사바쨩
 * - section 'tts': 읽어주는 사바쨩 볼륨·속도·피치와 이름 부르기 (2026-10-31 사용자 지정, 글자 제한은 고정)
 * 말하는 사바쨩의 되묻기 단계는 챗봇 설정의 이 서버 값(`guilds[guild].confirmLevel`)이다
 */
export function BotSettingsTab({ node, section = 'bot', sampleModuleWord = '' }) {
    const { t } = useTranslation('gui');
    const guildId = node.guildId;
    const [settings, setSettings] = useState(null);
    const [sync, setSync] = useState(null);
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState(null);
    const [chatbotConfirm, setChatbotConfirm] = useState(null);

    const loadChatbot = useCallback(async () => {
        if (!window.api?.chatbotGet) return;
        try {
            const res = await window.api.chatbotGet();
            if (failed(res)) return;
            setChatbotConfirm(
                res.guilds?.[guildId]?.confirmLevel ??
                    res.settings?.confirmLevel ??
                    res.defaults?.confirmLevel ??
                    'all',
            );
        } catch {
            // 챗봇 설정을 못 읽으면 그 칸만 숨긴다
        }
    }, [guildId]);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.discordSettingsGet(guildId);
            if (failed(res) || res.error) setError(botSettingsErrorText(res, t));
            else {
                setSettings(res);
                setSync(res.sync || null);
            }
        } catch (e) {
            setError(e.message);
        }
        setLoading(false);
    }, [guildId, t]);

    useEffect(() => {
        load();
        loadChatbot();
    }, [load, loadChatbot]);

    const save = async (patch) => {
        setSaving(true);
        try {
            const res = await window.api.discordSettingsSet(guildId, patch);
            if (failed(res)) safeShowToast(botSettingsErrorText(res, t), 'error', 6000);
            else {
                // 응답에 릴레이 원본 값(큰 이모지)이 빠질 수 있어 지금 값 위에 얹는다
                if (res.settings) setSettings((prev) => ({ ...prev, ...res.settings }));
                setSync(res.sync || null);
            }
        } catch (e) {
            safeShowToast(e.message, 'error', 6000);
        }
        setSaving(false);
    };

    const saveChatbotConfirm = async (confirmLevel) => {
        setSaving(true);
        try {
            const res = await window.api.chatbotSave({ guilds: { [guildId]: { confirmLevel } } });
            if (failed(res)) safeShowToast(botSettingsErrorText(res, t), 'error', 6000);
            else setChatbotConfirm(confirmLevel);
        } catch (e) {
            safeShowToast(e.message, 'error', 6000);
        }
        setSaving(false);
    };

    if (loading && !settings) {
        return (
            <p className="muted pad">
                <SabaSpinner size="sm" /> {t('discord.bot_settings.loading')}
            </p>
        );
    }
    if (!settings) {
        return (
            <div className="setting-validation-error" role="alert">
                <Icon name="alertCircle" size="sm" /> {error}
            </div>
        );
    }

    const music = settings.music || {};
    const tts = settings.tts || {};
    const prefixes = Array.isArray(settings.prefixes) ? settings.prefixes : [];
    // 릴레이가 알려 준 이 서버의 기본 접두사 (모르면 앱 언어의 기본값)
    const fallbackPrefix = defaultPrefix(t, [node]);
    const p = prefixes[0] || fallbackPrefix;
    // 릴레이에서 설정을 못 읽었으면 기본값만 보인다. 그대로 저장하면 릴레이 값을 덮으므로 막는다
    const unloaded = settings.loaded === false;
    const locked = saving || unloaded;
    const toggleRow = (label, hint, checked, onChange) => (
        <div className="cb-toggle-row">
            <span className="dc-list-text">
                <span className="dc-list-name">{label}</span>
                {hint && <span className="cb-hint">{hint}</span>}
            </span>
            <SabaToggle checked={!!checked} disabled={locked} aria-label={label} onChange={onChange} />
        </div>
    );

    const confirmField = (label, hint, value, onChange) => (
        <div className="mt-field">
            <span>{label}</span>
            <CustomDropdown
                className="setting-select"
                value={value}
                ariaLabel={label}
                disabled={locked}
                onChange={onChange}
                options={CONFIRM_LEVELS.map((l) => ({ value: l, label: t(`chatbot.confirm_${l}`) }))}
            />
            <p className="dc-hint">{hint}</p>
        </div>
    );

    return (
        <div className="dc-bot-settings">
            {unloaded && (
                <output className="notice tone-warn">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('discord.bot_settings.not_loaded')}</span>
                    <button type="button" className="btn btn-ghost btn-sm" disabled={loading} onClick={load}>
                        {t('maintenance.retry')}
                    </button>
                </output>
            )}
            {sync && sync.ok === false && (
                <output className="notice tone-warn">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('discord.bot_settings.not_synced')}</span>
                </output>
            )}

            {section === 'bot' && (
                <>
                    <section className="dc-card">
                        <div className="dc-card-head">
                            <h2>{t('discord.bot_settings.general_title')}</h2>
                        </div>
                        <div className="mt-field">
                            <span>{t('discord.bot_settings.language')}</span>
                            <CustomDropdown
                                className="setting-select"
                                value={settings.locale || 'auto'}
                                disabled={locked}
                                onChange={(locale) => locale !== settings.locale && save({ locale })}
                                options={[
                                    { value: 'auto', label: t('discord.bot_settings.language_auto') },
                                    ...LANGUAGE_OPTIONS,
                                ]}
                            />
                        </div>
                        {/* 큰 이모지: 원본은 서버쨩(릴레이). 릴레이를 못 읽으면 값이 없어 보이지 않는다 */}
                        {typeof settings.bigEmoji === 'boolean' &&
                            toggleRow(
                                t('discord.bot_settings.big_emoji'),
                                t('discord.bot_settings.big_emoji_desc'),
                                settings.bigEmoji,
                                (bigEmoji) => save({ bigEmoji }),
                            )}
                    </section>

                    <section className="dc-card">
                        <div className="dc-card-head">
                            <h2>{t('discord.prefix_title')}</h2>
                        </div>
                        <p className="dc-card-desc">{t('discord.prefix_desc')}</p>
                        <PrefixField
                            value={prefixes}
                            fallback={fallbackPrefix}
                            disabled={unloaded}
                            onSave={(next) => save({ prefixes: next })}
                        />
                        {toggleRow(
                            t('discord.translate_commands_title'),
                            t('discord.translate_commands_desc'),
                            settings.translateCommands !== false,
                            (translateCommands) => save({ translateCommands }),
                        )}
                    </section>

                    <section className="dc-card">
                        <div className="dc-card-head">
                            <h2>{t('discord.slash_title')}</h2>
                        </div>
                        <p className="dc-card-desc">{t('discord.slash_desc')}</p>
                        {toggleRow(
                            t('discord.slash_prefix_title'),
                            t('discord.slash_prefix_desc'),
                            settings.slashPrefix !== false,
                            (slashPrefix) => save({ slashPrefix }),
                        )}
                        <div className="mt-field">
                            <span>{t('discord.slash_root_name_title')}</span>
                            <CustomDropdown
                                className="setting-select"
                                value={settings.slashRootName === 'word' ? 'word' : 'name'}
                                ariaLabel={t('discord.slash_root_name_title')}
                                disabled={locked}
                                onChange={(slashRootName) => save({ slashRootName })}
                                options={['name', 'word'].map((v) => ({
                                    value: v,
                                    label: t(`discord.slash_root_name_${v}`),
                                }))}
                            />
                            <p className="dc-hint">{t('discord.slash_root_name_desc')}</p>
                        </div>
                        {toggleRow(
                            t('discord.private_queries_title'),
                            t('discord.private_queries_desc'),
                            settings.privateQueries === true,
                            (privateQueries) => save({ privateQueries }),
                        )}
                    </section>

                    <section className="dc-card">
                        <div className="dc-card-head">
                            <h2>{t('discord.bot_settings.confirm_title')}</h2>
                        </div>
                        <p className="dc-card-desc">{t('discord.bot_settings.confirm_desc')}</p>
                        {confirmField(
                            t('discord.bot_settings.confirm_direct'),
                            t('discord.bot_settings.confirm_direct_hint'),
                            settings.confirmLevel || 'danger',
                            (confirmLevel) => confirmLevel !== settings.confirmLevel && save({ confirmLevel }),
                        )}
                        {chatbotConfirm &&
                            window.api?.chatbotSave &&
                            confirmField(
                                t('discord.bot_settings.confirm_chatbot'),
                                t('discord.bot_settings.confirm_chatbot_hint'),
                                chatbotConfirm,
                                (level) => level !== chatbotConfirm && saveChatbotConfirm(level),
                            )}
                    </section>

                    <section className="dc-card">
                        <div className="dc-card-head">
                            <h2>{t('discord.usage_title')}</h2>
                        </div>
                        <ul className="dc-usage">
                            <li>
                                <code>{p}</code>
                                <span>{t('discord.usage_help')}</span>
                            </li>
                            <li>
                                <code>{`${p} ${t('discord.usage_list_word')}`}</code>
                                <span>{t('discord.usage_list')}</span>
                            </li>
                            <li>
                                <code>{`${p} ${sampleModuleWord || t('discord.usage_module_word')} ${t('discord.usage_start_word')}`}</code>
                                <span>{t('discord.usage_command')}</span>
                            </li>
                        </ul>
                    </section>
                </>
            )}
            {section === 'music' && (
                <section className="dc-card">
                    {toggleRow(t('discord.bot_settings.music_enabled'), null, music.enabled, (enabled) =>
                        save({ music: { enabled } }),
                    )}
                    {toggleRow(
                        t('discord.bot_settings.music_ui'),
                        t('discord.bot_settings.music_ui_hint'),
                        music.ui,
                        (ui) => save({ music: { ui } }),
                    )}
                    {toggleRow(
                        t('discord.bot_settings.music_normalize'),
                        t('discord.bot_settings.music_normalize_hint'),
                        music.normalize,
                        (normalize) => save({ music: { normalize } }),
                    )}
                    <div className="cb-fields">
                        <NumberField
                            label={t('discord.bot_settings.queue_lines')}
                            unit={t('discord.bot_settings.lines')}
                            value={music.queueLines ?? 10}
                            range={QUEUE_LINES}
                            disabled={locked}
                            onCommit={(queueLines) => save({ music: { queueLines } })}
                        />
                        <NumberField
                            label={t('discord.bot_settings.refresh')}
                            unit={t('chatbot.seconds')}
                            value={Math.round((music.refreshInterval ?? 5000) / 1000)}
                            range={REFRESH_SECONDS}
                            disabled={locked}
                            onCommit={(s) => save({ music: { refreshInterval: s * 1000 } })}
                        />
                    </div>
                </section>
            )}
            {section === 'tts' && (
                <section className="dc-card">
                    <div className="cb-fields">
                        <NumberField
                            label={t('discord.bot_settings.tts_volume')}
                            hint={t('discord.bot_settings.tts_volume_hint')}
                            unit="%"
                            value={tts.volume ?? 100}
                            range={TTS_VOLUME}
                            disabled={locked}
                            onCommit={(volume) => save({ tts: { volume } })}
                        />
                        <NumberField
                            label={t('discord.bot_settings.tts_speed')}
                            unit={t('discord.bot_settings.tts_speed_unit')}
                            value={tts.speed ?? 100}
                            range={TTS_SPEED}
                            scale={100}
                            disabled={locked}
                            onCommit={(speed) => save({ tts: { speed } })}
                        />
                        <NumberField
                            label={t('discord.bot_settings.tts_pitch')}
                            unit={t('discord.bot_settings.tts_pitch_unit')}
                            value={tts.pitch ?? 0}
                            range={TTS_PITCH}
                            disabled={locked}
                            onCommit={(pitch) => save({ tts: { pitch } })}
                        />
                    </div>
                    <div className="mt-field">
                        <span>{t('discord.bot_settings.tts_names')}</span>
                        <Segmented
                            items={TTS_NAMES.map((id) => ({ id, label: t(`discord.bot_settings.tts_names_${id}`) }))}
                            value={TTS_NAMES.includes(tts.names) ? tts.names : 'multi'}
                            onChange={(names) => !locked && save({ tts: { names } })}
                            ariaLabel={t('discord.bot_settings.tts_names')}
                        />
                        <p className="dc-hint">{t('discord.bot_settings.tts_names_desc')}</p>
                    </div>
                    <p className="cb-hint">{t('discord.bot_settings.tts_limits', TTS_LIMITS)}</p>
                </section>
            )}
        </div>
    );
}
