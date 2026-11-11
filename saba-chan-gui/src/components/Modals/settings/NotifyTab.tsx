import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../../stores/useSettingsStore';
import type { SoundSettings } from '../../../types/daemon';
import CustomDropdown from '../../CustomDropdown/CustomDropdown';
import { Icon } from '../../Icon';
import { SabaToggle } from '../../ui/SabaUI';
import { setOption } from './options';

// 효과음 종류별 스위치 (DEFAULT_SOUNDS 키 → 이름)
const SOUND_KINDS: [Exclude<keyof SoundSettings, 'enabled'>, string][] = [
    ['serverStart', 'gui:settings_modal.sound_server_start'],
    ['serverStop', 'gui:settings_modal.sound_server_stop'],
    ['crash', 'gui:settings_modal.sound_crash'],
    ['install', 'gui:settings_modal.sound_install'],
    ['update', 'gui:settings_modal.sound_update'],
    ['startup', 'gui:settings_modal.sound_startup'],
];

/** 설정 › 알림: 토스트 시간·OS 알림·효과음 */
export function NotifyTab() {
    const { t } = useTranslation(['gui', 'common']);
    const osNotifications = useSettingsStore((s) => s.osNotifications);
    const sounds = useSettingsStore((s) => s.sounds);
    const toastDuration = useSettingsStore((s) => s.toastDuration);
    const setSound = (key: keyof SoundSettings, on: boolean) =>
        useSettingsStore.getState().update({ sounds: { ...sounds, [key]: on } });
    return (
        <section className="saba-card">
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="messageSquare" size="sm" /> {t('gui:settings_modal.toast_duration_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.toast_duration_description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={toastDuration}
                    onChange={(val) => setOption({ toastDuration: val })}
                    options={['short', 'normal', 'long'].map((value) => ({
                        value,
                        label: t(`gui:settings_modal.toast_duration_${value}`),
                    }))}
                />
            </div>

            {/* 중대한 알림을 OS 알림으로도 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="bell" size="sm" /> {t('gui:settings_modal.os_notifications_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.os_notifications_description')}</span>
                </label>
                <SabaToggle
                    checked={osNotifications}
                    onChange={(checked) => useSettingsStore.getState().update({ osNotifications: checked })}
                />
            </div>

            {/* 밖에서 켜고 끈 것·크래시·설치 결과·업데이트 효과음: 전체 스위치 + 종류별 */}
            <div data-spot="settings.sound" className="setting-item setting-item-group">
                <div className="setting-item-row">
                    <label className="setting-label">
                        <span className="setting-title">
                            <Icon name="volume" size="sm" /> {t('gui:settings_modal.sound_effects_label')}
                        </span>
                        <span className="setting-description">{t('gui:settings_modal.sound_effects_description')}</span>
                    </label>
                    <SabaToggle checked={sounds.enabled} onChange={(on) => setSound('enabled', on)} />
                </div>
                {sounds.enabled && (
                    <div className="setting-sublist">
                        {SOUND_KINDS.map(([key, label]) => (
                            <div className="setting-subitem" key={key}>
                                <span>{t(label)}</span>
                                <SabaToggle size="sm" checked={sounds[key]} onChange={(on) => setSound(key, on)} />
                            </div>
                        ))}
                    </div>
                )}
            </div>
        </section>
    );
}
