import { useTranslation } from 'react-i18next';
import { type BackupNotice, type DmNoticeKind, useDiscordStore } from '../../stores/useDiscordStore';
import { SabaToggle } from '../ui/SabaUI';
import { Segmented } from '../ui/StatusChip';

// PC 주인 DM 알림 묶음 (사용자 2026-11-08). 백업은 범위(끄기·실패만·모두), 나머지는 켜고 끈다
const DM_GROUPS: { label: string; kinds: DmNoticeKind[] }[] = [
    { label: 'dm_group_server', kinds: ['crash', 'unresponsive', 'externalStart', 'schedule'] },
    { label: 'dm_group_pc', kinds: ['update', 'disk', 'offline'] },
    { label: 'dm_group_guild', kinds: ['chatbotUsage', 'moderation'] },
];
const DM_KEYS: Record<DmNoticeKind, string> = {
    crash: 'dm_crash',
    unresponsive: 'dm_unresponsive',
    externalStart: 'dm_external_start',
    schedule: 'dm_schedule',
    update: 'dm_update',
    disk: 'dm_disk',
    offline: 'dm_offline',
    chatbotUsage: 'dm_chatbot_usage',
    moderation: 'dm_moderation',
};

/** DM 알림 카드 (사바쨩 설정 맨 아래): PC 주인에게 가는 디스코드 DM을 전체·종류별로 켜고 끈다 (데몬 봇 설정 `dmNotice`·`backupNotice`) */
export function DmNoticeCard() {
    const { t } = useTranslation('gui');
    const dmNotice = useDiscordStore((s) => s.discordDmNotice);
    const backupNotice = useDiscordStore((s) => s.discordBackupNotice);
    const toggleRow = (kind: DmNoticeKind) => {
        const key = DM_KEYS[kind];
        return (
            <div className="cb-toggle-row" key={kind}>
                <span className="dc-list-text">
                    <span className="dc-list-name">{t(`discord.${key}_title`)}</span>
                    <span className="cb-hint">{t(`discord.${key}_desc`)}</span>
                    {kind === 'crash' && !dmNotice.crash && (
                        <span className="cb-hint dc-dm-warn">{t('discord.dm_crash_off_warn')}</span>
                    )}
                </span>
                <SabaToggle
                    checked={dmNotice[kind]}
                    aria-label={t(`discord.${key}_title`)}
                    onChange={(on) =>
                        useDiscordStore.getState().update({ discordDmNotice: { ...dmNotice, [kind]: on } })
                    }
                />
            </div>
        );
    };
    return (
        <section className="dc-card">
            <div className="dc-card-head">
                <h2>{t('discord.dm_notice_title')}</h2>
            </div>
            <p className="dc-card-desc">{t('discord.dm_notice_desc')}</p>
            <div className="cb-toggles">
                <div className="cb-toggle-row">
                    <span className="dc-list-text">
                        <span className="dc-list-name">{t('discord.dm_enabled_title')}</span>
                    </span>
                    <SabaToggle
                        checked={dmNotice.enabled}
                        aria-label={t('discord.dm_enabled_title')}
                        onChange={(on) =>
                            useDiscordStore.getState().update({ discordDmNotice: { ...dmNotice, enabled: on } })
                        }
                    />
                </div>
            </div>
            {dmNotice.enabled && DM_GROUPS.map((group, i) => (
                <div className="dc-dm-group" key={group.label}>
                    <span className="dc-dm-group-label">{t(`discord.${group.label}`)}</span>
                    <div className="cb-toggles">
                        {group.kinds.map(toggleRow)}
                        {i === 0 && (
                            <div className="cb-toggle-row">
                                <span className="dc-list-text">
                                    <span className="dc-list-name">{t('discord.dm_backup_title')}</span>
                                    <span className="cb-hint">{t('discord.dm_backup_desc')}</span>
                                </span>
                                <Segmented
                                    ariaLabel={t('discord.dm_backup_title')}
                                    value={backupNotice}
                                    onChange={(v) =>
                                        useDiscordStore.getState().update({ discordBackupNotice: v as BackupNotice })
                                    }
                                    items={[
                                        { id: 'off', label: t('discord.dm_backup_off') },
                                        { id: 'failures', label: t('discord.dm_backup_failures') },
                                        { id: 'all', label: t('discord.dm_backup_all') },
                                    ]}
                                />
                            </div>
                        )}
                    </div>
                </div>
            ))}
        </section>
    );
}
