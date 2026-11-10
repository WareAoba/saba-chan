import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../../stores/useSettingsStore';
import { LANGUAGE_OPTIONS } from '../../../utils/languages';
import CustomDropdown from '../../CustomDropdown/CustomDropdown';
import { Icon } from '../../Icon';
import { SabaToggle } from '../../ui/SabaUI';
import { GUIDE_ENTRIES, openGuide } from '../../guides/guides';
import { AutostartSetting, autostartAvailable } from '../AutostartSetting';
import { DesktopOverlaySetting } from '../DesktopOverlaySetting';
import { setOption } from './options';

interface GeneralTabProps {
    selectedLanguage: string;
    handleLanguageChange: (lng: string) => void;
}

/** 설정 › 일반: 가이드 다시 열기·언어·자동 새로고침·창 닫기·자동 시작·바탕화면 오버레이 */
export function GeneralTab({ selectedLanguage, handleLanguageChange }: GeneralTabProps) {
    const { t } = useTranslation(['gui', 'common']);
    const refreshInterval = useSettingsStore((s) => s.refreshInterval);
    const closeAction = useSettingsStore((s) => s.closeAction);
    const autoRefresh = useSettingsStore((s) => s.autoRefresh);
    return (
        <>
            {/* 닫은 가이드를 다시 여는 곳 (사용자 11-10: 상단 바 대신 여기와 명령 팔레트) */}
            <div className="setting-item setting-guides">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="lightbulb" size="sm" /> {t('gui:guides.title')}
                    </span>
                </label>
                <ul className="guide-list">
                    {GUIDE_ENTRIES.map((g) => (
                        <li key={g.id}>
                            <Icon name={g.icon} size="sm" />
                            <span className="guide-list-text">
                                <strong>{t(`gui:guides.entry_${g.id}`)}</strong>
                                <span>{t(`gui:guides.entry_${g.id}_desc`)}</span>
                            </span>
                            <button type="button" className="btn btn-sm" onClick={() => openGuide(g.id)}>
                                {t('gui:guides.open')}
                            </button>
                        </li>
                    ))}
                </ul>
            </div>

            <div className="setting-item">
                <label className="setting-label">
                    <span className="setting-title">
                        <Icon name="globe" size="sm" /> {t('gui:settings_modal.language_label')}
                    </span>
                    <span className="setting-description">
                        {t('gui:settings_modal.language_description')}
                    </span>
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
                    <span className="setting-description">
                        {t('gui:settings_modal.auto_refresh_description')}
                    </span>
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
                        <Icon name="refresh" size="sm" />{' '}
                        {t('gui:settings_modal.refresh_interval_label')}
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
                    <span className="setting-description">
                        {t('gui:settings_modal.close_action_description')}
                    </span>
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

            {/* 바탕화면 오버레이: 그 slot을 쓰는 익스텐션이 켜져 있을 때만 */}
            <DesktopOverlaySetting />
        </>
    );
}
