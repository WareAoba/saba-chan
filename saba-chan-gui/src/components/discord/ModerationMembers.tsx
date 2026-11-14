import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { Avatar } from './Avatar';
import { durationText } from './modDuration';
import { ActionNote, memberName, modErrorText, type ModState, Section, useAction } from './ModerationParts';
import type { DiscordMember } from './types';

/* 서버 관리 탭 › 멤버 화면: 음소거하기와 음소거 목록 */

export const MUTE_MINUTES = [10, 60, 360, 1440, 10080, 40320];

interface MembersViewProps {
    guildId: string;
    mod: ModState;
    reload: () => void;
    members: DiscordMember[];
    membersLoading?: boolean;
    onRefreshMembers?: () => void;
}

export function MembersView({ guildId, mod, reload, members, membersLoading, onRefreshMembers }: MembersViewProps) {
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
    const send = (userId: string, body: ApiBody, done?: () => void) =>
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
        <div data-spot="mod.members" className="dc-mod-members">
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