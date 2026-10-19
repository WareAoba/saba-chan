import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { WindowControls } from './shell/AppBar';
import { MatrixRain } from './shell/MatrixRain';

/**
 * LoadingScreen — displayed while daemon is initializing.
 * failed가 있으면 데몬을 못 띄운 것: 진행 막대 대신 이유와 [다시 시도]·[그대로 열기]를 보인다.
 */
export function LoadingScreen({ logoSrc, initStatus, initProgress, failed, onRetry, onContinue }) {
    const { t } = useTranslation('gui');
    const [retrying, setRetrying] = useState(false);

    return (
        <div className="loading-screen">
            {/* 뒤에서 흐르는 서버 콘솔 (가운데는 비워 둔다) */}
            <MatrixRain className="loading-matrix" />
            <div className="loading-vignette" aria-hidden="true" />
            <div className="appbar appbar-bare">
                <div className="appbar-spacer" />
                <WindowControls />
            </div>
            <div className="loading-content">
                <div className="loading-logo-container">
                    <i className="glow-blur"></i>
                    <i className="glow-ring"></i>
                    <i className="glow-mask"></i>
                    <img src="./title.png" alt="" className="loading-logo-img" />
                </div>
                {/* 전용 글자 로고: 그림을 마스크로 써서 테마 글자색으로 칠한다 */}
                <span
                    className="loading-logo-text"
                    role="img"
                    aria-label={t('common:app_name')}
                    style={{ '--wordmark': `url("${logoSrc}")` }}
                />
                {failed ? (
                    <div className="loading-failed" role="alert">
                        <p className="loading-failed-message">
                            <Icon name="alertTriangle" size="sm" /> {failed}
                        </p>
                        <p className="loading-failed-hint">{t('loading.failed_hint')}</p>
                        <div className="loading-failed-actions">
                            <button
                                type="button"
                                className="btn btn-primary"
                                disabled={retrying}
                                onClick={async () => {
                                    setRetrying(true);
                                    try {
                                        await onRetry?.();
                                    } finally {
                                        setRetrying(false);
                                    }
                                }}
                            >
                                <Icon name="refresh" size="sm" className={retrying ? 'spin' : undefined} />{' '}
                                {t('loading.retry')}
                            </button>
                            <button type="button" className="btn btn-ghost" disabled={retrying} onClick={onContinue}>
                                {t('loading.continue')}
                            </button>
                        </div>
                    </div>
                ) : (
                    <div className="loading-term">
                        {/* 콘솔 한 줄처럼: 프롬프트 + 지금 하는 일 + 깜빡이는 커서, 아래에 진행 막대 */}
                        <div className="loading-status" aria-live="polite">
                            <span className="loading-prompt" aria-hidden="true">
                                saba@core:~$
                            </span>
                            <span className="loading-status-text">{initStatus || t('loading.step_init')}</span>
                            <span className="loading-caret" aria-hidden="true" />
                        </div>
                        <div className="loading-progress-row">
                            <div className="loading-progress-bar">
                                <div className="loading-progress-fill" style={{ width: `${initProgress}%` }}></div>
                            </div>
                            <span className="loading-progress-pct">{Math.round(initProgress || 0)}%</span>
                        </div>
                    </div>
                )}
                <div className="loading-tips">
                    <Icon name="info" size="sm" /> {t('buttons.loading_tips')}
                </div>
            </div>
        </div>
    );
}
