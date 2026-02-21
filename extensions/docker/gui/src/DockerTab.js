/**
 * DockerTab — 서버 설정 모달의 Docker 탭 (CPU/메모리 제한)
 * 슬롯: ServerSettings.tab
 */
import React from 'react';

const DOCKER_ICON = (
  <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="16" height="16" style={{ verticalAlign: 'middle' }}>
    <path fill="currentColor" d="M507 211.16c-1.42-1.19-14.25-10.94-41.79-10.94a132.55 132.55 0 00-21.61 1.9c-5.22-36.4-35.38-54-36.57-55l-7.36-4.28-4.75 6.9a101.65 101.65 0 00-13.06 30.45c-5 20.7-1.9 40.2 8.55 56.85-12.59 7.14-33 8.8-37.28 9H15.94A15.93 15.93 0 000 262.07a241.25 241.25 0 0014.75 86.83C26.39 379.35 43.72 402 66 415.74 91.22 431.2 132.3 440 178.6 440a344.23 344.23 0 0062.45-5.71 257.44 257.44 0 0081.69-29.73 223.55 223.55 0 0055.57-45.67c26.83-30.21 42.74-64 54.38-94h4.75c29.21 0 47.26-11.66 57.23-21.65a63.31 63.31 0 0015.2-22.36l2.14-6.18z"/>
  </svg>
);

const LIGHTBULB_ICON = (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="currentColor" style={{ verticalAlign: 'middle' }}>
    <path d="M12 2a7 7 0 0 1 7 7c0 2.38-1.19 4.47-3 5.74V17a1 1 0 0 1-1 1H9a1 1 0 0 1-1-1v-2.26C6.19 13.47 5 11.38 5 9a7 7 0 0 1 7-7zM9 21v-1h6v1a1 1 0 0 1-1 1h-4a1 1 0 0 1-1-1z"/>
  </svg>
);

/**
 * DockerTab 내용 + 탭 버튼 렌더링
 * Props:
 *   server: 서버 인스턴스
 *   activeTab / setActiveTab: 현재 설정 탭 상태
 *   settings: 현재 설정 값 (settingsValues)
 *   onSettingsChange: 설정 변경 핸들러 (handleSettingChange)
 *   t: i18n 번역 함수
 */
export default function DockerTab({ server, activeTab, setActiveTab, settings, onSettingsChange, t }) {
  const isDocker = server?.extension_data?.docker_enabled;
  if (!isDocker) return null;

  const translate = t || ((key, opts) => opts?.defaultValue || key);
  const extData = settings?._extension_data || {};
  const cpuLimit = extData.docker_cpu_limit != null ? String(extData.docker_cpu_limit) : '';
  const memLimit = extData.docker_memory_limit || '';

  const updateExtData = (key, value) => {
    onSettingsChange('_extension_data', { ...extData, [key]: value });
  };

  return (
    <>
      {/* 탭 버튼 */}
      <button
        className={`settings-tab ${activeTab === 'docker' ? 'active' : ''}`}
        onClick={() => setActiveTab('docker')}
      >
        {DOCKER_ICON} {translate('server_settings.docker_tab', { defaultValue: 'Docker' })}
      </button>

      {/* 탭 내용 */}
      {activeTab === 'docker' && (
        <div className="settings-form" style={{ position: 'absolute', top: 0, left: 0, right: 0 }}>
          <div className="settings-group">
            <h4 className="settings-group-title">
              {DOCKER_ICON} {translate('server_settings.docker_resources_title', { defaultValue: 'Resource Limits' })}
            </h4>
            <p className="protocol-mode-description">
              {translate('server_settings.docker_resources_desc', { defaultValue: 'Configure CPU and memory limits for this Docker container. Changes will regenerate docker-compose.yml.' })}
            </p>

            {/* CPU Limit */}
            <div className="settings-field">
              <label>{translate('server_settings.docker_cpu_limit_label', { defaultValue: 'CPU Limit (cores)' })}</label>
              <input
                type="number"
                min="0.25"
                max="128"
                step="0.25"
                value={cpuLimit}
                onChange={(e) => updateExtData('docker_cpu_limit', e.target.value ? Number(e.target.value) : null)}
                placeholder={translate('server_settings.docker_cpu_limit_placeholder', { defaultValue: 'e.g., 2.0 (no limit if empty)' })}
              />
              <small className="field-description">
                {translate('server_settings.docker_cpu_limit_desc', { defaultValue: 'Number of CPU cores to allocate. Leave empty for no limit.' })}
              </small>
            </div>

            {/* Memory Limit */}
            <div className="settings-field">
              <label>{translate('server_settings.docker_memory_limit_label', { defaultValue: 'Memory Limit' })}</label>
              <input
                type="text"
                value={memLimit}
                onChange={(e) => updateExtData('docker_memory_limit', e.target.value || null)}
                placeholder={translate('server_settings.docker_memory_limit_placeholder', { defaultValue: 'e.g., 4g, 512m (no limit if empty)' })}
              />
              <small className="field-description">
                {translate('server_settings.docker_memory_limit_desc', { defaultValue: 'Memory limit with unit (e.g., 512m, 2g, 4g). Leave empty for no limit.' })}
              </small>
            </div>
          </div>

          {/* Info box */}
          <div className="protocol-mode-section protocol-mode-info" style={{ marginTop: '16px' }}>
            <p className="protocol-mode-hint">
              <span className="hint-icon">{LIGHTBULB_ICON}</span>
              {translate('server_settings.docker_restart_hint', { defaultValue: 'Resource limit changes take effect after restarting the server.' })}
            </p>
          </div>
        </div>
      )}
    </>
  );
}
