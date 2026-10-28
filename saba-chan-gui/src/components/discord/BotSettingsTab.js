import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { LANGUAGE_OPTIONS } from '../../utils/languages';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';

export const botSettingsAvailable = () => Boolean(window.api?.discordSettingsGet && window.api?.discordSettingsSet);

/** relay nodeSettingsSchema와 같은 범위 (I2 계약) */
export const QUEUE_LINES = { min: 1, max: 20 };
export const REFRESH_SECONDS = { min: 2, max: 10 };

/** 되묻기 단계 (데몬 `CONFIRM_LEVELS`): 모두 / 주의 이상 / 위험만 / 묻지 않음 */
export const CONFIRM_LEVELS = ['all', 'caution', 'danger', 'none'];

const failed = (res) => !res || res.success === false || Boolean(res.error_code);

/** 데몬 오류 → 한 줄 */
export function botSettingsErrorText(res, t) {
    if (res?.error_code === 'not_paired') return t('discord.bot_settings.err_not_paired');
    if (res?.error_code === 'invalid_input') return t('discord.bot_settings.err_invalid', { field: res.field || '' });
    return res?.error || res?.message || t('discord.bot_settings.err_generic');
}

/** 숫자 칸: 범위 안의 정수만 저장하고, 벗어나면 그 자리에 알린다 */
function NumberField({ label, unit, value, range, disabled, onCommit }) {
    const { t } = useTranslation('gui');
    const [draft, setDraft] = useState(String(value));
    useEffect(() => setDraft(String(value)), [value]);
    const n = Number(draft);
    const valid = draft !== '' && Number.isInteger(n) && n >= range.min && n <= range.max;
    const commit = () => {
        if (valid && n !== value) onCommit(n);
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
                    value={draft}
                    disabled={disabled}
                    onChange={(e) => setDraft(e.target.value)}
                    onBlur={commit}
                    onKeyDown={(e) => e.key === 'Enter' && commit()}
                />
                <span className="muted">{unit}</span>
            </span>
            {!valid && (
                <span className="dc-field-error">
                    {t('discord.bot_settings.range', { min: range.min, max: range.max })}
                </span>
            )}
        </label>
    );
}

/**
 * 디스코드 서버 하나의 봇 취향 설정: 언어·잠자기·음악·되묻기 (데몬 GET/PATCH /api/discord/:guild/settings).
 * 원본은 데몬 config.json이고, 데몬이 relay로 보낸다. 바꾸는 즉시 저장한다.
 * 말하는 사바쨩의 되묻기 단계는 챗봇 설정의 이 서버 값(`guilds[guild].confirmLevel`)이다
 */
export function BotSettingsTab({ node }) {
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
                if (res.settings) setSettings(res.settings);
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
                    <h2>{t('discord.bot_settings.music_title')}</h2>
                </div>
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
        </div>
    );
}
