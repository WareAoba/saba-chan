import React from 'react';
import { useTranslation } from 'react-i18next';
import './TitleBar.css';

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
                <span>🎮 {t('common:app_name')}</span>
            </div>
            <div className="title-bar-controls">
                <button 
                    className="title-bar-btn minimize-btn"
                    onClick={handleMinimize}
                    title={t('title_bar.minimize')}
                >
                    −
                </button>
                <button 
                    className="title-bar-btn maximize-btn"
                    onClick={handleMaximize}
                    title={t('title_bar.maximize')}
                >
                    ▢
                </button>
                <button 
                    className="title-bar-btn close-btn"
                    onClick={handleClose}
                    title={t('title_bar.close')}
                >
                    ✕
                </button>
            </div>
        </div>
    );
}

export default TitleBar;
