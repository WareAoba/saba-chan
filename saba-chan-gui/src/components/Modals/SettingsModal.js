import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { getTheme, setTheme as saveTheme } from '../../utils/themeManager';

function SettingsModal({ isOpen, onClose, refreshInterval, onRefreshIntervalChange }) {
    const { t, i18n } = useTranslation(['gui', 'common']);
    const [activeTab, setActiveTab] = useState('general');
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);
    const [selectedTheme, setSelectedTheme] = useState(getTheme());
    const [slideDirection, setSlideDirection] = useState('');
    const tabOrder = ['general', 'appearance', 'advanced'];

    // 탭 전환 핸들러
    const handleTabChange = (newTab) => {
        const oldIndex = tabOrder.indexOf(activeTab);
        const newIndex = tabOrder.indexOf(newTab);
        if (newTab === activeTab) return;
        setSlideDirection(newIndex > oldIndex ? 'slide-left' : 'slide-right');
        setActiveTab(newTab);
    };

    // refreshInterval prop이 변경되면 로컬 상태 업데이트
    useEffect(() => {
        setLocalRefreshInterval(refreshInterval);
    }, [refreshInterval]);

    // 현재 언어 동기화
    useEffect(() => {
        setSelectedLanguage(i18n.language);
    }, [i18n.language]);

    // 리프레시 주기 변경 핸들러
    const handleRefreshIntervalChange = (value) => {
        setLocalRefreshInterval(value);
        if (onRefreshIntervalChange) {
            onRefreshIntervalChange(value);
        }
    };

    // 언어 변경 핸들러
    const handleLanguageChange = (lng) => {
        setSelectedLanguage(lng);
        // 1. localStorage에 저장
        localStorage.setItem('i18nextLng', lng);
        // 2. i18n 언어 변경
        i18n.changeLanguage(lng);
        // 3. Electron 설정에 저장 (settings.json)
        if (window.electron) {
            window.electron.setLanguage(lng).catch((err) => {
                console.error('Failed to save language to Electron settings:', err);
            });
        }
    };

    // 테마 변경 핸들러
    const handleThemeChange = (theme) => {
        setSelectedTheme(theme);
        saveTheme(theme);
    };

    if (!isOpen) {
        return null;
    }

    return (
        <div className="settings-modal-overlay" onClick={onClose}>
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                <div className="settings-modal-header">
                    <h2 style={{ fontSize: '1.3rem' }}>{t('gui:settings_modal.title')}</h2>
                </div>

                <div className="settings-modal-tabs" data-tab={activeTab}>
                    <button
                        className={`settings-tab ${activeTab === 'general' ? 'active' : ''}`}
                        onClick={() => handleTabChange('general')}
                    >
                        {t('gui:settings_modal.general')}
                    </button>
                    <button
                        className={`settings-tab ${activeTab === 'appearance' ? 'active' : ''}`}
                        onClick={() => handleTabChange('appearance')}
                    >
                        {t('gui:settings_modal.appearance')}
                    </button>
                    <button
                        className={`settings-tab ${activeTab === 'advanced' ? 'active' : ''}`}
                        onClick={() => handleTabChange('advanced')}
                    >
                        {t('gui:settings_modal.advanced_tab')}
                    </button>
                </div>

                <div className="settings-modal-content">
                    {activeTab === 'general' && (
                        <div className={`settings-tab-content ${slideDirection}`} key="general" onAnimationEnd={() => setSlideDirection('')}>
                            <h3>{t('gui:settings_modal.general')}</h3>
                            
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title"><Icon name="globe" size="sm" /> {t('gui:settings_modal.language_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.language_description')}</span>
                                </label>
                                <CustomDropdown
                                    className="setting-select"
                                    value={selectedLanguage}
                                    onChange={(val) => handleLanguageChange(val)}
                                    options={[
                                        { value: 'en', label: 'English' },
                                        { value: 'ko', label: '한국어 (Korean)' },
                                        { value: 'ja', label: '日本語 (Japanese)' },
                                        { value: 'zh-CN', label: '简体中文 (Simplified Chinese)' },
                                        { value: 'zh-TW', label: '繁體中文 (Traditional Chinese)' },
                                        { value: 'es', label: 'Español (Spanish)' },
                                        { value: 'pt-BR', label: 'Português (Portuguese - Brazil)' },
                                        { value: 'ru', label: 'Русский (Russian)' },
                                        { value: 'de', label: 'Deutsch (German)' },
                                        { value: 'fr', label: 'Français (French)' },
                                    ]}
                                />
                            </div>

                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title"><Icon name="refresh" size="sm" /> {t('gui:settings_modal.refresh_interval_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.refresh_interval_description')}</span>
                                </label>
                                <CustomDropdown
                                    className="setting-select"
                                    value={localRefreshInterval}
                                    onChange={(val) => handleRefreshIntervalChange(Number(val))}
                                    options={[
                                        { value: 1000, label: t('gui:settings_modal.refresh_1s') },
                                        { value: 2000, label: t('gui:settings_modal.refresh_2s') },
                                        { value: 3000, label: t('gui:settings_modal.refresh_3s') },
                                        { value: 5000, label: t('gui:settings_modal.refresh_5s') },
                                        { value: 10000, label: t('gui:settings_modal.refresh_10s') },
                                    ]}
                                />
                            </div>
                        </div>
                    )}

                    {activeTab === 'appearance' && (
                        <div className={`settings-tab-content ${slideDirection}`} key="appearance" onAnimationEnd={() => setSlideDirection('')}>
                            <h3>{t('gui:settings_modal.appearance')}</h3>
                            
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title"><Icon name="palette" size="sm" /> {t('gui:settings_modal.theme_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.theme_description')}</span>
                                </label>
                                <CustomDropdown
                                    className="setting-select"
                                    value={selectedTheme}
                                    onChange={(val) => handleThemeChange(val)}
                                    options={[
                                        { value: 'auto', label: t('gui:settings_modal.theme_auto'), icon: 'monitor' },
                                        { value: 'light', label: t('gui:settings_modal.theme_light'), icon: 'sun' },
                                        { value: 'dark', label: t('gui:settings_modal.theme_dark'), icon: 'moon' },
                                    ]}
                                />
                            </div>
                        </div>
                    )}

                    {activeTab === 'advanced' && (
                        <div className={`settings-tab-content ${slideDirection}`} key="advanced" onAnimationEnd={() => setSlideDirection('')}>
                            <h3>{t('gui:settings_modal.advanced_tab')}</h3>
                            <p>{t('gui:settings_modal.advanced_placeholder')}</p>
                        </div>
                    )}
                </div>

                <div className="settings-modal-footer">
                    <button className="settings-btn-cancel" onClick={onClose}>
                        {t('gui:modals.cancel')}
                    </button>
                </div>
            </div>
        </div>
    );
}

export default SettingsModal;
