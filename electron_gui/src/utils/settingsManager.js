/**
 * 설정 관리 통합 유틸리티
 * GUI 설정과 Bot 설정을 중앙에서 관리
 */

/**
 * 모든 설정 로드 (GUI + Bot)
 * @param {object} api - window.api 객체
 * @returns {Promise<object>} { guiSettings, botConfig, settingsPath }
 */
export async function loadAllSettings(api) {
    try {
        // GUI 설정 로드
        const guiSettings = await api.settingsLoad() || {
            autoRefresh: true,
            refreshInterval: 2000,
            modulesPath: '',
            discordToken: '',
            discordAutoStart: false
        };
        
        // Bot 설정 로드
        const botConfig = await api.botConfigLoad() || {
            prefix: '!saba',
            moduleAliases: {},
            commandAliases: {}
        };
        
        // 설정 파일 경로
        const settingsPath = await api.settingsGetPath();
        
        console.log('[Settings] All settings loaded successfully');
        
        return {
            guiSettings,
            botConfig,
            settingsPath
        };
    } catch (error) {
        console.error('[Settings] Failed to load settings:', error);
        throw error;
    }
}

/**
 * GUI 설정 저장
 * @param {object} api - window.api 객체
 * @param {object} settings - 저장할 설정
 * @returns {Promise<boolean>} 성공 여부
 */
export async function saveGuiSettings(api, settings) {
    try {
        await api.settingsSave(settings);
        console.log('[Settings] GUI settings saved');
        return true;
    } catch (error) {
        console.error('[Settings] Failed to save GUI settings:', error);
        return false;
    }
}

/**
 * Bot 설정 저장
 * @param {object} api - window.api 객체
 * @param {object} botConfig - 저장할 Bot 설정
 * @returns {Promise<boolean>} 성공 여부
 */
export async function saveBotConfig(api, botConfig) {
    try {
        const result = await api.botConfigSave(botConfig);
        if (result.error) {
            console.error('[Settings] Failed to save bot config:', result.error);
            return false;
        }
        console.log('[Settings] Bot config saved');
        return true;
    } catch (error) {
        console.error('[Settings] Failed to save bot config:', error);
        return false;
    }
}

/**
 * 설정 값이 실제로 변경되었는지 확인
 * @param {object} oldSettings - 이전 설정
 * @param {object} newSettings - 새 설정
 * @returns {boolean} 변경 여부
 */
export function hasSettingsChanged(oldSettings, newSettings) {
    if (!oldSettings || !newSettings) return true;
    
    const oldKeys = Object.keys(oldSettings).sort();
    const newKeys = Object.keys(newSettings).sort();
    
    if (oldKeys.length !== newKeys.length) return true;
    
    for (const key of oldKeys) {
        if (oldSettings[key] !== newSettings[key]) {
            return true;
        }
    }
    
    return false;
}
