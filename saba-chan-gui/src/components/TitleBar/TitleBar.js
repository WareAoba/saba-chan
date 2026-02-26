import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './TitleBar.css';
import { getEffectiveTheme } from '../../utils/themeManager';
import { Icon } from '../Icon';

function TitleBar() {
    const { t } = useTranslation('gui');
    const [effectiveTheme, setEffectiveTheme] = useState(getEffectiveTheme());

    useEffect(() => {
        // Listen for theme changes (data-theme attribute)
        const observer = new MutationObserver(() => {
            setEffectiveTheme(getEffectiveTheme());
        });
        observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });

        // Listen for system preference changes
        const mql = window.matchMedia('(prefers-color-scheme: dark)');
        const handleChange = () => setEffectiveTheme(getEffectiveTheme());
        mql.addEventListener('change', handleChange);

        return () => {
            observer.disconnect();
            mql.removeEventListener('change', handleChange);
        };
    }, []);

    const faviconSrc = effectiveTheme === 'dark' ? './favicon-dark.png' : './favicon-light.png';

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
                <img src={faviconSrc} alt="" className="title-bar-icon" />
                <span>{t('common:app_name')}</span>
            </div>
            <div className="title-bar-controls">
                <button className="title-bar-btn minimize-btn" onClick={handleMinimize} title={t('title_bar.minimize')}>
                    <Icon name="minimize" size={12} />
                </button>
                <button className="title-bar-btn maximize-btn" onClick={handleMaximize} title={t('title_bar.maximize')}>
                    <Icon name="maximize" size={12} />
                </button>
                <button className="title-bar-btn close-btn" onClick={handleClose} title={t('title_bar.close')}>
                    <Icon name="close" size={12} />
                </button>
            </div>
        </div>
    );
}

export default TitleBar;
