import clsx from 'clsx';
import { type CSSProperties, type ReactNode, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useDevMode } from '../../hooks/useDevMode';
import { useModalClose } from '../../hooks/useModalClose';
import { DiagnosticsPanel, diagnosticsAvailable } from '../DiagnosticsPanel';
import { useSettingsStore, DEFAULT_IPC_PORT } from '../../stores/useSettingsStore';
import type { ModalConfig, ProgressBarConfig } from '../../stores/useUIStore';
import type { ApiBody, SoundSettings } from '../../types/daemon';
import {
    getTheme,
    setTheme as saveTheme,
    getCustomTheme,
    setCustomTheme,
    resetCustomTheme,
    ACCENT_PRESETS,
    applyCustomTheme,
    applyWindowBackdrop,
    WINDOW_BACKDROP_DEFAULTS,
    type ThemeConfig,
    type ThemeMode,
} from '../../utils/themeManager';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { OklchPicker } from '../ui/OklchPicker';
import { TOKENS } from '../../utils/syntaxHighlight';
import { useSlotPages } from '../ExtensionSlot';
import { Icon } from '../Icon';
import { safeShowToast } from '../../utils/helpers';
import { LANGUAGE_OPTIONS } from '../../utils/languages';
import { CONSOLE_FONT_CHOICES, CONSOLE_FONT_NAMES, CONSOLE_FONT_SAMPLE, CONSOLE_FONT_SLOTS } from '../../utils/consoleFonts';
import { SabaToggle } from '../ui/SabaUI';
import { DataTransferSection, dataTransferAvailable } from './DataTransferSection';
import { FolderPathsSection, folderPathsAvailable } from './FolderPathsSection';
import { RemoteDaemonSection, remoteDaemonAvailable } from './RemoteDaemonSection';
import { useDaemonTarget } from '../shell/DaemonSwitcher';
import { AutostartSetting, autostartAvailable } from './AutostartSetting';
import { WindowBackdropSetting } from './WindowBackdropSetting';
import { NetTraceSetting, netTraceAvailable } from './NetTraceSetting';
import { AgentLink, agentLinkAvailable } from '../discord/AgentLink';
import { PrivacyErase, privacyEraseAvailable } from '../discord/PrivacyErase';
import { useDiscordStore } from '../../stores/useDiscordStore';
import { ExtensionGuiSection, extensionGuiAvailable } from './ExtensionGuiSection';
import { ExtensionSettingsPanel } from './ExtensionSettingsPanel';
import { UnisolatedSection, unisolatedAvailable } from './UnisolatedSection';
import SabaStorage from './SabaStorage';
import { AboutPage } from './AboutPage';
import { type SettingsTab, SETTINGS_TAB_LABELS as TAB_LABELS, SETTINGS_TABS as TABS } from './settingsTabs';

// 효과음 종류별 스위치 (DEFAULT_SOUNDS 키 → 이름)
const SOUND_KINDS: [Exclude<keyof SoundSettings, 'enabled'>, string][] = [
    ['serverStart', 'gui:settings_modal.sound_server_start'],
    ['serverStop', 'gui:settings_modal.sound_server_stop'],
    ['crash', 'gui:settings_modal.sound_crash'],
    ['install', 'gui:settings_modal.sound_install'],
    ['update', 'gui:settings_modal.sound_update'],
];

const TAB_ICONS: Record<SettingsTab, string> = {
    general: 'settings',
    notify: 'bell',
    appearance: 'palette',
    console: 'terminal',
    servers: 'server',
    connection: 'network',
    storage: 'folder',
    sabaStorage: 'package',
    security: 'key',
    troubleshoot: 'tool',
    about: 'info',
};

/** 모든 구문 강조 색을 기본색으로 (빈 값) */
const clearedTokenColors = () => Object.fromEntries(TOKENS.map((token) => [token, '']));

/** 지금 테마가 정한 토큰 기본색 (사용자 색을 걷어낸 값) */
function tokenDefaultColor(token: string) {
    const root = document.documentElement;
    const name = `--tok-${token}`;
    const custom = root.style.getPropertyValue(name);
    if (custom) root.style.removeProperty(name);
    const value = getComputedStyle(root).getPropertyValue(name).trim();
    if (custom) root.style.setProperty(name, custom);
    return value || '#cdd6f4';
}

interface SettingsModalProps {
    isOpen: boolean;
    onClose: () => void;
    refreshInterval: number;
    onRefreshIntervalChange?: (value: number) => void;
    ipcPort: number;
    onIpcPortChange?: (port: number) => void;
    consoleBufferSize: number;
    onConsoleBufferSizeChange?: (value: number) => void;
    onTestModal?: (modal: ModalConfig | null) => void;
    onTestProgressBar?: (config: ProgressBarConfig | null) => void;
    onTestWaitingImage?: () => void;
    onTestLoadingScreen?: () => void;
    initialView?: string | null;
    discordCloudRelayUrl?: string;
    onDiscordCloudRelayUrlChange?: (value: string) => void;
    variant?: 'modal' | 'page';
}

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
}: SettingsModalProps) {
    const { t, i18n } = useTranslation(['gui', 'common']);
    const daemonTarget = useDaemonTarget();
    const [chosenTab, setActiveTab] = useState<SettingsTab | `ext:${string}`>('general');
    // 익스텐션 설정 탭 (`Settings.tab` slot): "정보" 바로 위. 그 익스텐션이 꺼지면 일반 탭으로
    const extTabs = useSlotPages('Settings.tab');
    const tabOrder: string[] = [...TABS.filter((id) => id !== 'about'), ...extTabs.map((p) => p.id), 'about'];
    const activeTab = tabOrder.includes(chosenTab) ? chosenTab : 'general';
    const activeExtTab = extTabs.find((p) => p.id === activeTab);
    // 사바 스토리지를 열 때 처음 보일 탭 (설정을 '익스텐션'으로 열면 익스텐션 탭)
    const [storageTab, setStorageTab] = useState('components');
    const [localRefreshInterval, setLocalRefreshInterval] = useState(refreshInterval);
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);
    const [selectedTheme, setSelectedTheme] = useState(getTheme());
    const [customTheme, setCustomThemeState] = useState(getCustomTheme());
    const [slideDirection, setSlideDirection] = useState('');
    const [guiTestOpen, setGuiTestOpen] = useState(false);
    const [localIpcPort, setLocalIpcPort] = useState<number | string>(ipcPort || DEFAULT_IPC_PORT);
    const [ipcPortChanged, setIpcPortChanged] = useState(false);
    const [ipcPortError, setIpcPortError] = useState('');
    const [localConsoleBuffer, setLocalConsoleBuffer] = useState(consoleBufferSize || 2000);
    // 받기 전에는 모른다: 지어낸 버전을 보이지 않는다
    const [appVersion, setAppVersion] = useState<string | null>(null);
    const [componentInfo, setComponentInfo] = useState<ApiBody | null>(null);
    // 이스터에그: 정보 페이지 로고를 다섯 번 누르면 이름 해명
    const logoClicks = useRef(0);
    const { isClosing, requestClose } = useModalClose(onClose);

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
    const consoleFonts = useSettingsStore((s) => s.consoleFonts);
    const consoleTimestampDate = useSettingsStore((s) => s.consoleTimestampDate);
    const consoleWrap = useSettingsStore((s) => s.consoleWrap);
    const consoleAutoScroll = useSettingsStore((s) => s.consoleAutoScroll);
    const autoRefresh = useSettingsStore((s) => s.autoRefresh);
    const discordHostId = useDiscordStore((s) => s.discordCloudHostId);
    const setOption = (patch: Parameters<ReturnType<typeof useSettingsStore.getState>['update']>[0]) => useSettingsStore.getState().update(patch);
    const setSound = (key: keyof SoundSettings, on: boolean) => useSettingsStore.getState().update({ sounds: { ...sounds, [key]: on } });

    // 외부에서 initialView 지정으로 열렸을 때 해당 탭 자동 진입 (설정 탭 이름이면 그 탭: 화면 주소 settings/<탭>)
    useEffect(() => {
        if (isOpen && (initialView === 'update' || initialView === 'extensions')) {
            setStorageTab(initialView === 'extensions' ? 'extensions' : 'components');
            setActiveTab('sabaStorage');
        } else if (isOpen && ((TABS as readonly string[]).includes(initialView ?? '') || initialView?.startsWith('ext:'))) {
            setActiveTab(initialView as SettingsTab | `ext:${string}`);
        }
    }, [isOpen, initialView]);

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
    const handleTabChange = (newTab: SettingsTab | `ext:${string}`) => {
        const oldIndex = tabOrder.indexOf(activeTab);
        const newIndex = tabOrder.indexOf(newTab);
        if (newTab === activeTab) return;
        // 세로 탭: 아래 탭이면 아래에서, 위 탭이면 위에서 들어온다
        setSlideDirection(newIndex > oldIndex ? 'slide-up' : 'slide-down');
        setActiveTab(newTab);
    };

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
                    let result: ApiBody | undefined;
                    try {
                        result = await window.api.launchUninstaller();
                    } catch (err) {
                        console.error('Failed to launch uninstaller:', err);
                        result = { error: String((err as Error)?.message || err) };
                    }
                    // 성공하면 GUI가 곧 닫힌다. 못 띄웠으면 말없이 끝나지 않게 알린다
                    if (!result?.success) {
                        window.showToast?.(
                            t('gui:settings_modal.uninstall_failed', { error: result?.error || '' }),
                            'error',
                            6000,
                        );
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
    const handleRefreshIntervalChange = (value: number) => {
        setLocalRefreshInterval(value);
        if (onRefreshIntervalChange) {
            onRefreshIntervalChange(value);
        }
    };

    // 언어 변경 핸들러
    const handleLanguageChange = async (lng: string) => {
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
    const handleThemeChange = (theme: ThemeMode) => {
        setSelectedTheme(theme);
        saveTheme(theme);
        useSettingsStore.getState().update({ theme });
        // Re-apply custom theme after base theme change (accent colors adapt to light/dark)
        setTimeout(() => applyCustomTheme(customTheme), 50);
    };

    // 커스텀 테마 속성 변경 핸들러
    const handleCustomThemeChange = (partial: Partial<ThemeConfig>) => {
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
            // 부분 저장은 객체를 키별로 합친다: 지운 색은 빈 값으로 보내야 저장된 색이 사라진다
            consoleTokenColors: clearedTokenColors(),
            sidebarCompact: defaults.sidebarCompact,
            windowBackdrop: WINDOW_BACKDROP_DEFAULTS,
        });
        void applyWindowBackdrop(WINDOW_BACKDROP_DEFAULTS);
    };

    // IPC 포트 변경 핸들러
    const handleIpcPortChange = (value: string) => {
        const port = parseInt(value, 10);
        setLocalIpcPort(value);
        if (Number.isNaN(port) || port < 1024 || port > 65535) {
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
    const handleConsoleBufferChange = (value: number) => {
        setLocalConsoleBuffer(value);
        if (onConsoleBufferSizeChange) {
            onConsoleBufferSizeChange(value);
        }
    };

    if (!isOpen) {
        return null;
    }

    const isPage = variant === 'page';

    const header = (
        <div className="settings-modal-header">
            <h2 style={{ fontSize: '1.3rem' }}>{t('gui:settings_modal.title')}</h2>
        </div>
    );
    // 페이지에서는 서버·디스코드·작업 기록 화면의 왼쪽 목록과 같은 줄 모양을 쓴다
    const tabList = (
        <div
            className={isPage ? 'server-list dc-nav-list' : 'settings-modal-tabs'}
            role="tablist"
            aria-orientation="vertical"
            aria-label={t('gui:settings_modal.title')}
        >
            {tabOrder.map((id) => {
                const ext = extTabs.find((p) => p.id === id);
                const label = ext ? ext.label : t(TAB_LABELS[id as SettingsTab]);
                const icon = ext ? ext.icon : TAB_ICONS[id as SettingsTab];
                return (
                    <button
                        key={id}
                        type="button"
                        role="tab"
                        id={`settings-tab-${id}`}
                        aria-selected={activeTab === id}
                        aria-controls={`settings-panel-${id}`}
                        className={
                            isPage
                                ? clsx('server-row dc-nav-row', { selected: activeTab === id })
                                : clsx('settings-tab', { active: activeTab === id })
                        }
                        onClick={() => handleTabChange(id as SettingsTab | `ext:${string}`)}
                    >
                        {isPage ? (
                            <>
                                <span className="server-row-icon">
                                    <Icon name={icon} size="sm" />
                                </span>
                                <span className="server-row-text">
                                    <span className="server-row-name">{label}</span>
                                </span>
                            </>
                        ) : (
                            <>
                                <Icon name={icon} size="sm" />
                                <span>{label}</span>
                            </>
                        )}
                    </button>
                );
            })}
        </div>
    );

    return (
        <div
            className={clsx(isPage ? 'settings-page' : 'settings-modal-overlay', { closing: isClosing && !isPage })}
            onClick={isPage ? undefined : requestClose}
        >
            <div className="settings-modal-container" onClick={(e) => e.stopPropagation()}>
                {!isPage && header}

                <div className="settings-body">
                {(isPage ? <aside className="server-list-pane dc-nav settings-nav">{tabList}</aside> : tabList)}

                <div className="settings-viewport">
                <div className={'settings-modal-content'}>
                    {isPage && header}

                    {activeTab === 'general' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="general"
                            id="settings-panel-general"
                            role="tabpanel"
                            aria-labelledby="settings-tab-general"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.general)}</h3>

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
                        </div>
                    )}

                    {activeTab === 'notify' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="notify"
                            id="settings-panel-notify"
                            role="tabpanel"
                            aria-labelledby="settings-tab-notify"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.notify)}</h3>

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
                        </div>
                    )}

                    {activeTab === 'appearance' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="appearance"
                            id="settings-panel-appearance"
                            role="tabpanel"
                            aria-labelledby="settings-tab-appearance"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.appearance)}</h3>

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
                                                background: `linear-gradient(in oklch 135deg, ${preset.primary} 0%, ${preset.secondary} 100%)`,
                                            }}
                                            onClick={() =>
                                                handleCustomThemeChange({
                                                    accentColor: preset.primary,
                                                    accentSecondary: preset.secondary,
                                                })
                                            }
                                            data-tooltip={t(`gui:settings_modal.accent_${preset.name}`, preset.name)}
                                            aria-label={t(`gui:settings_modal.accent_${preset.name}`, preset.name)}
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

                            {/* ── 2-2. 창 바탕 반투명 ── */}
                            <WindowBackdropSetting />

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
                                        style={{ '--fill': `${((customTheme.fontScale - 50) / 150) * 100}%` } as CSSProperties}
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

                            {/* ── 사이드바 좁게 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="server" size="sm" /> {t('gui:settings_modal.sidebar_compact_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.sidebar_compact_description')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={!!customTheme.sidebarCompact}
                                    onChange={(checked) => handleCustomThemeChange({ sidebarCompact: checked })}
                                />
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

                    {activeTab === 'console' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="console"
                            id="settings-panel-console"
                            role="tabpanel"
                            aria-labelledby="settings-tab-console"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.console)}</h3>

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

                            {/* ── 5-1b. 구문 강조 색 (토큰마다, 빈 값이면 테마 기본색) ── */}
                            {customTheme.consoleSyntaxHighlight && (
                                <div className="setting-item setting-item-vertical">
                                    <div className="setting-label-row">
                                        <label className="setting-label">
                                            <span className="setting-title">
                                                <Icon name="palette" size="sm" /> {t('gui:settings_modal.console_token_colors_label')}
                                            </span>
                                            <span className="setting-description">
                                                {t('gui:settings_modal.console_token_colors_description')}
                                            </span>
                                        </label>
                                        <button
                                            type="button"
                                            className="btn btn-ghost btn-sm"
                                            disabled={!Object.values(customTheme.consoleTokenColors || {}).some(Boolean)}
                                            onClick={() => handleCustomThemeChange({ consoleTokenColors: clearedTokenColors() })}
                                        >
                                            <Icon name="undo" size="sm" /> {t('gui:settings_modal.console_token_colors_reset')}
                                        </button>
                                    </div>
                                    <div className="console-token-grid">
                                        {TOKENS.map((token) => {
                                            const custom = customTheme.consoleTokenColors?.[token] || '';
                                            const label = t(`gui:settings_modal.console_token_${token}`);
                                            return (
                                                <div key={token} className={clsx('console-token-row', { custom })}>
                                                    <span className={`console-token-name tok-${token}`}>{label}</span>
                                                    <OklchPicker
                                                        label={label}
                                                        value={custom || tokenDefaultColor(token)}
                                                        onChange={(hex) =>
                                                            handleCustomThemeChange({
                                                                consoleTokenColors: { ...customTheme.consoleTokenColors, [token]: hex },
                                                            })
                                                        }
                                                    />
                                                </div>
                                            );
                                        })}
                                    </div>
                                </div>
                            )}

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

                            {/* ── 긴 줄 접기 · 새 줄 따라가기 ── */}
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_wrap_label')}
                                    </span>
                                    <span className="setting-description">{t('gui:settings_modal.console_wrap_description')}</span>
                                </label>
                                <SabaToggle checked={consoleWrap} onChange={(checked) => setOption({ consoleWrap: checked })} />
                            </div>
                            <div className="setting-item">
                                <label className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_auto_scroll_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_auto_scroll_description')}
                                    </span>
                                </label>
                                <SabaToggle
                                    checked={consoleAutoScroll}
                                    onChange={(checked) => setOption({ consoleAutoScroll: checked })}
                                />
                            </div>

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

                            {/* 콘솔 글꼴: 글자마다 하나 (utils/consoleFonts.js) */}
                            <div className="setting-item setting-item-vertical">
                                <div className="setting-label">
                                    <span className="setting-title">
                                        <Icon name="terminal" size="sm" /> {t('gui:settings_modal.console_fonts_label')}
                                    </span>
                                    <span className="setting-description">
                                        {t('gui:settings_modal.console_fonts_description')}
                                    </span>
                                </div>
                                <div className="console-font-grid">
                                    {CONSOLE_FONT_SLOTS.map((slot) => (
                                        <div key={slot} className="console-font-slot">
                                            <span className="console-font-slot-label">
                                                {t(`gui:settings_modal.console_fonts_${slot}`)}
                                            </span>
                                            <CustomDropdown
                                                className="setting-select"
                                                ariaLabel={t(`gui:settings_modal.console_fonts_${slot}`)}
                                                value={consoleFonts[slot]}
                                                onChange={(val) => setOption({ consoleFonts: { ...consoleFonts, [slot]: val } })}
                                                options={CONSOLE_FONT_CHOICES[slot].map((value) => ({
                                                    value,
                                                    label: CONSOLE_FONT_NAMES[value as keyof typeof CONSOLE_FONT_NAMES],
                                                }))}
                                            />
                                        </div>
                                    ))}
                                </div>
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
                                        style={{ '--fill': `${((customTheme.consoleFontScale - 50) / 150) * 100}%` } as CSSProperties}
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
                                    const tok = (token: string, text: string) =>
                                        customTheme.consoleSyntaxHighlight ? <span className={`tok-${token}`}>{text}</span> : text;
                                    return (
                                        <>
                                            <div className="console-preview-line">
                                                {tok('timestamp', '[12:34:56]')} Server started on port {tok('number', '25565')}
                                            </div>
                                            {/* 글꼴 미리보기: 영문·한글·일본어·한자 한 줄 (글자마다 고른 글꼴) */}
                                            <div className="console-preview-line">
                                                {tok('timestamp', '[12:35:02]')} FPS {tok('number', '59')} {CONSOLE_FONT_SAMPLE}
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
                        </div>
                    )}

                    {activeTab === 'servers' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="servers"
                            id="settings-panel-servers"
                            role="tabpanel"
                            aria-labelledby="settings-tab-servers"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.servers)}</h3>

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

                            {/* 포트·서버별 네트워크 (기본 꺼짐, Windows만): 시스템 모니터를 펼쳐 둔 동안 데몬이 센다 */}
                            {netTraceAvailable() && <NetTraceSetting />}

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
                        </div>
                    )}

                    {activeTab === 'connection' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="connection"
                            id="settings-panel-connection"
                            role="tabpanel"
                            aria-labelledby="settings-tab-connection"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.connection)}</h3>

                            {remoteDaemonAvailable() && <RemoteDaemonSection />}

                            {/* AI 에이전트(MCP) 연결: PC 카드와 같은 카드로 보인다 */}
                            {discordHostId && agentLinkAvailable() && <AgentLink />}

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
                        </div>
                    )}

                    {activeTab === 'storage' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="storage"
                            id="settings-panel-storage"
                            role="tabpanel"
                            aria-labelledby="settings-tab-storage"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.storage)}</h3>

                            {/* 원격 데몬을 다루는 동안은 경로를 그 PC의 폴더에서 고른다 (RemotePathPicker, D2). 데몬을 바꾸면 다시 읽는다 */}
                            {folderPathsAvailable() && <FolderPathsSection key={daemonTarget || 'local'} />}

                            {dataTransferAvailable() && <DataTransferSection key={daemonTarget || 'local'} />}
                        </div>
                    )}

                    {activeTab === 'security' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="security"
                            id="settings-panel-security"
                            role="tabpanel"
                            aria-labelledby="settings-tab-security"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.security)}</h3>

                            {unisolatedAvailable() && <UnisolatedSection />}

                            {extensionGuiAvailable() && <ExtensionGuiSection />}

                            {/* 디스코드 계정: 개인정보 지우기 (디스코드 화면에서 옮겨 옴). AI 에이전트 연결은 연결 탭에 있다 */}
                            {discordHostId && privacyEraseAvailable() && (
                                <PrivacyErase
                                    className="setting-item"
                                    onErased={() =>
                                        // 데몬이 연결을 끊었다: 디스코드 화면이 연결 전으로 돌아가게 비운다
                                        useDiscordStore.getState().update({ discordCloudHostId: '', cloudNodes: [], cloudMembers: {} })
                                    }
                                />
                            )}
                        </div>
                    )}

                    {activeTab === 'troubleshoot' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="troubleshoot"
                            id="settings-panel-troubleshoot"
                            role="tabpanel"
                            aria-labelledby="settings-tab-troubleshoot"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.troubleshoot)}</h3>

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
                            {typeof window.api?.configReset === 'function' && <ResetGuiSettings />}

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
                        </div>
                    )}

                    {activeExtTab && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key={activeExtTab.id}
                            id={`settings-panel-${activeExtTab.id}`}
                            role="tabpanel"
                            aria-labelledby={`settings-tab-${activeExtTab.id}`}
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{activeExtTab.label}</h3>
                            <ExtensionSettingsPanel entry={activeExtTab.entry} />
                        </div>
                    )}

                    {activeTab === 'about' && (
                        <div
                            className={clsx('settings-tab-content', slideDirection)}
                            key="about"
                            id="settings-panel-about"
                            role="tabpanel"
                            aria-labelledby="settings-tab-about"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.about)}</h3>
                            <FitHeight>
                            <AboutPage
                                active
                                appVersion={appVersion}
                                componentInfo={componentInfo}
                                lastUpdated={formatLastUpdated(componentInfo?.lastUpdated, i18n.language)}
                                onLogoClick={() => {
                                    logoClicks.current += 1;
                                    if (logoClicks.current < 5) return;
                                    logoClicks.current = 0;
                                    safeShowToast(t('gui:settings_modal.name_egg'), 'info', 6000);
                                }}
                            />
                            </FitHeight>
                        </div>
                    )}

                    {/* 사바 스토리지는 설정을 열 때 함께 띄워 목록을 미리 받아 두고, 다른 탭에서는 숨긴다 */}
                    {isOpen && (
                        <div
                            className={clsx('settings-tab-content', activeTab === 'sabaStorage' && slideDirection)}
                            hidden={activeTab !== 'sabaStorage'}
                            key="sabaStorage"
                            id="settings-panel-sabaStorage"
                            role="tabpanel"
                            aria-labelledby="settings-tab-sabaStorage"
                            onAnimationEnd={() => setSlideDirection('')}
                        >
                            <h3>{t(TAB_LABELS.sabaStorage)}</h3>
                            <SabaStorage
                                key={storageTab}
                                devMode={devMode}
                                initialTab={storageTab}
                            />
                        </div>
                    )}
                </div>
                </div>
                </div>

                {!isPage && (
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
export function formatLastUpdated(secs: string | number | null | undefined, language: string) {
    const n = Number(secs);
    if (!secs || !Number.isFinite(n)) return null;
    return new Date(n * 1000).toLocaleDateString(language, { year: 'numeric', month: 'long', day: 'numeric' });
}

export default SettingsModal;

/**
 * 안의 내용이 스크롤 칸(.settings-modal-content)의 남은 높이를 넘으면 통째로 줄여(scale) 한 화면에 담는다.
 * 칸 크기가 바뀌거나 그림을 다 받으면 다시 잰다. 내용 자신의 크기 변화는 보지 않는다 (줄이면 다시 재는 고리가 생긴다)
 */
function FitHeight({ children }: { children: ReactNode }) {
    const ref = useRef<HTMLDivElement>(null);
    useLayoutEffect(() => {
        const el = ref.current;
        const box = el?.closest<HTMLElement>('.settings-modal-content');
        if (!el || !box) return;
        // zoom은 폭까지 다시 계산해 그림이 커지므로, 배치는 그대로 두고 그리기만 줄인다(transform)
        const fit = () => {
            const padBottom = Number.parseFloat(getComputedStyle(box).paddingBottom) || 0;
            const room = box.getBoundingClientRect().bottom - padBottom - el.getBoundingClientRect().top;
            const need = el.offsetHeight;
            const scale = need > room && room > 0 ? Math.max(0.4, room / need) : 1;
            el.style.transform = scale < 1 ? `scale(${scale})` : '';
        };
        fit();
        // 테스트 환경(jsdom)에는 ResizeObserver가 없다
        const observer = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(fit);
        observer?.observe(box);
        el.addEventListener('load', fit, true);
        return () => {
            observer?.disconnect();
            el.removeEventListener('load', fit, true);
        };
    }, []);
    return (
        <div ref={ref} className="fit-height" style={{ transformOrigin: 'top center' }}>
            {children}
        </div>
    );
}

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
