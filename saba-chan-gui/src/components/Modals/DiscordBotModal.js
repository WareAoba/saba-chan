import React from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';

function DiscordBotModal({ 
    isOpen, 
    onClose, 
    discordBotStatus,
    discordToken,
    setDiscordToken,
    discordPrefix,
    setDiscordPrefix,
    discordAutoStart,
    setDiscordAutoStart,
    handleStartDiscordBot,
    handleStopDiscordBot,
    saveCurrentSettings
}) {
    const { t } = useTranslation('gui');
    if (!isOpen) {
        return null;
    }

    return (
        <div className="discord-modal-container" onClick={(e) => e.stopPropagation()}>
            <div className="discord-modal-header">
                    <div className="discord-modal-title">
                        <span className={`status-indicator ${discordBotStatus === 'running' ? 'status-online' : 'status-offline'}`}></span>
                        <h2>{t('discord_modal.title')}</h2>
                    </div>
                    <button className="discord-modal-close" onClick={onClose}>✕</button>
                </div>

                <div className="discord-modal-content">
                    <div className="discord-status-section">
                        <span className="status-label">{t('discord_modal.status_label')}</span>
                        <span className={`status-value status-${discordBotStatus}`}>
                            {discordBotStatus === 'running' ? t('discord_modal.status_online') : discordBotStatus === 'error' ? t('discord_modal.status_error') : t('discord_modal.status_offline')}
                        </span>
                    </div>

                    <div className="discord-config-section">
                        <div className="discord-form-group">
                            <label>{t('discord_modal.token_label')}</label>
                            <input
                                type="password"
                                placeholder={t('discord_modal.token_placeholder')}
                                value={discordToken}
                                onChange={(e) => setDiscordToken(e.target.value)}
                                className="discord-input"
                            />
                        </div>

                        <div className="discord-form-group">
                            <label>{t('discord_modal.prefix_label')}</label>
                            <input
                                type="text"
                                placeholder={t('discord_modal.prefix_placeholder')}
                                value={discordPrefix}
                                onChange={(e) => setDiscordPrefix(e.target.value)}
                                className="discord-input"
                            />
                            <small>{t('discord_modal.prefix_description')}</small>
                            {!discordPrefix && <small className="warning-text">{t('discord_modal.prefix_warning')}</small>}
                        </div>

                        <div className="discord-form-group">
                            <label className="discord-checkbox-label">
                                <input
                                    type="checkbox"
                                    checked={discordAutoStart}
                                    onChange={(e) => setDiscordAutoStart(e.target.checked)}
                                />
                                {t('discord_modal.auto_start_label')}
                            </label>
                        </div>
                    </div>

                    <div className="discord-info-box">
                        <h4>💡 {t('discord_modal.usage_title')}</h4>
                        <p>{t('discord_modal.usage_instruction')}</p>
                        <code>{discordPrefix || '!saba'} [module] [command]</code>
                        <p className="info-note">
                            {t('discord_modal.usage_note')}
                        </p>
                    </div>
                </div>

                <div className="discord-modal-footer">
                    <button
                        className={`discord-btn ${
                            discordBotStatus === 'running' 
                                ? 'discord-btn-stop' 
                                : 'discord-btn-start'
                        }`}
                        onClick={() => {
                            if (discordBotStatus === 'running') {
                                handleStopDiscordBot();
                            } else {
                                handleStartDiscordBot();
                            }
                        }}
                    >
                        {discordBotStatus === 'running' ? t('discord_modal.stop_button') : t('discord_modal.start_button')}
                    </button>
                    <button
                        className="discord-btn discord-btn-save"
                        onClick={saveCurrentSettings}
                    >
                        {t('discord_modal.save_button')}
                    </button>
                </div>
            </div>
    );
}

export default DiscordBotModal;
