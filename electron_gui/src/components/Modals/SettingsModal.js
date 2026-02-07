import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';

function SettingsModal({ isOpen, onClose, refreshInterval, onRefreshIntervalChange }) {
    const { t, i18n } = useTranslation(['gui', 'common']);
    const [activeTab, setActiveTab] = useState('general');
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);

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

    if (!isOpen) {
        return null;
    }

    return (
        <div className="settings-modal-overlay" onClick={onClose}>
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                <div className="settings-modal-header">
                    <h2><Icon name="settings" size="md" /> {t('gui:settings_modal.title')}</h2>
                    <button className="settings-modal-close" onClick={onClose}>✕</button>
                </div>

                <div className="settings-modal-tabs">
                    <button
                        className={`settings-tab ${activeTab === 'general' ? 'active' : ''}`}
                        onClick={() => setActiveTab('general')}
                    >
                        {t('gui:settings_modal.general')}
                    </button>
                    <button
                        className={`settings-tab ${activeTab === 'appearance' ? 'active' : ''}`}
                        onClick={() => setActiveTab('appearance')}
                    >
                        {t('gui:settings_modal.appearance')}
                    </button>
                    <button
                        className={`settings-tab ${activeTab === 'advanced' ? 'active' : ''}`}
                        onClick={() => setActiveTab('advanced')}
                    >
                        {t('gui:settings_modal.advanced_tab')}
                    </button>
                </div>

                <div className="settings-modal-content">
                    {activeTab === 'general' && (
                        <div className="settings-tab-content">
                            <h3>{t('gui:settings_modal.general')}</h3>
                            
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">🌐 {t('gui:settings_modal.language_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.language_description')}</span>
                                </label>
                                <select 
                                    className="setting-select"
                                    value={selectedLanguage}
                                    onChange={(e) => handleLanguageChange(e.target.value)}
                                >
                                    <option value="en">English (English)</option>
                                    <option value="ko">한국어 (Korean)</option>
                                    <option value="ja">日本語 (Japanese)</option>
                                </select>
                            </div>

                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">🔄 {t('gui:settings_modal.refresh_interval_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.refresh_interval_description')}</span>
                                </label>
                                <select 
                                    className="setting-select"
                                    value={localRefreshInterval}
                                    onChange={(e) => handleRefreshIntervalChange(Number(e.target.value))}
                                >
                                    <option value={1000}>{t('gui:settings_modal.refresh_1s')}</option>
                                    <option value={2000}>{t('gui:settings_modal.refresh_2s')}</option>
                                    <option value={3000}>{t('gui:settings_modal.refresh_3s')}</option>
                                    <option value={5000}>{t('gui:settings_modal.refresh_5s')}</option>
                                    <option value={10000}>{t('gui:settings_modal.refresh_10s')}</option>
                                </select>
                            </div>
                        </div>
                    )}

                    {activeTab === 'appearance' && (
                        <div className="settings-tab-content">
                            <h3>{t('gui:settings_modal.appearance')}</h3>
                            <p>{t('gui:settings_modal.appearance_placeholder')}</p>
                        </div>
                    )}

                    {activeTab === 'advanced' && (
                        <div className="settings-tab-content">
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
