import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { WindowControls } from './shell/AppBar';

/**
 * LoadingScreen — displayed while daemon is initializing.
 * failed가 있으면 데몬을 못 띄운 것: 진행 막대 대신 이유와 [다시 시도]·[그대로 열기]를 보인다.
 */
export function LoadingScreen({ logoSrc, initStatus, initProgress, failed, onRetry, onContinue }) {
    const { t } = useTranslation('gui');
    const [retrying, setRetrying] = useState(false);

    return (
        <div className="loading-screen">
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
                <img src={logoSrc} alt={t('common:app_name')} className="loading-logo-text" />
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
                    <>
                        <div className="loading-status">
                            <Icon name="loader" size="sm" /> {initStatus || t('loading.step_init')}
                        </div>
                        <div className="loading-progress-bar">
                            <div className="loading-progress-fill" style={{ width: `${initProgress}%` }}></div>
                        </div>
                    </>
                )}
                <div className="loading-tips">
                    <Icon name="info" size="sm" /> {t('buttons.loading_tips')}
                </div>
            </div>
        </div>
    );
}
