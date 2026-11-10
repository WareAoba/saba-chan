import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../../stores/useSettingsStore';
import { LANGUAGE_OPTIONS } from '../../../utils/languages';
import CustomDropdown from '../../CustomDropdown/CustomDropdown';
import { Icon } from '../../Icon';
import { SabaToggle } from '../../ui/SabaUI';
import { AutostartSetting, autostartAvailable } from '../AutostartSetting';
import { setOption } from './options';

interface GeneralTabProps {
    selectedLanguage: string;
    handleLanguageChange: (lng: string) => void;
}

/** 설정 › 일반: 언어·자동 새로고침·창 닫기·자동 시작 */
export function GeneralTab({ selectedLanguage, handleLanguageChange }: GeneralTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    const refreshInterval = useSettingsStore((s) => s.refreshInterval);
    const closeAction = useSettingsStore((s) => s.closeAction);
    const autoRefresh = useSettingsStore((s) => s.autoRefresh);
    return (
        <section className="saba-card">
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="globe" size="sm" /> {t('gui:settings_modal.language_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.language_description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={selectedLanguage}
                    onChange={(val) => handleLanguageChange(val)}
                    options={LANGUAGE_OPTIONS}
                />
            </div>

            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="refresh" size="sm" /> {t('gui:settings_modal.auto_refresh_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.auto_refresh_description')}</span>
                </label>
                <SabaToggle
                    checked={autoRefresh}
                    onChange={(checked) => useSettingsStore.getState().update({ autoRefresh: checked })}
                />
            </div>

            {autoRefresh && (
                <div className="setting-item">
                    <label className="setting-label">
                        <span className="setting-title">
                            <Icon name="refresh" size="sm" /> {t('gui:settings_modal.refresh_interval_label')}
                        </span>
                        <span className="setting-description">
                            {t('gui:settings_modal.refresh_interval_description')}
                        </span>
                    </label>
                    <CustomDropdown
                        className="setting-select"
                        value={refreshInterval}
                        onChange={(val) => setOption({ refreshInterval: Number(val) })}
                        options={[
                            { value: 1000, label: t('gui:settings_modal.refresh_1s') },
                            { value: 2000, label: t('gui:settings_modal.refresh_2s') },
                            { value: 3000, label: t('gui:settings_modal.refresh_3s') },
                            { value: 5000, label: t('gui:settings_modal.refresh_5s') },
                            { value: 10000, label: t('gui:settings_modal.refresh_10s') },
                        ]}
                    />
                </div>
            )}

            {/* 창 닫기 버튼: 매번 묻기 / 트레이로 / 사바쨩까지 끄기 */}
            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="close" size="sm" /> {t('gui:settings_modal.close_action_label')}
                    </span>
                    <span className="setting-description">{t('gui:settings_modal.close_action_description')}</span>
                </label>
                <CustomDropdown
                    className="setting-select"
                    value={closeAction}
                    onChange={(val) => setOption({ closeAction: val })}
                    options={['ask', 'tray', 'quit'].map((value) => ({
                        value,
                        label: t(`gui:settings_modal.close_action_${value}`),
                    }))}
                />
            </div>

            {autostartAvailable() && <AutostartSetting />}
        </section>
    );
}
