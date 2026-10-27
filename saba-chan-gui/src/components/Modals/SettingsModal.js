import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useDevMode } from '../../hooks/useDevMode';
import { useModalClose } from '../../hooks/useModalClose';
import { DiagnosticsPanel, diagnosticsAvailable } from '../DiagnosticsPanel';
import { useSettingsStore, DEFAULT_IPC_PORT } from '../../stores/useSettingsStore';
import {
    getTheme,
    setTheme as saveTheme,
    getCustomTheme,
    setCustomTheme,
    resetCustomTheme,
    ACCENT_PRESETS,
    THEME_DEFAULTS,
    applyCustomTheme,
} from '../../utils/themeManager';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { OklchPicker } from '../ui/OklchPicker';
import { Icon } from '../Icon';
import { safeShowToast } from '../../utils/helpers';
import { LANGUAGE_OPTIONS } from '../../utils/languages';
import { SabaToggle } from '../ui/SabaUI';
import { DataTransferSection, dataTransferAvailable } from './DataTransferSection';
import { FolderPathsSection, folderPathsAvailable } from './FolderPathsSection';
import { RemoteDaemonSection, remoteDaemonAvailable } from './RemoteDaemonSection';
import { useDaemonTarget } from '../shell/DaemonSwitcher';
import { AutostartSetting, autostartAvailable } from './AutostartSetting';
import { ExtensionGuiSection, extensionGuiAvailable } from './ExtensionGuiSection';
import { UnisolatedSection, unisolatedAvailable } from './UnisolatedSection';
import SabaStorage from './SabaStorage';

// 효과음 종류별 스위치 (DEFAULT_SOUNDS 키 → 이름)
const SOUND_KINDS = [
    ['serverStart', 'gui:settings_modal.sound_server_start'],
    ['serverStop', 'gui:settings_modal.sound_server_stop'],
    ['crash', 'gui:settings_modal.sound_crash'],
    ['install', 'gui:settings_modal.sound_install'],
    ['update', 'gui:settings_modal.sound_update'],
];

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
    variant = 'modal',
}) {
    const { t, i18n } = useTranslation(['gui', 'common']);
    const daemonTarget = useDaemonTarget();
    const [activeTab, setActiveTab] = useState('general');
    const [showUpdatePanel, setShowUpdatePanel] = useState(false);
    const [updatePanelExiting, setUpdatePanelExiting] = useState(false);
    // 사바 스토리지에서 돌아갈 탭 (들어온 탭으로 돌아간다)
    const storageReturnTab = useRef('general');
    // 사바 스토리지를 열 때 처음 보일 탭 (설정을 '익스텐션'으로 열면 익스텐션 탭)
    const [storageTab, setStorageTab] = useState('components');
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);
    const [selectedTheme, setSelectedTheme] = useState(getTheme());
    const [customTheme, setCustomThemeState] = useState(getCustomTheme());
    const [slideDirection, setSlideDirection] = useState('');
    const [guiTestOpen, setGuiTestOpen] = useState(false);
    const [localIpcPort, setLocalIpcPort] = useState(ipcPort || DEFAULT_IPC_PORT);
    const [ipcPortChanged, setIpcPortChanged] = useState(false);
    const [ipcPortError, setIpcPortError] = useState('');
    const [localConsoleBuffer, setLocalConsoleBuffer] = useState(consoleBufferSize || 2000);
    const [showAboutPage, setShowAboutPage] = useState(false);
    const [aboutExiting, setAboutExiting] = useState(false);
    // 받기 전에는 모른다: 지어낸 버전을 보이지 않는다
    const [appVersion, setAppVersion] = useState(null);
    const [componentInfo, setComponentInfo] = useState(null);
    const aboutBtnRef = useRef(null);
    // 이스터에그: 정보 페이지 로고를 다섯 번 누르면 이름 해명
    const logoClicks = useRef(0);
    const [aboutRevealOrigin, setAboutRevealOrigin] = useState({ x: '100%', y: '0%' });
    const tabOrder = ['general', 'appearance', 'advanced'];
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
    const autoGeneratePasswords = useSettingsStore((s) => s.autoGeneratePasswords);
    const portConflictCheck = useSettingsStore((s) => s.portConflictCheck);
    const autoPortCheck = useSettingsStore((s) => s.autoPortCheck);
    const osNotifications = useSettingsStore((s) => s.osNotifications);
    const sounds = useSettingsStore((s) => s.sounds);
    const closeAction = useSettingsStore((s) => s.closeAction);
    const toastDuration = useSettingsStore((s) => s.toastDuration);
    const newServerBackupKeep = useSettingsStore((s) => s.newServerBackupKeep);
    const newServerAutoRestart = useSettingsStore((s) => s.newServerAutoRestart);
    const auditKeepDays = useSettingsStore((s) => s.auditKeepDays);
    const consoleTimestamps = useSettingsStore((s) => s.consoleTimestamps);
    const consoleTimestampDate = useSettingsStore((s) => s.consoleTimestampDate);
    const setOption = (patch) => useSettingsStore.getState().update(patch);
    const setSound = (key, on) => useSettingsStore.getState().update({ sounds: { ...sounds, [key]: on } });

    // 외부에서 initialView 지정으로 열렸을 때 해당 탭 자동 진입
    useEffect(() => {
        if (isOpen && (initialView === 'update' || initialView === 'extensions')) {
            setActiveTab('general');
            setStorageTab(initialView === 'extensions' ? 'extensions' : 'components');
            setShowUpdatePanel(true);
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
                window.api.getAppVersion().then((v) => typeof v === 'string' && setAppVersion(v)).catch(() => {});
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

    // 사바 스토리지 열기: 탭을 바꾸지 않고 바로 들어간다
    const openStorage = useCallback(() => {
        storageReturnTab.current = activeTab;
        setStorageTab('components');
        setShowUpdatePanel(true);
    }, [activeTab]);

    // 업데이트 패널 뒤로가기 (나가기 애니메이션 후 들어온 탭으로)
    const handleUpdatePanelBack = useCallback(() => {
        setUpdatePanelExiting(true);
        setTimeout(() => {
            setUpdatePanelExiting(false);
            setShowUpdatePanel(false);
            setActiveTab(storageReturnTab.current);
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
        // 1. config.json의 gui에 먼저 확실히 저장 (데몬과 공유하는 원천)
        if (window.electron) {
            // 저장하지 못하면 알린다: 화면은 바뀌어도 다음 실행에 옛 언어로 돌아간다 (G-U6)
            try {
                const res = await window.electron.setLanguage(lng);
                if (res?.success === false) safeShowToast(t('gui:settings.save_error'), 'error');
            } catch (err) {
                console.error('Failed to save language to Electron settings:', err);
                safeShowToast(t('gui:settings.save_error'), 'error');
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
        useSettingsStore.getState().update({ theme });
        // Re-apply custom theme after base theme change (accent colors adapt to light/dark)
        setTimeout(() => applyCustomTheme(customTheme), 50);
    };

    // 커스텀 테마 속성 변경 핸들러
    const handleCustomThemeChange = (partial) => {
        const merged = setCustomTheme(partial);
        setCustomThemeState(merged);
        // Sync to settings store for persistence via gui.json
        useSettingsStore.getState().update(partial);
    };

    // 테마 초기화 핸들러
    const handleResetTheme = () => {
        const defaults = resetCustomTheme();
        setCustomThemeState(defaults);
        useSettingsStore.getState().update({
            accentColor: defaults.accentColor,
            accentSecondary: defaults.accentSecondary,
            useGradient: defaults.useGradient,
            fontScale: defaults.fontScale,
            enableTransitions: defaults.enableTransitions,
            consoleSyntaxHighlight: defaults.consoleSyntaxHighlight,
            consoleBgColor: defaults.consoleBgColor,
            consoleTextColor: defaults.consoleTextColor,
            consoleFontScale: defaults.consoleFontScale,
            sidebarCompact: defaults.sidebarCompact,
        });
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

    const isPage = variant === 'page';

    return (
        <div
            className={clsx(isPage ? 'settings-page' : 'settings-modal-overlay', { closing: isClosing && !isPage })}
            onClick={isPage ? undefined : requestClose}
        >
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
                                    options={LANGUAGE_OPTIONS}
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

                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="messageSquare" size="sm" />{' '}
                                        {t('gui:settings_modal.toast_duration_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.toast_duration_description')}
                                    </span>
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

                            {autostartAvailable() && <AutostartSetting />}

                            {/* 중대한 알림을 OS 알림으로도 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="bell" size="sm" />{' '}
                                        {t('gui:settings_modal.os_notifications_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.os_notifications_description')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={osNotifications}
                                    onChange={(checked) => useSettingsStore.getState().update({ osNotifications: checked })}
                                />
                            </div>

                            {/* 밖에서 켜고 끈 것·크래시·설치 결과·업데이트 효과음: 전체 스위치 + 종류별 */}
                            <div className="setting-item setting-item-group">
                                <div className="setting-item-row">
                                    <label className="setting-label">
                                        <span className="setting-title">
                                            <Icon name="volume" size="sm" /> {t('gui:settings_modal.sound_effects_label')}
                                        </span>
                                        <span className="setting-description">
                                            {t('gui:settings_modal.sound_effects_description')}
                                        </span>
                                    </label>
                                    <SabaToggle checked={sounds.enabled} onChange={(on) => setSound('enabled', on)} />
                                </div>
                                {sounds.enabled && (
                                    <div className="setting-sublist">
                                        {SOUND_KINDS.map(([key, label]) => (
                                            <div className="setting-subitem" key={key}>
                                                <span>{t(label)}</span>
                                                <SabaToggle
                                                    size="sm"
                                                    checked={sounds[key]}
                                                    onChange={(on) => setSound(key, on)}
                                                />
                                            </div>
                                        ))}
                                    </div>
                                )}
                            </div>

                            {/* 사바 스토리지 — 클릭하면 SabaStorage로 전환 */}
                            <div
                                className="setting-item setting-item-clickable"
                                onClick={openStorage}
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

                    {showUpdatePanel && (
                        <div className="settings-tab-content" key="update-panel">
                            <SabaStorage
                                key={storageTab}
                                onBack={handleUpdatePanelBack}
                                isExiting={updatePanelExiting}
                                devMode={devMode}
                                initialTab={storageTab}
                            />
                        </div>
                    )}

                    {activeTab === 'appearance' && !showUpdatePanel && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="appearance"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t('gui:settings_modal.appearance')}</h3>

                            {/* ── 1. 라이트/다크 테마 ── */}
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

                            {/* ── 2. 강조색 프리셋 ── */}
                            <div className="setting-item setting-item-vertical">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="palette" size="sm" /> {t('gui:settings_modal.accent_color_label', 'Accent Color')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.accent_color_description', 'Choose a brand color for buttons, highlights, and accents')}
                                    </span>
                                </label>
                                <div className="accent-preset-grid">
                                    {ACCENT_PRESETS.map((preset) => (
                                        <button
                                            key={preset.name}
                                            className={clsx('accent-preset-btn', {
                                                active: customTheme.accentColor === preset.primary,
                                            })}
                                            style={{
                                                background: `linear-gradient(135deg, ${preset.primary} 0%, ${preset.secondary} 100%)`,
                                            }}
                                            onClick={() =>
                                                handleCustomThemeChange({
                                                    accentColor: preset.primary,
                                                    accentSecondary: preset.secondary,
                                                })
                                            }
                                            title={t(`gui:settings_modal.accent_${preset.name}`, preset.name)}
                                        />
                                    ))}
                                    <OklchPicker
                                        variant="swatch"
                                        className="accent-preset-btn accent-preset-custom"
                                        label={t('gui:settings_modal.accent_custom', 'Custom')}
                                        value={customTheme.accentColor}
                                        onChange={(hex) =>
                                            handleCustomThemeChange({ accentColor: hex, accentSecondary: hex })
                                        }
                                    />
                                </div>
                            </div>

                            {/* ── 2-1. 그라데이션 토글 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="palette" size="sm" /> {t('gui:settings_modal.gradient_label', 'Gradient Effect')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.gradient_description', 'Apply gradient effect to buttons and app background')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={customTheme.useGradient}
                                    onChange={(checked) => handleCustomThemeChange({ useGradient: checked })}
                                />
                            </div>

                            {/* ── 3. 문자 크기 배율 ── */}
                            <div className="setting-item setting-item-vertical">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="settings" size="sm" /> {t('gui:settings_modal.font_scale_label', 'Font Size')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.font_scale_description', 'Adjust the overall text size (50%–200%)')}
                                    </span>
                                </label>
                                <div className="font-scale-row">
                                    <input
                                        type="range"
                                        className="font-scale-slider"
                                        min={50}
                                        max={200}
                                        step={5}
                                        value={customTheme.fontScale}
                                        onChange={(e) => handleCustomThemeChange({ fontScale: Number(e.target.value) })}
                                    />
                                    <span className="font-scale-value">{customTheme.fontScale}%</span>
                                </div>
                            </div>

                            {/* ── 4. 트랜지션 토글 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="settings" size="sm" /> {t('gui:settings_modal.transitions_label', 'Animations')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.transitions_description', 'Enable or disable UI transition animations')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={customTheme.enableTransitions}
                                    onChange={(checked) => handleCustomThemeChange({ enableTransitions: checked })}
                                />
                            </div>

                            {/* ── 콘솔 커스터마이즈 섹션 ── */}
                            <h3 style={{ marginTop: 16 }}>{t('gui:settings_modal.console_appearance_title', 'Console Appearance')}</h3>

                            {/* ── 5-1. 콘솔 구문강조 토글 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_syntax_label', 'Syntax Highlighting')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_syntax_description', 'Color-code log output in the server console')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={customTheme.consoleSyntaxHighlight}
                                    onChange={(checked) => handleCustomThemeChange({ consoleSyntaxHighlight: checked })}
                                />
                            </div>

                            {/* 줄 앞 시각 (데몬이 받은 시각) */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="clock" size="sm" /> {t('gui:settings_modal.console_timestamps_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_timestamps_description')}
                                    </span>
                                </label>
                                <CustomDropdown
                                    className="setting-select"
                                    value={consoleTimestamps}
                                    onChange={(val) => setOption({ consoleTimestamps: val })}
                                    options={['off', '24h', '12h'].map((value) => ({
                                        value,
                                        label: t(`gui:settings_modal.console_timestamps_${value}`),
                                    }))}
                                />
                            </div>
                            {consoleTimestamps !== 'off' && (
                                <div className="setting-item">
                                    <label className="setting-label">
                                        <span className="setting-title">
                                            <Icon name="clock" size="sm" />{' '}
                                            {t('gui:settings_modal.console_timestamp_date_label')}
                                        </span>
                                    </label>
                                    <SabaToggle
                                        checked={consoleTimestampDate}
                                        onChange={(checked) => setOption({ consoleTimestampDate: checked })}
                                    />
                                </div>
                            )}

                            {/* ── 5-2. 콘솔 뒷배경색 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_bg_label', 'Console Background')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_bg_description', 'Background color of the console panel')}
                                    </span>
                                </label>
                                <OklchPicker
                                    label={t('gui:settings_modal.console_bg_label', 'Console Background')}
                                    value={customTheme.consoleBgColor}
                                    onChange={(hex) => handleCustomThemeChange({ consoleBgColor: hex })}
                                />
                            </div>

                            {/* ── 5-3. 콘솔 기본 폰트 컬러 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_text_label', 'Console Text Color')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_text_description', 'Default text color in the console output')}
                                    </span>
                                </label>
                                <OklchPicker
                                    label={t('gui:settings_modal.console_text_label', 'Console Text Color')}
                                    value={customTheme.consoleTextColor}
                                    onChange={(hex) => handleCustomThemeChange({ consoleTextColor: hex })}
                                />
                            </div>

                            {/* ── 5-4. 콘솔 텍스트 크기 ── */}
                            <div className="setting-item setting-item-vertical">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_font_scale_label', 'Console Text Size')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_font_scale_description', 'Adjust the console text size (50%–200%)')}
                                    </span>
                                </label>
                                <div className="font-scale-row">
                                    <input
                                        type="range"
                                        className="font-scale-slider"
                                        min={50}
                                        max={200}
                                        step={5}
                                        value={customTheme.consoleFontScale}
                                        onChange={(e) => handleCustomThemeChange({ consoleFontScale: Number(e.target.value) })}
                                    />
                                    <span className="font-scale-value">{customTheme.consoleFontScale}%</span>
                                </div>
                            </div>

                            {/* ── 콘솔 미리보기 ── */}
                            <div
                                className="console-preview-box"
                                style={{
                                    background: customTheme.consoleBgColor,
                                    color: customTheme.consoleTextColor,
                                    fontSize: `calc(12.5px * ${customTheme.consoleFontScale / 100})`,
                                }}
                            >
                                {(() => {
                                    // 하이라이트가 켜져 있으면 실제 콘솔과 같은 토큰 색으로 (테마 --tok-*)
                                    const tok = (token, text) =>
                                        customTheme.consoleSyntaxHighlight ? <span className={`tok-${token}`}>{text}</span> : text;
                                    return (
                                        <>
                                            <div className="console-preview-line">
                                                {tok('timestamp', '[12:34:56]')} Server started on port {tok('number', '25565')}
                                            </div>
                                            <div className="console-preview-line">
                                                {tok('timestamp', '[12:34:57]')} {tok('player', 'Steve')} joined the game
                                            </div>
                                            <div className="console-preview-line">
                                                {tok('timestamp', '[12:34:58]')} {tok('warn', 'WARN')}: Low memory detected
                                            </div>
                                            <div className="console-preview-line">
                                                {tok('timestamp', '[12:34:59]')} {tok('error', 'ERROR')}: Connection timeout
                                            </div>
                                        </>
                                    );
                                })()}
                            </div>

                            {/* ── 초기화 버튼 ── */}
                            <div className="setting-item setting-item-clickable" onClick={handleResetTheme} style={{ marginTop: 8 }}>
                                <label className="setting-label" style={{ cursor: 'pointer' }}>
                                    <span className="setting-title">
                                        <Icon name="refresh" size="sm" /> {t('gui:settings_modal.reset_theme_label', 'Reset to Default')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.reset_theme_description', 'Restore all appearance settings to defaults')}
                                    </span>
                                </label>
                            </div>
                        </div>
                    )}

                    {activeTab === 'advanced' && !showUpdatePanel && (
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
                                        {t('gui:settings_modal.auto_generate_passwords_description', 'If a server\'s REST password is empty, fill it with a random one when the server starts. Existing passwords are never changed.')}
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

                            {/* 자동 포트포워딩 확인 (데몬이 서버를 만들 때·켜질 때) */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="globe" size="sm" />{' '}
                                        {t('gui:settings_modal.auto_port_check_label', 'Automatic Port Forwarding Check')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.auto_port_check_description', 'When a server is created and when Saba-chan starts, quietly check through the relay whether friends can reach your servers.')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={autoPortCheck}
                                    onChange={(checked) => useSettingsStore.getState().update({ autoPortCheck: checked })}
                                />
                            </div>

                            {/* 새 서버 기본값: 서버를 만들 때 데몬이 채운다. 만든 뒤에는 서버마다 바꾼다 */}
                            <div className="setting-item setting-item-group">
                                <div className="setting-item-row">
                                    <label className="setting-label">
                                        <span className="setting-title">
                                            <Icon name="server" size="sm" />{' '}
                                            {t('gui:settings_modal.new_server_defaults_label')}
                                        </span>
                                        <span className="setting-description">
                                            {t('gui:settings_modal.new_server_defaults_description')}
                                        </span>
                                    </label>
                                </div>
                                <div className="setting-sublist">
                                    <div className="setting-subitem">
                                        <span>{t('gui:settings_modal.new_server_backup_keep')}</span>
                                        <CustomDropdown
                                            className="setting-select"
                                            value={newServerBackupKeep}
                                            onChange={(val) => setOption({ newServerBackupKeep: Number(val) })}
                                            options={[...new Set([3, 5, 10, 20, 30, 50, newServerBackupKeep])]
                                                .sort((a, b) => a - b)
                                                .map((value) => ({
                                                    value,
                                                    label: t('gui:settings_modal.new_server_backup_keep_count', {
                                                        n: value,
                                                    }),
                                                }))}
                                        />
                                    </div>
                                    <div className="setting-subitem">
                                        <span>{t('gui:settings_modal.new_server_auto_restart')}</span>
                                        <SabaToggle
                                            size="sm"
                                            checked={newServerAutoRestart}
                                            onChange={(on) => setOption({ newServerAutoRestart: on })}
                                        />
                                    </div>
                                </div>
                            </div>

                            {/* 활동 기록 보관 기간: 지난 기록은 데몬이 지운다 */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="file" size="sm" /> {t('gui:settings_modal.audit_keep_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.audit_keep_description')}
                                    </span>
                                </label>
                                <CustomDropdown
                                    className="setting-select"
                                    value={auditKeepDays}
                                    onChange={(val) => setOption({ auditKeepDays: Number(val) })}
                                    options={[...new Set([0, 7, 30, 90, 365, auditKeepDays])]
                                        .sort((a, b) => a - b)
                                        .map((value) => ({
                                            value,
                                            label: value
                                                ? t('gui:settings_modal.audit_keep_days', { n: value })
                                                : t('gui:settings_modal.audit_keep_all'),
                                        }))}
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

                            {remoteDaemonAvailable() && <RemoteDaemonSection />}
                            {/* 원격 데몬을 다루는 동안은 경로를 그 PC의 폴더에서 고른다 (RemotePathPicker, D2). 데몬을 바꾸면 다시 읽는다 */}
                            {folderPathsAvailable() && <FolderPathsSection key={daemonTarget || 'local'} />}

                            {dataTransferAvailable() && <DataTransferSection key={daemonTarget || 'local'} />}

                            {unisolatedAvailable() && <UnisolatedSection />}

                            {extensionGuiAvailable() && <ExtensionGuiSection />}

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
                                                        message: 'The task finished successfully.',
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
                                                        message: 'An unexpected error occurred.',
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
                                                        message: 'A new update is available.',
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
                                                        message: 'Do you want to continue?',
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
                                                    window.showToast('ℹ️ Info toast message.', 'info', 3000);
                                            }}
                                        >
                                            <Icon name="info" size="sm" /> Toast (Info)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-success"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('✅ Success toast message.', 'success', 3000);
                                            }}
                                        >
                                            <Icon name="check" size="sm" /> Toast (Success)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-warning"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('⚠️ Warning toast message.', 'warning', 3000);
                                            }}
                                        >
                                            <Icon name="alertCircle" size="sm" /> Toast (Warning)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-toast-error"
                                            onClick={() => {
                                                window.showToast &&
                                                    window.showToast('❌ Error toast message.', 'error', 4000);
                                            }}
                                        >
                                            <Icon name="x" size="sm" /> Toast (Error)
                                        </button>
                                        <button
                                            className="gui-test-btn gui-test-progress"
                                            onClick={() => {
                                                if (!onTestProgressBar) return;
                                                onTestProgressBar({ message: 'Downloading...', percent: 0 });
                                                let p = 0;
                                                const iv = setInterval(() => {
                                                    p += Math.random() * 15 + 5;
                                                    if (p >= 100) {
                                                        p = 100;
                                                        onTestProgressBar({ message: 'Done!', percent: 100 });
                                                        clearInterval(iv);
                                                        setTimeout(() => onTestProgressBar(null), 1500);
                                                    } else {
                                                        onTestProgressBar({
                                                            message: `Downloading... ${Math.round(p)}%`,
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
                                                onTestProgressBar({ message: 'Processing...', indeterminate: true });
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
                                                    window.showToast('ℹ️ Info notification test', 'info', 3000, {
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
                                                    window.showToast('✅ Success notification test', 'success', 3000, {
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
                                                    window.showToast('❌ Error notification test', 'error', 4000, {
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

                            {/* 데몬 진단 정보 (GET /api/diagnostics) */}
                            {diagnosticsAvailable() && (
                                <div className="setting-item setting-item-stack">
                                    <label className="setting-label">
                                        <span className="setting-title">
                                            <Icon name="tool" size="sm" /> {t('gui:diagnostics.title')}
                                        </span>
                                        <span className="setting-description">{t('gui:diagnostics.description')}</span>
                                    </label>
                                    <DiagnosticsPanel />
                                </div>
                            )}

                            {/* GUI 설정 기본값으로 (데몬 /api/config/gui/reset) */}
                            {window.api?.configReset && <ResetGuiSettings />}

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
                                <Icon name="chevronRight" size="sm" color="var(--stop)" />
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
                                <span className="about-dev-nickname">{t('gui:settings_modal.dev_nickname')}</span>
                                <span className="about-dev-id">@WareAoba</span>
                                <span className="about-dev-bio">{t('gui:settings_modal.dev_bio')}</span>
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
                                <span>{t('gui:settings_modal.github_repo')}</span>
                            </button>
                        </div>

                        {/* 사바쨩 정보 카드 */}
                        <div className="about-card about-app-card">
                            <div className="about-app-logo-container">
                                <img
                                    src="./title.png"
                                    alt="Saba-chan"
                                    className="about-app-logo"
                                    onClick={() => {
                                        logoClicks.current += 1;
                                        if (logoClicks.current < 5) return;
                                        logoClicks.current = 0;
                                        safeShowToast(t('gui:settings_modal.name_egg'), 'info', 6000);
                                    }}
                                />
                                {appVersion && <span className="about-version-badge">v{appVersion}</span>}
                            </div>
                            <div className="about-app-info">
                                {/* 컴포넌트 버전 */}
                                <div className="about-component-list">
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="package" size="xs" /> Core</span>
                                        <span className="about-component-ver">{versionLabel(componentInfo?.components?.['saba-core'] || appVersion)}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="monitor" size="xs" /> GUI</span>
                                        <span className="about-component-ver">{versionLabel(componentInfo?.components?.gui || appVersion)}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="terminal" size="xs" /> CLI</span>
                                        <span className="about-component-ver">{versionLabel(componentInfo?.components?.cli || appVersion)}</span>
                                    </div>
                                    <div className="about-component-item">
                                        <span className="about-component-label"><Icon name="refresh" size="xs" /> Updater</span>
                                        <span className="about-component-ver">{versionLabel(componentInfo?.components?.updater || appVersion)}</span>
                                    </div>
                                </div>

                                {/* 마지막 업데이트 & 라이선스 */}
                                <div className="about-app-footer">
                                    {formatLastUpdated(componentInfo?.lastUpdated, i18n.language) && (
                                        <span className="about-app-meta">
                                            <Icon name="clock" size="xs" /> {formatLastUpdated(componentInfo.lastUpdated, i18n.language)}
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

                {!showUpdatePanel && !isPage && (
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

/** 데몬의 lastUpdated(unix 초, 문자열) → 날짜 글자. 읽을 수 없으면 null */
/** 컴포넌트 버전 표시. 모르면 대시 */
const versionLabel = (v) => (v ? `v${v}` : '—');

export function formatLastUpdated(secs, language) {
    const n = Number(secs);
    if (!secs || !Number.isFinite(n)) return null;
    return new Date(n * 1000).toLocaleDateString(language, { year: 'numeric', month: 'long', day: 'numeric' });
}

export default SettingsModal;

/** 두 번 눌러야 실행되는 "GUI 설정 기본값으로" 줄 */
function ResetGuiSettings() {
    const { t } = useTranslation(['gui']);
    const [armed, setArmed] = useState(false);
    const [busy, setBusy] = useState(false);
    return (
        <div className="setting-item">
            <label className="setting-label">
                <span className="setting-title">
                    <Icon name="refresh" size="sm" /> {t('gui:settings_modal.reset_gui_label')}
                </span>
                <span className="setting-description">{t('gui:settings_modal.reset_gui_description')}</span>
            </label>
            <button
                type="button"
                className={clsx('btn', armed ? 'btn-danger' : 'btn-secondary')}
                disabled={busy}
                onBlur={() => setArmed(false)}
                onClick={async () => {
                    if (!armed) {
                        setArmed(true);
                        return;
                    }
                    setBusy(true);
                    const ok = await useSettingsStore.getState().resetToDefaults();
                    setBusy(false);
                    setArmed(false);
                    safeShowToast(
                        t(ok ? 'gui:settings_modal.reset_gui_done' : 'gui:settings_modal.reset_gui_failed'),
                        ok ? 'success' : 'error',
                        3000,
                    );
                }}
            >
                {armed ? t('gui:settings_modal.reset_gui_confirm') : t('gui:settings_modal.reset_gui_button')}
            </button>
        </div>
    );
}
