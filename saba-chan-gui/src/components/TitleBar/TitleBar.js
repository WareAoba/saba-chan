import React from 'react';
import { useTranslation } from 'react-i18next';
import './TitleBar.css';
import { Icon } from '../Icon';

function TitleBar() {
    const { t } = useTranslation('gui');
    
    const handleMinimize = () => {
        window.electron.minimizeWindow();
    };

    const handleMaximize = () => {
        window.electron.maximizeWindow();
    };

    const handleClose = () => {
        window.electron.closeWindow();
    };

    return (
        <div className="title-bar">
            <div className="title-bar-text">
                <img src="/favicon.png" alt="" className="title-bar-icon" />
                <span>{t('common:app_name')}</span>
            </div>
            <div className="title-bar-controls">
                <button 
                    className="title-bar-btn minimize-btn"
                    onClick={handleMinimize}
                    title={t('title_bar.minimize')}
                >
                    <Icon name="minimize" size={12} />
                </button>
                <button 
                    className="title-bar-btn maximize-btn"
                    onClick={handleMaximize}
                    title={t('title_bar.maximize')}
                >
                    <Icon name="maximize" size={12} />
                </button>
                <button 
                    className="title-bar-btn close-btn"
                    onClick={handleClose}
                    title={t('title_bar.close')}
                >
                    <Icon name="close" size={12} />
                </button>
            </div>
        </div>
    );
}

export default TitleBar;
