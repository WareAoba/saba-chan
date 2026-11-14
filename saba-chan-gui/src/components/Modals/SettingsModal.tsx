import clsx from 'clsx';
import { type ReactNode, useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useDevMode } from '../../hooks/useDevMode';
import { usePageActive } from '../../hooks/usePageActive';
import { useSettingsStore, DEFAULT_IPC_PORT } from '../../stores/useSettingsStore';
import type { ModalConfig, ProgressBarConfig } from '../../stores/useUIStore';
import type { ApiBody } from '../../types/daemon';
import {
    getTheme,
    setTheme as saveTheme,
    getCustomTheme,
    setCustomTheme,
    resetCustomTheme,
    applyCustomTheme,
    applyWindowBackdrop,
    WINDOW_BACKDROP_DEFAULTS,
    type ThemeConfig,
    type ThemeMode,
} from '../../utils/themeManager';
import { useSlotPages } from '../ExtensionSlot';
import { Icon } from '../Icon';
import { safeShowToast } from '../../utils/helpers';
import { useDaemonTarget } from '../shell/DaemonSwitcher';
import { ExtensionSettingsPanel } from './ExtensionSettingsPanel';
import SabaStorage from './SabaStorage';
import { type SettingsTab, SETTINGS_TAB_LABELS as TAB_LABELS, SETTINGS_TABS as TABS } from './settingsTabs';
import { AboutTab } from './settings/AboutTab';
import { AppearanceTab } from './settings/AppearanceTab';
import { ConnectionTab } from './settings/ConnectionTab';
import { ConsoleTab } from './settings/ConsoleTab';
import { GeneralTab } from './settings/GeneralTab';
import { NotifyTab } from './settings/NotifyTab';
import { clearedTokenColors, setOption } from './settings/options';
import { SecurityTab } from './settings/SecurityTab';
import { ServersTab } from './settings/ServersTab';
import { StorageTab } from './settings/StorageTab';
import { TroubleshootTab } from './settings/TroubleshootTab';
import { SaveBarHost } from '../ui/SaveBar';
import { failed } from '../../utils/apiResult';

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

interface SettingsModalProps {
    onTestModal?: (modal: ModalConfig | null) => void;
    onTestProgressBar?: (config: ProgressBarConfig | null) => void;
    onTestWaitingImage?: () => void;
    onTestLoadingScreen?: () => void;
    initialView?: string | null;
    discordCloudRelayUrl?: string;
    onDiscordCloudRelayUrlChange?: (value: string) => void;
}

/** 설정 화면 (앱의 설정 페이지) */
function SettingsModal({
    onTestModal,
    onTestProgressBar,
    onTestWaitingImage,
    onTestLoadingScreen,
    initialView,
    discordCloudRelayUrl,
    onDiscordCloudRelayUrlChange,
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
    const [selectedLanguage, setSelectedLanguage] = useState(i18n.language);
    const [selectedTheme, setSelectedTheme] = useState(getTheme());
    const [customTheme, setCustomThemeState] = useState(getCustomTheme());
    const [slideDirection, setSlideDirection] = useState('');
    const [guiTestOpen, setGuiTestOpen] = useState(false);
    const ipcPort = useSettingsStore((s) => s.ipcPort) || DEFAULT_IPC_PORT;
    // 지금 데몬이 쓰는 포트(화면을 열 때 값). 바꾸면 다시 시작해야 적용된다
    const [runningIpcPort] = useState(ipcPort);
    // 받을 수 없는 포트를 쓰는 중이면 그 글자 (저장하지 않는다)
    // 받기 전에는 모른다: 지어낸 버전을 보이지 않는다
    const [appVersion, setAppVersion] = useState<string | null>(null);
    const [componentInfo, setComponentInfo] = useState<ApiBody | null>(null);
    // 이스터에그: 정보 페이지 로고를 다섯 번 누르면 이름 해명
    const logoClicks = useRef(0);

    const devMode = useDevMode();

    // 외부에서 initialView 지정으로 열렸을 때 해당 탭 자동 진입 (설정 탭 이름이면 그 탭: 화면 주소 settings/<탭>)
    // 숨겨 둔 설정 탭에 같은 바로 가기로 다시 와도 그 칸을 연다
    const active = usePageActive();
    useEffect(() => {
        if (!active) return;
        // 화면 주소 settings/sabaStorage/components 는 'components'로, openSettings('update')는 'update'로 온다
        if (initialView === 'update' || initialView === 'components' || initialView === 'extensions' || initialView === 'modules') {
            setStorageTab(initialView === 'update' ? 'components' : initialView);
            setActiveTab('sabaStorage');
        } else if ((TABS as readonly string[]).includes(initialView ?? '') || initialView?.startsWith('ext:')) {
            setActiveTab(initialView as SettingsTab | `ext:${string}`);
        }
    }, [initialView, active]);

    // 앱 버전 & 컴포넌트 정보 가져오기
    useEffect(() => {
        if (window.api?.getAppVersion) {
            window.api.getAppVersion().then((v) => typeof v === 'string' && setAppVersion(v)).catch(() => {});
        }
        if (window.api?.getComponentInfo) {
            window.api.getComponentInfo().then((info) => info && setComponentInfo(info)).catch(() => {});
        }
    }, []);

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

    // 현재 언어 동기화
    useEffect(() => {
        setSelectedLanguage(i18n.language);
    }, [i18n.language]);

    // 언어 변경 핸들러
    const handleLanguageChange = async (lng: string) => {
        setSelectedLanguage(lng);
        // 1. config.json의 gui에 먼저 확실히 저장 (데몬과 공유하는 원천)
        if (window.api?.languageSet) {
            // 저장하지 못하면 알린다: 화면은 바뀌어도 다음 실행에 옛 언어로 돌아간다 (G-U6)
            try {
                const res = await window.api.languageSet(lng);
                if (failed(res)) safeShowToast(t('gui:settings.save_error'), 'error');
            } catch (err) {
                console.error('Failed to save language to settings:', err);
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


    // 탭 한 칸: 고른 탭만 그리고, 들어올 때 탭 순서에 따라 위·아래에서 미끄러진다
    const panel = (id: SettingsTab, body: ReactNode) =>
        activeTab === id && (
            <div
                className={clsx('settings-tab-content', slideDirection)}
                key={id}
                id={`settings-panel-${id}`}
                role="tabpanel"
                aria-labelledby={`settings-tab-${id}`}
                onAnimationEnd={() => setSlideDirection('')}
            >
                <h2 className="settings-tab-title">{t(TAB_LABELS[id])}</h2>
                {body}
            </div>
        );

    // 서버·디스코드·작업 기록 화면의 왼쪽 목록과 같은 줄 모양을 쓴다
    const tabList = (
        <div
            className="server-list dc-nav-list"
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
                        data-spot={`settings.tab.${id}`}
                        type="button"
                        role="tab"
                        id={`settings-tab-${id}`}
                        aria-selected={activeTab === id}
                        aria-controls={`settings-panel-${id}`}
                        className={clsx('server-row dc-nav-row', { selected: activeTab === id })}
                        onClick={() => handleTabChange(id as SettingsTab | `ext:${string}`)}
                    >
                        <span className="server-row-icon">
                            <Icon name={icon} size="sm" />
                        </span>
                        <span className="server-row-text">
                            <span className="server-row-name">{label}</span>
                        </span>
                    </button>
                );
            })}
        </div>
    );

    return (
        <div className="settings-page">
            <div className="settings-modal-container">
                <div className="settings-body">
                <aside className="server-list-pane dc-nav settings-nav">{tabList}</aside>

                <div className="settings-viewport">
                <div className={'settings-modal-content'}>
                    {/* 즉시 저장할 수 없는 칸(포트·주소)의 바뀜을 모아 아래에서 올라오는 막대로 묻는다 */}
                    <SaveBarHost>

                    {panel(
                        'general',
                        <GeneralTab selectedLanguage={selectedLanguage} handleLanguageChange={handleLanguageChange} />,
                    )}

                    {panel(
                        'notify',
                        <NotifyTab />,
                    )}

                    {panel(
                        'appearance',
                        <AppearanceTab
                            selectedTheme={selectedTheme}
                            customTheme={customTheme}
                            handleThemeChange={handleThemeChange}
                            handleCustomThemeChange={handleCustomThemeChange}
                            handleResetTheme={handleResetTheme}
                        />,
                    )}

                    {panel(
                        'console',
                        <ConsoleTab customTheme={customTheme} handleCustomThemeChange={handleCustomThemeChange} />,
                    )}

                    {panel(
                        'servers',
                        <ServersTab />,
                    )}

                    {panel(
                        'connection',
                        <ConnectionTab
                            ipcPort={ipcPort}
                            runningIpcPort={runningIpcPort}
                            onIpcPortSave={(port) => setOption({ ipcPort: port })}
                            discordCloudRelayUrl={discordCloudRelayUrl}
                            onDiscordCloudRelayUrlChange={onDiscordCloudRelayUrlChange}
                        />,
                    )}

                    {panel(
                        'storage',
                        <StorageTab daemonTarget={daemonTarget} />,
                    )}

                    {panel(
                        'security',
                        <SecurityTab />,
                    )}

                    {panel(
                        'troubleshoot',
                        <TroubleshootTab
                            devMode={devMode}
                            guiTestOpen={guiTestOpen}
                            setGuiTestOpen={setGuiTestOpen}
                            handleUninstall={handleUninstall}
                            onTestModal={onTestModal}
                            onTestProgressBar={onTestProgressBar}
                            onTestWaitingImage={onTestWaitingImage}
                            onTestLoadingScreen={onTestLoadingScreen}
                        />,
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
                            <h2 className="settings-tab-title">{activeExtTab.label}</h2>
                            <ExtensionSettingsPanel entry={activeExtTab.entry} />
                        </div>
                    )}

                    {panel(
                        'about',
                        <AboutTab
                            appVersion={appVersion}
                            componentInfo={componentInfo}
                            onLogoClick={() => {
                                logoClicks.current += 1;
                                if (logoClicks.current < 5) return;
                                logoClicks.current = 0;
                                safeShowToast(t('gui:settings_modal.name_egg'), 'info', 6000);
                            }}
                        />,
                    )}

                    {/* 사바 스토리지는 설정을 열 때 함께 띄워 목록을 미리 받아 두고, 다른 탭에서는 숨긴다 */}
                    <div
                        className={clsx('settings-tab-content', activeTab === 'sabaStorage' && slideDirection)}
                        hidden={activeTab !== 'sabaStorage'}
                        key="sabaStorage"
                        id="settings-panel-sabaStorage"
                        role="tabpanel"
                        aria-labelledby="settings-tab-sabaStorage"
                        onAnimationEnd={() => setSlideDirection('')}
                    >
                        <h2 className="settings-tab-title">{t(TAB_LABELS.sabaStorage)}</h2>
                        <SabaStorage
                            key={storageTab}
                            devMode={devMode}
                            initialTab={storageTab}
                        />
                    </div>
                    </SaveBarHost>
                </div>
                </div>
                </div>
            </div>
        </div>
    );
}

export default SettingsModal;

