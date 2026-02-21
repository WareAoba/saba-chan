/**
 * DockerToggle — 서버 추가 모달의 Docker 격리 토글
 * 슬롯: AddServer.options
 */
import React from 'react';

const PACKAGE_ICON = (
  <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" strokeWidth="2" style={{ verticalAlign: 'middle' }}>
    <path d="M16.5 9.4l-9-5.19M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
    <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
    <line x1="12" y1="22.08" x2="12" y2="12" />
  </svg>
);

export default function DockerToggle({ options, onOptionsChange, t }) {
  const translate = t || ((key, opts) => opts?.defaultValue || key);
  const useContainer = options?.use_container || false;

  return (
    <div className="as-section as-docker-row" style={{ marginBottom: '12px' }}>
      <div className="as-toggle-row" style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: '12px',
        padding: '12px 14px',
        borderRadius: '8px',
        background: 'var(--bg-surface-tertiary)',
        transition: 'background 0.15s',
      }}>
        <label className="as-toggle-switch" style={{
          position: 'relative',
          display: 'inline-flex',
          flexShrink: 0,
          marginTop: '2px',
        }}>
          <input
            type="checkbox"
            checked={useContainer}
            onChange={(e) => onOptionsChange({ ...options, use_container: e.target.checked })}
            style={{ opacity: 0, width: 0, height: 0, position: 'absolute' }}
          />
          <span className="as-toggle-track" />
        </label>
        <div className="as-toggle-info">
          <span className="as-toggle-title">
            {PACKAGE_ICON}{' '}
            {translate('add_server_modal.docker_isolation', { defaultValue: 'Docker Isolation' })}
          </span>
          <span className="as-toggle-desc">
            {useContainer
              ? translate('add_server_modal.docker_isolation_hint_on', { defaultValue: 'Server will run inside a Docker container for isolation.' })
              : translate('add_server_modal.docker_isolation_hint_off', { defaultValue: 'Server will run natively on the host system.' })}
          </span>
        </div>
      </div>
    </div>
  );
}
