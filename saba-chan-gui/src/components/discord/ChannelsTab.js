import clsx from 'clsx';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaCheckbox, SabaSpinner } from '../ui/SabaUI';

/** 릴레이 계약: 길드당 챗봇 채널 최대 20개, 음악 채널 1개 (assign/contract.md) */
export const CHATBOT_MAX = 20;

export const channelsAvailable = () => Boolean(window.api?.relayNodeChannels && window.api?.relayNodeChannelsSave);

const sameSet = (a, b) => a.length === b.length && a.every((x) => b.includes(x));

/** 바뀐 키만 보낸다: 릴레이 PUT은 준 키만 바꾼다 */
export function channelsPatch(saved, draft) {
    const patch = {};
    if (!sameSet(saved.chatbot, draft.chatbot)) patch.chatbot = draft.chatbot;
    if ((saved.music || null) !== (draft.music || null)) patch.music = draft.music || null;
    return patch;
}

/** 데몬·릴레이 오류 → 한 줄 (error_code 또는 code 또는 error) */
export function channelsErrorText(res, t, nameOf) {
    const code = res?.error_code || res?.code || res?.error;
    switch (code) {
        case 'CHANNEL_CONFLICT':
            return t('discord.channels_err_conflict');
        case 'UNKNOWN_CHANNEL':
            return t('discord.channels_err_unknown', { channel: nameOf(res?.channelId) });
        case 'BOT_UNAVAILABLE':
            return t('discord.channels_err_bot');
        default:
            return res?.message || res?.error || t('discord.channels_err_generic');
    }
}

const fromResponse = (res) => ({
    chatbot: Array.isArray(res?.chatbot) ? res.chatbot.map(String) : [],
    music: res?.music ? String(res.music) : null,
});

/**
 * 디스코드 서버 하나의 채널 배정: 챗봇 채널(여러 개)·음악 채널(하나).
 * 원본은 릴레이다 (GET/PUT /api/relay/node/:guild_id/channels). 한 채널은 한 종류만 맡는다.
 */
export function ChannelsTab({ node }) {
    const { t } = useTranslation('gui');
    const guildId = node.guildId;
    const [data, setData] = useState(null); // 마지막 응답
    const [draft, setDraft] = useState({ chatbot: [], music: null });
    const [loading, setLoading] = useState(true);
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState(null);
    const [armed, setArmed] = useState(false);

    const apply = useCallback((res) => {
        setData(res);
        setDraft(fromResponse(res));
        setArmed(false);
    }, []);

    const load = useCallback(async () => {
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.relayNodeChannels(guildId);
            if (res && !res.error && !res.error_code) apply(res);
            else setError(res || {});
        } catch (e) {
            setError({ message: e.message });
        }
        setLoading(false);
    }, [guildId, apply]);

    useEffect(() => {
        load();
    }, [load]);

    const channels = data?.channels || [];
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

    const saved = fromResponse(data);
    const patch = channelsPatch(saved, draft);
    const dirty = Object.keys(patch).length > 0;
    const tooMany = draft.chatbot.length > CHATBOT_MAX;
    // 음악 채널을 새로 고르면 그 채널의 메시지가 지워진다: 경고하고 두 번 눌러 저장
    const newMusic = patch.music != null && patch.music !== saved.music;

    const toggleChatbot = (id) =>
        setDraft((d) => ({
            ...d,
            chatbot: d.chatbot.includes(id) ? d.chatbot.filter((x) => x !== id) : [...d.chatbot, id],
        }));
    const setMusic = (id) => {
        // 경고를 확인한 채널이 바뀌면 다시 확인받는다
        setArmed(false);
        setDraft((d) => ({
            chatbot: id ? d.chatbot.filter((x) => x !== id) : d.chatbot,
            music: id || null,
        }));
    };

    const save = async () => {
        if (newMusic && !armed) {
            setArmed(true);
            return;
        }
        setSaving(true);
        setError(null);
        try {
            const res = await window.api.relayNodeChannelsSave(guildId, patch);
            if (res && !res.error && !res.error_code) apply(res);
            else setError(res || {});
        } catch (e) {
            setError({ message: e.message });
        }
        setSaving(false);
        setArmed(false);
    };

    if (loading && !data) {
        return (
            <p className="muted pad">
                <SabaSpinner size="sm" /> {t('discord.channels_loading')}
            </p>
        );
    }

    const botAvailable = data?.botAvailable !== false;
    // 봇이 없어 목록이 비어도 지금 배정된 채널은 보여 주고 해제는 할 수 있다 (해제는 봇 없이도 된다)
    const orphans = draft.chatbot.filter((id) => !channels.some((c) => String(c.id) === id));

    return (
        <div className="dc-channels">
            {data && !botAvailable && (
                <div className="notice tone-warn" role="status">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{t('discord.channels_no_bot')}</span>
                </div>
            )}

            <section className="dc-card">
                <h3>{t('discord.channels_chatbot')}</h3>
                <p className="dc-hint">{t('discord.channels_chatbot_hint', { max: CHATBOT_MAX })}</p>
                {groups.map((g) => (
                    <div key={g.key || '_'} className="dc-channel-group">
                        {g.key && <div className="dc-channel-cat">{g.key}</div>}
                        {g.items.map((c) => {
                            const id = String(c.id);
                            const isMusic = draft.music === id;
                            const checked = draft.chatbot.includes(id);
                            // 봇이 없으면 새로 배정할 수 없다 (503 BOT_UNAVAILABLE). 해제만 된다
                            const locked = isMusic || (!botAvailable && !checked);
                            return (
                                <div key={id} className={clsx('dc-channel-row', { disabled: locked })}>
                                    <SabaCheckbox
                                        checked={checked}
                                        disabled={locked}
                                        onChange={() => toggleChatbot(id)}
                                        aria-label={t('discord.channels_chatbot_aria', { channel: c.name })}
                                    />
                                    <span>#{c.name}</span>
                                    {isMusic && <span className="muted">{t('discord.channels_is_music')}</span>}
                                </div>
                            );
                        })}
                    </div>
                ))}
                {orphans.map((id) => (
                    <div key={id} className="dc-channel-row">
                        <SabaCheckbox
                            checked
                            onChange={() => toggleChatbot(id)}
                            aria-label={t('discord.channels_chatbot_aria', { channel: id })}
                        />
                        <code>{id}</code>
                    </div>
                ))}
                {channels.length === 0 && orphans.length === 0 && <p className="muted">{t('discord.channels_none')}</p>}
                {tooMany && (
                    <p className="dc-field-error">
                        <Icon name="alertTriangle" size="xs" /> {t('discord.channels_too_many', { max: CHATBOT_MAX })}
                    </p>
                )}
            </section>

            <section className="dc-card">
                <h3>{t('discord.channels_music')}</h3>
                <p className="dc-hint">{t('discord.channels_music_hint')}</p>
                <select
                    className="input"
                    value={draft.music || ''}
                    aria-label={t('discord.channels_music')}
                    onChange={(e) => setMusic(e.target.value)}
                >
                    <option value="">{t('discord.channels_music_none')}</option>
                    {draft.music && !channels.some((c) => String(c.id) === draft.music) && (
                        <option value={draft.music}>{draft.music}</option>
                    )}
                    {channels.map((c) => (
                        <option
                            key={c.id}
                            value={String(c.id)}
                            disabled={!botAvailable && String(c.id) !== draft.music}
                        >
                            {c.category ? `${c.category} / ` : ''}#{c.name}
                        </option>
                    ))}
                </select>
                {newMusic && (
                    <div className="notice tone-warn" role="alert">
                        <Icon name="alertTriangle" size="sm" />
                        <span>{t('discord.channels_music_warn', { channel: nameOf(draft.music) })}</span>
                    </div>
                )}
            </section>

            {error && (
                <p className="dc-field-error" role="alert">
                    <Icon name="alertTriangle" size="xs" /> {channelsErrorText(error, t, nameOf)}
                </p>
            )}

            <div className={clsx('dc-savebar', { dirty })}>
                <span className="dc-savebar-text">{dirty ? t('discord.unsaved') : ''}</span>
                <button type="button" className="btn btn-ghost" onClick={load} disabled={loading || saving}>
                    <Icon name="refresh" size="sm" /> {t('discord.channels_reload')}
                </button>
                <button
                    type="button"
                    className={clsx('btn', armed ? 'btn-danger' : 'btn-primary')}
                    onClick={save}
                    disabled={!dirty || saving || tooMany}
                >
                    {saving ? <SabaSpinner size="xs" /> : <Icon name="save" size="sm" />}{' '}
                    {armed ? t('discord.channels_save_confirm') : t('discord.channels_save')}
                </button>
            </div>
        </div>
    );
}
