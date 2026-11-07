/** 설정 탭 (사용자 요청 2026-11-06: 고급에 몰린 설정을 갈래별로 나눈다). 순서가 화면 순서다 */
export const SETTINGS_TABS = [
    'general',
    'notify',
    'appearance',
    'console',
    'servers',
    'connection',
    'storage',
    'sabaStorage',
    'security',
    'troubleshoot',
    'about',
] as const;
export type SettingsTab = (typeof SETTINGS_TABS)[number];
export const SETTINGS_TAB_LABELS: Record<SettingsTab, string> = {
    general: 'gui:settings_modal.general',
    notify: 'gui:settings_modal.tab_notify',
    appearance: 'gui:settings_modal.appearance',
    console: 'gui:settings_modal.tab_console',
    servers: 'gui:settings_modal.tab_servers',
    connection: 'gui:settings_modal.tab_connection',
    storage: 'gui:settings_modal.tab_storage',
    sabaStorage: 'gui:saba_storage.title',
    security: 'gui:settings_modal.tab_security',
    troubleshoot: 'gui:settings_modal.tab_troubleshoot',
    about: 'gui:settings_modal.about_title',
};
