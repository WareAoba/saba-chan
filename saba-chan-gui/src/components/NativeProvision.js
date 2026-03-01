/**
 * NativeProvision — 네이티브(비-Docker) 프로비저닝 진행 상태 UI
 * SteamCMD / download 방식으로 서버 바이너리 설치 시 진행률 표시
 *
 * Props:
 *   server: 서버 인스턴스
 *   provisionProgress: 프로비저닝 상태 객체 (done, error, message, percent)
 *   onDismiss: 프로비저닝 해제 핸들러
 *   t: i18n 번역 함수
 */
import React from 'react';

const SPINNER_ICON = (
    <svg
        viewBox="0 0 24 24"
        width="14"
        height="14"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        className="spin"
    >
        <path d="M21 12a9 9 0 1 1-6.219-8.56" />
    </svg>
);

const CHECK_ICON = (
    <svg
        viewBox="0 0 24 24"
        width="14"
        height="14"
        fill="none"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinecap="round"
        strokeLinejoin="round"
    >
        <polyline points="20 6 9 17 4 12" />
    </svg>
);

const ALERT_ICON = (
    <svg
        viewBox="0 0 24 24"
        width="14"
        height="14"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
    >
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="8" x2="12" y2="12" />
        <line x1="12" y1="16" x2="12.01" y2="16" />
    </svg>
);

export function NativeProvision({ server, provisionProgress, onDismiss, t }) {
    // Docker 인스턴스는 DockerProvision이 처리하므로 무시
    if (server?.extension_data?.docker_enabled) return null;
    if (!server.provisioning) return null;

    const translate = t || ((key, opts) => opts?.defaultValue || key);

    const isDone = provisionProgress?.done && !provisionProgress?.error;
    const isError = !!provisionProgress?.error;
    const percent = provisionProgress?.percent ?? 0;
    const message = provisionProgress?.message || translate('servers.provisioning', { defaultValue: 'Installing...' });

    // 상태 아이콘
    const statusIcon = isDone ? CHECK_ICON : isError ? ALERT_ICON : SPINNER_ICON;

    return (
        <div className="sc-provision-wrap">
            <div className="native-provision">
                <div className="native-provision-header">
                    <span className={`native-provision-icon ${isDone ? 'done' : isError ? 'error' : 'active'}`}>
                        {statusIcon}
                    </span>
                    <span className="native-provision-title">
                        {isDone
                            ? translate('servers.provision_complete', { defaultValue: 'Installation complete' })
                            : isError
                              ? translate('servers.provision_failed', { defaultValue: 'Installation failed' })
                              : translate('servers.provisioning', { defaultValue: 'Installing server...' })}
                    </span>
                </div>
                {/* 진행 바 */}
                <div className="as-provision-bar">
                    {!isDone && !isError && percent > 0 ? (
                        <div
                            className="as-provision-bar-fill determinate"
                            style={{ width: `${percent}%` }}
                        />
                    ) : (
                        <div
                            className={`as-provision-bar-fill ${isError ? 'error' : isDone ? 'done' : 'indeterminate'}`}
                        />
                    )}
                </div>
                {/* 메시지 + 퍼센트 */}
                {message && (
                    <p className="as-provision-message">
                        {message}
                        {percent > 0 && !isDone && !isError && (
                            <span className="as-provision-pct"> ({percent}%)</span>
                        )}
                    </p>
                )}
                {/* 에러 */}
                {isError && (
                    <div className="as-provision-error-row">
                        <p className="as-provision-error">{provisionProgress.error}</p>
                        <button className="as-provision-dismiss" onClick={onDismiss}>
                            {translate('common.dismiss', { defaultValue: 'Dismiss' })}
                        </button>
                    </div>
                )}
            </div>
        </div>
    );
}
