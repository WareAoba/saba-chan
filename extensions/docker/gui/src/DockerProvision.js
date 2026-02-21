/**
 * DockerProvision — 서버 카드의 Docker 프로비저닝 진행 상태 UI
 * 슬롯: ServerCard.provision
 *
 * Props:
 *   server: 서버 인스턴스
 *   provisionProgress: 프로비저닝 상태 객체 (steps, step, done, error, message, percent)
 *   onDismiss: 프로비저닝 해제 핸들러
 *   t: i18n 번역 함수
 */
import React from 'react';

const CHECK_ICON = (
  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

const SPINNER_ICON = (
  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="spin">
    <path d="M21 12a9 9 0 1 1-6.219-8.56" />
  </svg>
);

const ALERT_ICON = (
  <svg viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
    <circle cx="12" cy="12" r="10" />
    <line x1="12" y1="8" x2="12" y2="12" />
    <line x1="12" y1="16" x2="12.01" y2="16" />
  </svg>
);

/** Docker 프로비저닝의 기본 단계 (API 응답 전 fallback) */
const DEFAULT_STEPS = ['docker_engine', 'steamcmd', 'compose'];

export default function DockerProvision({ server, provisionProgress, onDismiss, t }) {
  const isDocker = server?.extension_data?.docker_enabled;
  if (!isDocker) return null;
  if (!server.provisioning) return null;

  const translate = t || ((key, opts) => opts?.defaultValue || key);
  const steps = provisionProgress?.steps || DEFAULT_STEPS;

  return (
    <div className="sc-provision-wrap">
      <div className="as-provision-steps">
        {steps.map((stepLabel, idx) => {
          const currentStep = provisionProgress?.step ?? -1;
          const isDone = provisionProgress?.done && !provisionProgress?.error;
          let stepClass = 'pending';
          if (isDone || idx < currentStep) stepClass = 'completed';
          else if (idx === currentStep) stepClass = provisionProgress?.error ? 'error' : 'active';

          const label = translate(`add_server_modal.step_${stepLabel}`, { defaultValue: stepLabel });
          return (
            <div key={stepLabel} className={`as-step ${stepClass}`}>
              <div className="as-step-icon">
                {stepClass === 'completed' ? CHECK_ICON :
                 stepClass === 'active' ? SPINNER_ICON :
                 stepClass === 'error' ? ALERT_ICON :
                 <span className="as-step-num">{idx + 1}</span>}
              </div>
              <span className="as-step-label">{label}</span>
            </div>
          );
        })}
      </div>
      <div className="as-provision-bar">
        {provisionProgress?.percent != null && !provisionProgress?.done && !provisionProgress?.error ? (
          <div className="as-provision-bar-fill determinate" style={{ width: `${provisionProgress.percent}%` }} />
        ) : (
          <div className={`as-provision-bar-fill ${provisionProgress?.error ? 'error' : provisionProgress?.done ? 'done' : 'indeterminate'}`} />
        )}
      </div>
      {provisionProgress?.message && (
        <p className="as-provision-message">
          {provisionProgress.message}
          {provisionProgress?.percent != null && !provisionProgress?.done && !provisionProgress?.error && (
            <span className="as-provision-pct"> ({provisionProgress.percent}%)</span>
          )}
        </p>
      )}
      {provisionProgress?.error && (
        <div className="as-provision-error-row">
          <p className="as-provision-error">{provisionProgress.error}</p>
          <button className="as-provision-dismiss" onClick={onDismiss}>
            {translate('common.dismiss', { defaultValue: 'Dismiss' })}
          </button>
        </div>
      )}
    </div>
  );
}
