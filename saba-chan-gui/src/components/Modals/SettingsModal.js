import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useExtensions } from '../../contexts/ExtensionContext';
import { useDevMode } from '../../hooks/useDevMode';
import { useModalClose } from '../../hooks/useModalClose';
import { useSettingsStore, DEFAULT_IPC_PORT } from '../../stores/useSettingsStore';
import { getTheme, setTheme as saveTheme } from '../../utils/themeManager';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaToggle } from '../ui/SabaUI';
import SabaStorage from './SabaStorage';

function SettingsModal({
    isOpen,
    onClose,
    refreshInterval,
    onRefreshIntervalChange,
    ipcPort,
    onIpcPortChange,
    consoleBufferSize,
    onConsoleBufferSizeChange,
    onTestModal,
    onTestProgressBar,
    onTestWaitingImage,
    onTestLoadingScreen,
    initialView,
    discordCloudRelayUrl,
    onDiscordCloudRelayUrlChange,
}) {
    const { t, i18n } = useTranslation(['gui', 'common']);
    const [activeTab, setActiveTab] = useState('general');
    const [showUpdatePanel, setShowUpdatePanel] = useState(false);
    const [updatePanelExiting, setUpdatePanelExiting] = useState(false);
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);
    const [selectedTheme, setSelectedTheme] = useState(getTheme());
    const [slideDirection, setSlideDirection] = useState('');
    const [guiTestOpen, setGuiTestOpen] = useState(false);
    const [localIpcPort, setLocalIpcPort] = useState(ipcPort || DEFAULT_IPC_PORT);
    const [ipcPortChanged, setIpcPortChanged] = useState(false);
    const [ipcPortError, setIpcPortError] = useState('');
    const [localConsoleBuffer, setLocalConsoleBuffer] = useState(consoleBufferSize || 2000);
    const [showAboutPage, setShowAboutPage] = useState(false);
    const [aboutExiting, setAboutExiting] = useState(false);
    const [appVersion, setAppVersion] = useState('0.1.0');
    const [componentInfo, setComponentInfo] = useState(null);
    const aboutBtnRef = useRef(null);
    const [aboutRevealOrigin, setAboutRevealOrigin] = useState({ x: '100%', y: '0%' });
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

    // biome-ignore lint/correctness/useExhaustiveDependencies: activeTab/showUpdatePanel/isOpen trigger DOM changes that syncIndicator reads via querySelector
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
    const autoGeneratePasswords = useSettingsStore((s) => s.autoGeneratePasswords);
    const portConflictCheck = useSettingsStore((s) => s.portConflictCheck);

    const handleExtensionToggle = useCallback(
        async (extId, enable) => {
            setTogglingIds((prev) => new Set(prev).add(extId));
            await toggleExtension(extId, enable);
            setTogglingIds((prev) => {
                const s = new Set(prev);
                s.delete(extId);
                return s;
            });
        },
        [toggleExtension],
    );

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
            setShowAboutPage(false);
        }
    }, [isOpen]);

    // 앱 버전 & 컴포넌트 정보 가져오기
    useEffect(() => {
        if (isOpen) {
            if (window.api?.getAppVersion) {
                window.api.getAppVersion().then((v) => v && setAppVersion(v)).catch(() => {});
            }
            if (window.api?.getComponentInfo) {
                window.api.getComponentInfo().then((info) => info && setComponentInfo(info)).catch(() => {});
            }
        }
    }, [isOpen]);

    // 탭 전환 핸들러
    const handleTabChange = (newTab) => {
        const oldIndex = tabOrder.indexOf(activeTab);
        const newIndex = tabOrder.indexOf(newTab);
        if (newTab === activeTab) return;
        setShowUpdatePanel(false); // 탭 전환 시 업데이트 패널 해제
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

    // 정보 페이지 뒤로가기
    const handleAboutBack = useCallback(() => {
        setAboutExiting(true);
        setTimeout(() => {
            setAboutExiting(false);
            setShowAboutPage(false);
        }, 400);
    }, []);

    // 정보 페이지 열기 (원형 확장 효과)
    const handleAboutOpen = useCallback(() => {
        const btn = aboutBtnRef.current;
        if (btn) {
            const container = btn.closest('.settings-modal-container');
            if (container) {
                const containerRect = container.getBoundingClientRect();
                const btnRect = btn.getBoundingClientRect();
                const x = btnRect.left - containerRect.left + btnRect.width / 2;
                const y = btnRect.top - containerRect.top + btnRect.height / 2;
                setAboutRevealOrigin({ x: `${x}px`, y: `${y}px` });
            }
        }
        setShowAboutPage(true);
    }, []);

    // 사바쨩 삭제 핸들러
    const handleUninstall = useCallback(() => {
        if (!window.api?.launchUninstaller) return;
        // question 모달을 통해 확인
        if (onTestModal) {
            onTestModal({
                type: 'question',
                title: t('gui:settings_modal.uninstall_confirm_title'),
                message: t('gui:settings_modal.uninstall_confirm_message'),
                confirmText: t('gui:settings_modal.uninstall_confirm_yes'),
                cancelText: t('gui:settings_modal.uninstall_confirm_no'),
                onConfirm: async () => {
                    onTestModal(null);
                    try {
                        await window.api.launchUninstaller();
                    } catch (err) {
                        console.error('Failed to launch uninstaller:', err);
                    }
                },
                onCancel: () => onTestModal(null),
            });
        }
    }, [t, onTestModal]);

    // refreshInterval prop이 변경되면 로컬 상태 업데이트
    useEffect(() => {
        setLocalRefreshInterval(refreshInterval);
    }, [refreshInterval]);

    // ipcPort prop 동기화
    useEffect(() => {
        setLocalIpcPort(ipcPort || DEFAULT_IPC_PORT);
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
    const handleLanguageChange = async (lng) => {
        setSelectedLanguage(lng);
        // 1. Electron settings.json에 먼저 확실히 저장 (봇·데몬 공유 원천)
        if (window.electron) {
            try {
                await window.electron.setLanguage(lng);
            } catch (err) {
                console.error('Failed to save language to Electron settings:', err);
            }
        }
        // 2. localStorage 동기화 (캐시)
        localStorage.setItem('i18nextLng', lng);
        // 3. i18n 언어 변경 (UI 즉시 반영)
        i18n.changeLanguage(lng);
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
        setIpcPortChanged(port !== (ipcPort || DEFAULT_IPC_PORT));
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
        <div className={clsx('settings-modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                {!showUpdatePanel && (
                    <>
                        <div className="settings-modal-header">
                            <h2 style={{ fontSize: '1.3rem' }}>{t('gui:settings_modal.title')}</h2>
                            <button
                                className="settings-about-btn"
                                ref={aboutBtnRef}
                                onClick={handleAboutOpen}
                                title={t('gui:settings_modal.about_title')}
                            >
                                <Icon name="info" size="sm" />
                            </button>
                        </div>

                        <div className="settings-modal-tabs" ref={tabsRef}>
                            <div className="settings-tab-indicator" ref={indicatorRef} />
                            <button
                                className={clsx('settings-tab', { active: activeTab === 'general' })}
                                onClick={() => handleTabChange('general')}
                            >
                                {t('gui:settings_modal.general')}
                            </button>
                            <button
                                className={clsx('settings-tab', { active: activeTab === 'appearance' })}
                                onClick={() => handleTabChange('appearance')}
                            >
                                {t('gui:settings_modal.appearance')}
                            </button>
                            <button
                                className={clsx('settings-tab', { active: activeTab === 'extensions' })}
                                onClick={() => handleTabChange('extensions')}
                            >
                                {t('gui:settings_modal.extensions_tab', 'Extensions')}
                            </button>
                            <button
                                className={clsx('settings-tab', { active: activeTab === 'advanced' })}
                                onClick={() => handleTabChange('advanced')}
                            >
                                {t('gui:settings_modal.advanced_tab')}
                            </button>
                        </div>
                    </>
                )}

                <div className={clsx('settings-modal-content', { 'update-panel-mode': showUpdatePanel })}>

                    {activeTab === 'general' && !showUpdatePanel && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="general"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t('gui:settings_modal.general')}</h3>

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
                            <div
                                className="setting-item setting-item-clickable"
                                onClick={() => setShowUpdatePanel(true)}
                            >
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="package" size="sm" /> {t('saba_storage.title', '사바 스토리지')}
                                    </span>
                                    <span className="setting-description">
                                        {t('saba_storage.setting_description', '업데이트, 모듈, 익스텐션 관리')}
                                    </span>
                                </label>
                                <Icon name="chevronRight" size="sm" color="var(--brand-primary)" />
                            </div>
                        </div>
                    )}

                    {activeTab === 'general' && showUpdatePanel && (
                        <div className="settings-tab-content" key="update-panel">
                            <SabaStorage
                                onBack={handleUpdatePanelBack}
                                isExiting={updatePanelExiting}
                                devMode={devMode}
                            />
                        </div>
                    )}

                    {activeTab === 'appearance' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="appearance"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t('gui:settings_modal.appearance')}</h3>

                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="palette" size="sm" /> {t('gui:settings_modal.theme_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.theme_description')}
                                    </span>
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
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="extensions"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t('gui:settings_modal.extensions_tab', 'Extensions')}</h3>
                            <p className="setting-description" style={{ margin: '0 0 12px', opacity: 0.75 }}>
                                {t(
                                    'gui:settings_modal.extensions_toggle_hint',
                                    '설치된 익스텐션을 활성화하거나 비활성화합니다. 설치·삭제는 사바 스토리지에서 가능합니다.',
                                )}
                            </p>

                            {extensions.length === 0 ? (
                                <div className="setting-item" style={{ opacity: 0.6 }}>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.extensions_none', '설치된 익스텐션이 없습니다.')}
                                    </span>
                                </div>
                            ) : (
                                extensions.map((ext) => (
                                    <div key={ext.id} className="setting-item extension-item">
                                        <label
                                            className="setting-label"
                                            htmlFor={`ext-toggle-${ext.id}`}
                                            style={{ cursor: 'pointer' }}
                                        >
                                            <span className="setting-title">
                                                {ext.name || ext.id}
                                                {ext.version && (
                                                    <span className="extension-version">v{ext.version}</span>
                                                )}
                                            </span>
                                            {ext.description && (
                                                <span className="setting-description">{ext.description}</span>
                                            )}
                                        </label>
                                        <SabaToggle
                                            checked={!!ext.enabled}
                                            disabled={togglingIds.has(ext.id)}
                                            onChange={(checked) => handleExtensionToggle(ext.id, checked)}
                                        />
                                    </div>
                                ))
                            )}

                            <div
                                className="setting-item setting-item-clickable"
                                style={{ marginTop: 8 }}
                                onClick={() => {
                                    handleTabChange('general');
                                    setTimeout(() => setShowUpdatePanel(true), 50);
                                }}
                            >
                                <label className="setting-label" style={{ cursor: 'pointer' }}>
                                    <span className="setting-title">
                                        <Icon name="package" size="sm" /> {t('saba_storage.title', '사바 스토리지')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.extensions_store_hint', '설치·삭제·업데이트')}
                                    </span>
                                </label>
                                <Icon name="chevronRight" size="sm" color="var(--brand-primary)" />
                            </div>
                        </div>
                    )}

                    {activeTab === 'advanced' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="advanced"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t('gui:settings_modal.advanced_tab')}</h3>

                            {/* 비밀번호 자동 생성 설정 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="key" size="sm" />{' '}
                                        {t('gui:settings_modal.auto_generate_passwords_label', 'Auto-generate Passwords')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.auto_generate_passwords_description', 'Automatically fill empty RCON/REST password fields with random passwords when opening server settings.')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={autoGeneratePasswords}
                                    onChange={(checked) => useSettingsStore.getState().update({ autoGeneratePasswords: checked })}
                                />
                            </div>

                            {/* 포트 충돌 검사 설정 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="alertCircle" size="sm" />{' '}
                                        {t('gui:settings_modal.port_conflict_check_label', 'Port Conflict Detection')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.port_conflict_check_description', 'Show warnings when multiple server instances use the same port.')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={portConflictCheck}
                                    onChange={(checked) => useSettingsStore.getState().update({ portConflictCheck: checked })}
                                />
                            </div>

                            {/* IPC 포트 설정 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="hash" size="sm" /> {t('gui:settings_modal.ipc_port_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.ipc_port_description')}
                                    </span>
                                </label>
                                <input
                                    type="number"
                                    className={clsx('setting-input-number', { 'setting-input-error': ipcPortError })}
                                    value={localIpcPort}
                                    onChange={(e) => handleIpcPortChange(e.target.value)}
                                    min={1024}
                                    max={65535}
                                    placeholder={String(DEFAULT_IPC_PORT)}
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

                            {/* 커스텀 릴레이 서버 URL */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="globe" size="sm" />{' '}
                                        {t('gui:settings_modal.relay_url_label', '릴레이 서버 URL')}
                                    </span>
                                    <span className="setting-description">
                                        {t(
                                            'gui:settings_modal.relay_url_description',
                                            '클라우드 모드에서 사용할 릴레이 서버의 URL입니다. 비어있으면 기본 서버를 사용합니다.',
                                        )}
                                    </span>
                                </label>
                                <input
                                    type="text"
                                    className="setting-input-text"
                                    value={discordCloudRelayUrl || ''}
                                    onChange={(e) =>
                                        onDiscordCloudRelayUrlChange && onDiscordCloudRelayUrlChange(e.target.value)
                                    }
                                    placeholder="https://relay.saba-chan.app"
                                />
                            </div>

                            {/* 콘솔 버퍼 크기 설정 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" />{' '}
                                        {t('gui:settings_modal.console_buffer_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_buffer_description')}
                                    </span>
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
                                <div className={clsx('gui-test-section', { open: guiTestOpen })}>
                                    <h4 className="gui-test-title" onClick={() => setGuiTestOpen(!guiTestOpen)}>
                                        <Icon name="tool" size="sm" /> GUI Test
                                        <Icon
                                            name={guiTestOpen ? 'chevronUp' : 'chevronDown'}
                                            size="sm"
                                            className="gui-test-chevron"
                                        />
                                    </h4>
                                    <div className="gui-test-grid">
                                        <button
                                            className="gui-test-btn gui-test-success"
                                            onClick={() => {
                                                onTestModal &&
                                                    onTestModal({
                                                        type: 'success',
                                                        title: 'Success!',
                                                        message: '작업이 성공적으로 완료되었습니다.',
                                                    });
                                            }}
                                        >
                                            <Icon name="check" size="sm" /> Success
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-failure"
                                            onClick={() => {
                                                onTestModal &&
                                                    onTestModal({
                                                        type: 'failure',
                                                        title: 'Error!',
                                                        message: '예기치 못한 오류가 발생했습니다.',
                                                    });
                                            }}
                                        >
                                            <Icon name="x" size="sm" /> Failure
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-notification"
                                            onClick={() => {
                                                onTestModal &&
                                                    onTestModal({
                                                        type: 'notification',
                                                        title: 'Notice',
                                                        message: '새로운 업데이트가 있습니다.',
                                                    });
                                            }}
                                        >
                                            <Icon name="info" size="sm" /> Notification
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-question"
                                            onClick={() => {
                                                onTestModal &&
                                                    onTestModal({
                                                        type: 'question',
                                                        title: 'Confirm?',
                                                        message: '이 작업을 진행하시겠습니까?',
                                                        onConfirm: () => onTestModal(null),
                                                        onCancel: () => onTestModal(null),
                                                    });
                                            }}
                                        >
                                            <Icon name="alertCircle" size="sm" /> Question
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-info"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('ℹ️ 정보 토스트 메시지입니다.', 'info', 3000);
                                            }}
                                        >
                                            <Icon name="info" size="sm" /> Toast (Info)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-success"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('✅ 성공 토스트 메시지입니다.', 'success', 3000);
                                            }}
                                        >
                                            <Icon name="check" size="sm" /> Toast (Success)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-warning"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('⚠️ 경고 토스트 메시지입니다.', 'warning', 3000);
                                            }}
                                        >
                                            <Icon name="alertCircle" size="sm" /> Toast (Warning)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-error"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('❌ 에러 토스트 메시지입니다.', 'error', 4000);
                                            }}
                                        >
                                            <Icon name="x" size="sm" /> Toast (Error)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-progress"
                                            onClick={() => {
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
                                                        onTestProgressBar({
                                                            message: `다운로드 중... ${Math.round(p)}%`,
                                                            percent: p,
                                                        });
                                                    }
                                                }, 400);
                                            }}
                                        >
                                            <Icon name="loader" size="sm" /> Progress Bar
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-progress-ind"
                                            onClick={() => {
                                                if (!onTestProgressBar) return;
                                                onTestProgressBar({ message: '처리 중...', indeterminate: true });
                                                setTimeout(() => onTestProgressBar(null), 4000);
                                            }}
                                        >
                                            <Icon name="loader" size="sm" /> Indeterminate
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-waiting"
                                            onClick={() => {
                                                onTestWaitingImage && onTestWaitingImage();
                                            }}
                                        >
                                            <Icon name="clock" size="sm" /> Waiting Image
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-loading"
                                            onClick={() => {
                                                onTestLoadingScreen && onTestLoadingScreen();
                                            }}
                                        >
                                            <Icon name="monitor" size="sm" /> Loading Screen
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-notice-info"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('ℹ️ 정보 알림 테스트', 'info', 3000, {
                                                        isNotice: true,
                                                        source: 'GUI Test',
                                                    });
                                            }}
                                        >
                                            <Icon name="bell" size="sm" /> Notice (Info)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-notice-success"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('✅ 성공 알림 테스트', 'success', 3000, {
                                                        isNotice: true,
                                                        source: 'GUI Test',
                                                    });
                                            }}
                                        >
                                            <Icon name="bell" size="sm" /> Notice (Success)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-notice-error"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('❌ 에러 알림 테스트', 'error', 4000, {
                                                        isNotice: true,
                                                        source: 'GUI Test',
                                                    });
                                            }}
                                        >
                                            <Icon name="bell" size="sm" /> Notice (Error)
                                        </button>
                                    </div>
                                </div>
                            )}

                            {/* 사바쨩 삭제 */}
                            <div className="setting-item setting-item-danger" onClick={handleUninstall}>
                                <label className="setting-label" style={{ cursor: 'pointer' }}>
                                    <span className="setting-title setting-title-danger">
                                        <Icon name="trash" size="sm" /> {t('gui:settings_modal.uninstall_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.uninstall_description')}
                                    </span>
                                </label>
                                <Icon name="chevronRight" size="sm" color="var(--color-danger, #e74c3c)" />
                            </div>
                        </div>
                    )}
                </div>

                {/* ── 정보 페이지 (원형 확장 오버레이) ── */}
                <div
                    className={clsx('about-page-reveal', {
                        'about-active': showAboutPage && !aboutExiting,
                        'about-closing': aboutExiting,
                    })}
                    style={{ '--reveal-x': aboutRevealOrigin.x, '--reveal-y': aboutRevealOrigin.y }}
                >
                    <div className="about-header">
                        <h2 className="about-page-title">{t('gui:settings_modal.about_title')}</h2>
                        <button className="about-close-btn" onClick={handleAboutBack}>
                            <Icon name="x" size="sm" />
                        </button>
                    </div>

                    <div className="about-scroll-area">
                        {/* 개발자 프로필 카드 */}
                        <div className="about-card about-dev-card">
                            <div className="about-dev-avatar">
                                <img
                                    src="https://github.com/WareAoba.png"
                                    alt="WareAoba"
                                    className="about-dev-img"
                                    onError={(e) => { e.target.style.display = 'none'; }}
                                />
                            </div>
                            <div className="about-dev-info">
                                <span className="about-dev-nickname">와레아오바</span>
                                <span className="about-dev-id">@WareAoba</span>
                                <span className="about-dev-bio">인공지능 조련사</span>
                            </div>
                            <span className="about-made-by">{t('gui:settings_modal.made_by')}</span>
                            <button
                                className="about-dev-github-btn"
                                onClick={() => {
                                    if (window.electron?.shell?.openExternal) {
                                        window.electron.shell.openExternal('https://github.com/WareAoba/saba-chan');
                                    } else {
                                        window.open('https://github.com/WareAoba/saba-chan', '_blank');
                                    }
                                }}
                            >
                                <Icon name="github" size="sm" />
                                <span>GitHub 리포지토리</span>
                            </button>
                        </div>

                        {/* 사바쨩 정보 카드 */}
                        <div className="about-card about-app-card">
                            <div className="about-app-logo-container">
                                <img src="/title.png" alt="Saba-chan" className="about-app-logo" />
                                <span className="about-version-badge">v{appVersion}</span>
                            </div>
                            <div className="about-app-info">
                                {/* 컴포넌트 버전 */}
                                <div className="about-component-list">
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="package" size="xs" /> Core</span>
                                        <span className="about-component-ver">v{componentInfo?.components?.['saba-core'] || appVersion}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="monitor" size="xs" /> GUI</span>
                                        <span className="about-component-ver">v{componentInfo?.components?.gui || appVersion}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="terminal" size="xs" /> CLI</span>
                                        <span className="about-component-ver">v{componentInfo?.components?.cli || appVersion}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="discord" size="xs" /> Discord Bot</span>
                                        <span className="about-component-ver">v{componentInfo?.components?.discord_bot || appVersion}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="refresh" size="xs" /> Updater</span>
                                        <span className="about-component-ver">v{componentInfo?.components?.updater || appVersion}</span>
                                    </div>
                                </div>

                                {/* 마지막 업데이트 & 라이선스 */}
                                <div className="about-app-footer">
                                    {componentInfo?.lastUpdated && (
                                        <span className="about-app-meta">
                                            <Icon name="clock" size="xs" /> {new Date(componentInfo.lastUpdated).toLocaleDateString(i18n.language, { year: 'numeric', month: 'long', day: 'numeric' })}
                                        </span>
                                    )}
                                    <span className="about-app-meta">
                                        <Icon name="file" size="xs" /> MIT License
                                    </span>
                                </div>
                            </div>
                        </div>
                    </div>
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
