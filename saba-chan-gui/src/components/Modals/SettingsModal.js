import React, { useState, useEffect, useCallback, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { getTheme, setTheme as saveTheme } from '../../utils/themeManager';
import { useModalClose } from '../../hooks/useModalClose';
import { useDevMode } from '../../hooks/useDevMode';
import { useExtensions } from '../../contexts/ExtensionContext';
import SabaStorage from './SabaStorage';

function SettingsModal({ isOpen, onClose, refreshInterval, onRefreshIntervalChange, ipcPort, onIpcPortChange, consoleBufferSize, onConsoleBufferSizeChange, onTestModal, onTestProgressBar, onTestWaitingImage, onTestLoadingScreen, initialView }) {
    const { t, i18n } = useTranslation(['gui', 'common']);
    const [activeTab, setActiveTab] = useState('general');
    const [showUpdatePanel, setShowUpdatePanel] = useState(false);
    const [updatePanelExiting, setUpdatePanelExiting] = useState(false);
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);
    const [selectedTheme, setSelectedTheme] = useState(getTheme());
    const [slideDirection, setSlideDirection] = useState('');
    const [guiTestOpen, setGuiTestOpen] = useState(false);
    const [localIpcPort, setLocalIpcPort] = useState(ipcPort || 57474);
    const [ipcPortChanged, setIpcPortChanged] = useState(false);
    const [ipcPortError, setIpcPortError] = useState('');
    const [localConsoleBuffer, setLocalConsoleBuffer] = useState(consoleBufferSize || 2000);
    const tabOrder = ['general', 'appearance', 'extensions', 'advanced'];
    const { isClosing, requestClose } = useModalClose(onClose);

    // ── Dynamic tab indicator ──
    const tabsRef = useRef(null);
    const indicatorRef = useRef(null);

    const syncIndicator = useCallback(() => {
        const container = tabsRef.current;
        const indicator = indicatorRef.current;
        if (!container || !indicator) return;
        const activeBtn = container.querySelector('.settings-tab.active');
        if (!activeBtn) return;
        const containerRect = container.getBoundingClientRect();
        const btnRect = activeBtn.getBoundingClientRect();
        indicator.style.left = `${btnRect.left - containerRect.left}px`;
        indicator.style.width = `${btnRect.width}px`;
    }, []);

    useEffect(() => {
        requestAnimationFrame(syncIndicator);
    }, [activeTab, showUpdatePanel, isOpen, syncIndicator]);

    useEffect(() => {
        window.addEventListener('resize', syncIndicator);
        return () => window.removeEventListener('resize', syncIndicator);
    }, [syncIndicator]);
    const devMode = useDevMode();
    const { extensions, toggleExtension } = useExtensions();
    const [togglingIds, setTogglingIds] = useState(new Set());

    const handleExtensionToggle = useCallback(async (extId, enable) => {
        setTogglingIds(prev => new Set(prev).add(extId));
        await toggleExtension(extId, enable);
        setTogglingIds(prev => { const s = new Set(prev); s.delete(extId); return s; });
    }, [toggleExtension]);

    // 외부에서 initialView 지정으로 열렸을 때 해당 탭 자동 진입
    useEffect(() => {
        if (isOpen && initialView === 'update') {
            setActiveTab('general');
            setShowUpdatePanel(true);
        } else if (isOpen && initialView === 'extensions') {
            setActiveTab('extensions');
        }
    }, [isOpen, initialView]);

    // 모달이 닫힐 때 업데이트 패널 초기화
    useEffect(() => {
        if (!isOpen) {
            setShowUpdatePanel(false);
        }
    }, [isOpen]);

    // 탭 전환 핸들러
    const handleTabChange = (newTab) => {
        const oldIndex = tabOrder.indexOf(activeTab);
        const newIndex = tabOrder.indexOf(newTab);
        if (newTab === activeTab) return;
        setShowUpdatePanel(false);  // 탭 전환 시 업데이트 패널 해제
        setSlideDirection(newIndex > oldIndex ? 'slide-left' : 'slide-right');
        setActiveTab(newTab);
    };

    // 업데이트 패널 뒤로가기 (나가기 애니메이션 후 전환)
    const handleUpdatePanelBack = useCallback(() => {
        setUpdatePanelExiting(true);
        setTimeout(() => {
            setUpdatePanelExiting(false);
            setShowUpdatePanel(false);
            setSlideDirection('slide-right');
        }, 150);
    }, []);

    // refreshInterval prop이 변경되면 로컬 상태 업데이트
    useEffect(() => {
        setLocalRefreshInterval(refreshInterval);
    }, [refreshInterval]);

    // ipcPort prop 동기화
    useEffect(() => {
        setLocalIpcPort(ipcPort || 57474);
        setIpcPortChanged(false);
        setIpcPortError('');
    }, [ipcPort]);

    // consoleBufferSize prop 동기화
    useEffect(() => {
        setLocalConsoleBuffer(consoleBufferSize || 2000);
    }, [consoleBufferSize]);

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

    // IPC 포트 변경 핸들러
    const handleIpcPortChange = (value) => {
        const port = parseInt(value, 10);
        setLocalIpcPort(value);
        if (isNaN(port) || port < 1024 || port > 65535) {
            setIpcPortError(t('gui:settings_modal.ipc_port_invalid'));
            return;
        }
        setIpcPortError('');
        setIpcPortChanged(port !== (ipcPort || 57474));
        if (onIpcPortChange) {
            onIpcPortChange(port);
        }
    };

    // 콘솔 버퍼 크기 변경 핸들러
    const handleConsoleBufferChange = (value) => {
        setLocalConsoleBuffer(value);
        if (onConsoleBufferSizeChange) {
            onConsoleBufferSizeChange(value);
        }
    };

    if (!isOpen) {
        return null;
    }

    return (
        <div className={`settings-modal-overlay ${isClosing ? 'closing' : ''}`} onClick={requestClose}>
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                {!showUpdatePanel && (
                    <>
                        <div className="settings-modal-header">
                            <h2 style={{ fontSize: '1.3rem' }}>{t('gui:settings_modal.title')}</h2>
                        </div>

                        <div className="settings-modal-tabs" ref={tabsRef}>
                            <div className="settings-tab-indicator" ref={indicatorRef} />
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
                                className={`settings-tab ${activeTab === 'extensions' ? 'active' : ''}`}
                                onClick={() => handleTabChange('extensions')}
                            >
                                {t('gui:settings_modal.extensions_tab', 'Extensions')}
                            </button>
                            <button
                                className={`settings-tab ${activeTab === 'advanced' ? 'active' : ''}`}
                                onClick={() => handleTabChange('advanced')}
                            >
                                {t('gui:settings_modal.advanced_tab')}
                            </button>
                        </div>
                    </>
                )}

                <div className={`settings-modal-content ${showUpdatePanel ? 'update-panel-mode' : ''}`}>
                    {activeTab === 'general' && !showUpdatePanel && (
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

                            {/* 사바 스토리지 — 클릭하면 SabaStorage로 전환 */}
                            <div className="setting-item setting-item-clickable" onClick={() => setShowUpdatePanel(true)}>
                                <label className="setting-label">
                                    <span className="setting-title"><Icon name="package" size="sm" /> {t('saba_storage.title', '사바 스토리지')}</span>
                                    <span className="setting-description">{t('saba_storage.setting_description', '업데이트, 모듈, 익스텐션 관리')}</span>
                                </label>
                                <Icon name="chevronRight" size="sm" color="var(--brand-primary)" />
                            </div>
                        </div>
                    )}

                    {activeTab === 'general' && showUpdatePanel && (
                        <div className="settings-tab-content" key="update-panel">
                            <SabaStorage onBack={handleUpdatePanelBack} isExiting={updatePanelExiting} devMode={devMode} />
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

                    {activeTab === 'extensions' && (
                        <div className={`settings-tab-content ${slideDirection}`} key="extensions" onAnimationEnd={() => setSlideDirection('')}>
                            <h3>{t('gui:settings_modal.extensions_tab', 'Extensions')}</h3>
                            <p className="setting-description" style={{ margin: '0 0 12px', opacity: 0.75 }}>
                                {t('gui:settings_modal.extensions_toggle_hint', '설치된 익스텐션을 활성화하거나 비활성화합니다. 설치·삭제는 사바 스토리지에서 가능합니다.')}
                            </p>

                            {extensions.length === 0 ? (
                                <div className="setting-item" style={{ opacity: 0.6 }}>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.extensions_none', '설치된 익스텐션이 없습니다.')}
                                    </span>
                                </div>
                            ) : (
                                extensions.map(ext => (
                                    <div key={ext.id} className="setting-item extension-item">
                                        <label className="setting-label" htmlFor={`ext-toggle-${ext.id}`} style={{ cursor: 'pointer' }}>
                                            <span className="setting-title">
                                                {ext.name || ext.id}
                                                {ext.version && <span className="extension-version">v{ext.version}</span>}
                                            </span>
                                            {ext.description && (
                                                <span className="setting-description">{ext.description}</span>
                                            )}
                                        </label>
                                        <label className="extension-toggle">
                                            <input
                                                id={`ext-toggle-${ext.id}`}
                                                type="checkbox"
                                                checked={!!ext.enabled}
                                                disabled={togglingIds.has(ext.id)}
                                                onChange={e => handleExtensionToggle(ext.id, e.target.checked)}
                                            />
                                            <span className="extension-toggle-slider" />
                                        </label>
                                    </div>
                                ))
                            )}

                            <div className="setting-item setting-item-clickable" style={{ marginTop: 8 }} onClick={() => { handleTabChange('general'); setTimeout(() => setShowUpdatePanel(true), 50); }}>
                                <label className="setting-label" style={{ cursor: 'pointer' }}>
                                    <span className="setting-title"><Icon name="package" size="sm" /> {t('saba_storage.title', '사바 스토리지')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.extensions_store_hint', '설치·삭제·업데이트')}</span>
                                </label>
                                <Icon name="chevronRight" size="sm" color="var(--brand-primary)" />
                            </div>
                        </div>
                    )}

                    {activeTab === 'advanced' && (
                        <div className={`settings-tab-content ${slideDirection}`} key="advanced" onAnimationEnd={() => setSlideDirection('')}>
                            <h3>{t('gui:settings_modal.advanced_tab')}</h3>

                            {/* IPC 포트 설정 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title"><Icon name="hash" size="sm" /> {t('gui:settings_modal.ipc_port_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.ipc_port_description')}</span>
                                </label>
                                <input
                                    type="number"
                                    className={`setting-input-number ${ipcPortError ? 'setting-input-error' : ''}`}
                                    value={localIpcPort}
                                    onChange={(e) => handleIpcPortChange(e.target.value)}
                                    min={1024}
                                    max={65535}
                                    placeholder="57474"
                                />
                            </div>
                            {ipcPortError && (
                                <div className="setting-validation-error">
                                    <Icon name="alertCircle" size="sm" /> {ipcPortError}
                                </div>
                            )}
                            {ipcPortChanged && !ipcPortError && (
                                <div className="setting-restart-notice">
                                    <Icon name="info" size="sm" /> {t('gui:settings_modal.ipc_port_restart_notice')}
                                </div>
                            )}

                            {/* 콘솔 버퍼 크기 설정 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title"><Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_buffer_label')}</span>
                                    <span className="setting-description">{t('gui:settings_modal.console_buffer_description')}</span>
                                </label>
                                <CustomDropdown
                                    className="setting-select"
                                    value={localConsoleBuffer}
                                    onChange={(val) => handleConsoleBufferChange(Number(val))}
                                    options={[
                                        { value: 500, label: '500' },
                                        { value: 1000, label: '1,000' },
                                        { value: 2000, label: '2,000' },
                                        { value: 5000, label: '5,000' },
                                        { value: 10000, label: '10,000' },
                                    ]}
                                />
                            </div>

                            {/* GUI 컴포넌트 테스트 섹션 (개발자 모드 전용) */}
                            {devMode && (
                            <div className={`gui-test-section ${guiTestOpen ? 'open' : ''}`}>
                                <h4 className="gui-test-title" onClick={() => setGuiTestOpen(!guiTestOpen)}>
                                    <Icon name="tool" size="sm" /> GUI Test
                                    <Icon name={guiTestOpen ? 'chevronUp' : 'chevronDown'} size="sm" className="gui-test-chevron" />
                                </h4>
                                <div className="gui-test-grid">
                                    <button className="gui-test-btn gui-test-success" onClick={() => {
                                        onTestModal && onTestModal({ type: 'success', title: 'Success!', message: '작업이 성공적으로 완료되었습니다.' });
                                    }}>
                                        <Icon name="check" size="sm" /> Success
                                    </button>
                                    <button className="gui-test-btn gui-test-failure" onClick={() => {
                                        onTestModal && onTestModal({ type: 'failure', title: 'Error!', message: '예기치 못한 오류가 발생했습니다.' });
                                    }}>
                                        <Icon name="x" size="sm" /> Failure
                                    </button>
                                    <button className="gui-test-btn gui-test-notification" onClick={() => {
                                        onTestModal && onTestModal({ type: 'notification', title: 'Notice', message: '새로운 업데이트가 있습니다.' });
                                    }}>
                                        <Icon name="info" size="sm" /> Notification
                                    </button>
                                    <button className="gui-test-btn gui-test-question" onClick={() => {
                                        onTestModal && onTestModal({
                                            type: 'question', title: 'Confirm?',
                                            message: '이 작업을 진행하시겠습니까?',
                                            onConfirm: () => onTestModal(null),
                                            onCancel: () => onTestModal(null)
                                        });
                                    }}>
                                        <Icon name="alertCircle" size="sm" /> Question
                                    </button>
                                    <button className="gui-test-btn gui-test-toast-info" onClick={() => {
                                        window.showToast && window.showToast('ℹ️ 정보 토스트 메시지입니다.', 'info', 3000);
                                    }}>
                                        <Icon name="info" size="sm" /> Toast (Info)
                                    </button>
                                    <button className="gui-test-btn gui-test-toast-success" onClick={() => {
                                        window.showToast && window.showToast('✅ 성공 토스트 메시지입니다.', 'success', 3000);
                                    }}>
                                        <Icon name="check" size="sm" /> Toast (Success)
                                    </button>
                                    <button className="gui-test-btn gui-test-toast-warning" onClick={() => {
                                        window.showToast && window.showToast('⚠️ 경고 토스트 메시지입니다.', 'warning', 3000);
                                    }}>
                                        <Icon name="alertCircle" size="sm" /> Toast (Warning)
                                    </button>
                                    <button className="gui-test-btn gui-test-toast-error" onClick={() => {
                                        window.showToast && window.showToast('❌ 에러 토스트 메시지입니다.', 'error', 4000);
                                    }}>
                                        <Icon name="x" size="sm" /> Toast (Error)
                                    </button>
                                    <button className="gui-test-btn gui-test-progress" onClick={() => {
                                        if (!onTestProgressBar) return;
                                        onTestProgressBar({ message: '다운로드 중...', percent: 0 });
                                        let p = 0;
                                        const iv = setInterval(() => {
                                            p += Math.random() * 15 + 5;
                                            if (p >= 100) {
                                                p = 100;
                                                onTestProgressBar({ message: '완료!', percent: 100 });
                                                clearInterval(iv);
                                                setTimeout(() => onTestProgressBar(null), 1500);
                                            } else {
                                                onTestProgressBar({ message: `다운로드 중... ${Math.round(p)}%`, percent: p });
                                            }
                                        }, 400);
                                    }}>
                                        <Icon name="loader" size="sm" /> Progress Bar
                                    </button>
                                    <button className="gui-test-btn gui-test-progress-ind" onClick={() => {
                                        if (!onTestProgressBar) return;
                                        onTestProgressBar({ message: '처리 중...', indeterminate: true });
                                        setTimeout(() => onTestProgressBar(null), 4000);
                                    }}>
                                        <Icon name="loader" size="sm" /> Indeterminate
                                    </button>
                                    <button className="gui-test-btn gui-test-waiting" onClick={() => {
                                        onTestWaitingImage && onTestWaitingImage();
                                    }}>
                                        <Icon name="clock" size="sm" /> Waiting Image
                                    </button>
                                    <button className="gui-test-btn gui-test-loading" onClick={() => {
                                        onTestLoadingScreen && onTestLoadingScreen();
                                    }}>
                                        <Icon name="monitor" size="sm" /> Loading Screen
                                    </button>
                                    <button className="gui-test-btn gui-test-notice-info" onClick={() => {
                                        window.showToast && window.showToast('ℹ️ 정보 알림 테스트', 'info', 3000, { isNotice: true, source: 'GUI Test' });
                                    }}>
                                        <Icon name="bell" size="sm" /> Notice (Info)
                                    </button>
                                    <button className="gui-test-btn gui-test-notice-success" onClick={() => {
                                        window.showToast && window.showToast('✅ 성공 알림 테스트', 'success', 3000, { isNotice: true, source: 'GUI Test' });
                                    }}>
                                        <Icon name="bell" size="sm" /> Notice (Success)
                                    </button>
                                    <button className="gui-test-btn gui-test-notice-error" onClick={() => {
                                        window.showToast && window.showToast('❌ 에러 알림 테스트', 'error', 4000, { isNotice: true, source: 'GUI Test' });
                                    }}>
                                        <Icon name="bell" size="sm" /> Notice (Error)
                                    </button>
                                </div>
                            </div>
                            )}
                        </div>
                    )}
                </div>

                {!showUpdatePanel && (
                    <div className="settings-modal-footer">
                        <button className="settings-btn-cancel" onClick={requestClose}>
                            {t('gui:modals.cancel')}
                        </button>
                    </div>
                )}
            </div>
        </div>
    );
}

export default SettingsModal;
