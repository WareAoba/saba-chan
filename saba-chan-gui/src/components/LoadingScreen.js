
import { useTranslation } from 'react-i18next';
import { Icon, TitleBar } from './index';

/**
 * LoadingScreen — displayed while daemon is initializing.
 */
export function LoadingScreen({ logoSrc, initStatus, initProgress }) {
    const { t } = useTranslation('gui');

    return (
        <div className="loading-screen">
            <TitleBar />
            <div className="loading-content">
                <div className="loading-logo-container">
                    <i className="glow-blur"></i>
                    <i className="glow-ring"></i>
                    <i className="glow-mask"></i>
                    <img src="./title.png" alt="" className="loading-logo-img" />
                </div>
                <img src={logoSrc} alt={t('common:app_name')} className="loading-logo-text" />
                <div className="loading-status">
                    <Icon name="loader" size="sm" /> {initStatus}
                </div>
                <div className="loading-progress-bar">
                    <div className="loading-progress-fill" style={{ width: `${initProgress}%` }}></div>
                </div>
                <div className="loading-tips">
                    <Icon name="info" size="sm" /> {t('buttons.loading_tips')}
                </div>
            </div>
        </div>
    );
}
