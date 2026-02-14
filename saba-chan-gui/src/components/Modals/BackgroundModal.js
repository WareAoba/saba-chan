import React, { useState, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';

function BackgroundModal({ isOpen, onClose, isClosing, ipcPort }) {
    const { t } = useTranslation('gui');
    const [daemonStatus, setDaemonStatus] = useState('checking');
    const [uptime, setUptime] = useState(t('background_modal.uptime_checking'));
    const [restarting, setRestarting] = useState(false);

    useEffect(() => {
        if (!isOpen) return;

        // 데몬 상태 확인 함수
        const checkDaemonStatus = async () => {
            try {
                if (window.api && window.api.daemonStatus) {
                    const status = await window.api.daemonStatus();
                    setDaemonStatus(status.running ? 'running' : 'stopped');
                    if (status.running) {
                        setUptime(t('background_modal.uptime_connected'));
                    } else {
                        setUptime(t('background_modal.uptime_disconnected'));
                    }
                } else {
                    setDaemonStatus('unknown');
                    setUptime(t('background_modal.uptime_checking'));
                }
            } catch (error) {
                console.error('Failed to check daemon status:', error);
                setDaemonStatus('error');
                setUptime(t('background_modal.uptime_failed'));
            }
        };

        // 초기 상태 확인
        checkDaemonStatus();

        // 모달이 열려 있는 동안 2초마다 상태 확인
        const interval = setInterval(checkDaemonStatus, 2000);

        return () => clearInterval(interval);
    }, [isOpen]);

    if (!isOpen) {
        return null;
    }

    const getStatusClass = () => {
        switch (daemonStatus) {
            case 'running':
                return 'status-online';
            case 'stopped':
            case 'error':
                return 'status-offline';
            default:
                return 'status-checking';
        }
    };

    const getStatusText = () => {
        switch (daemonStatus) {
            case 'running':
                return t('background_modal.status_running');
            case 'stopped':
                return t('background_modal.status_stopped');
            case 'error':
                return t('background_modal.status_error');
            default:
                return t('background_modal.status_checking');
        }
    };

    const handleRestartDaemon = async () => {
        setRestarting(true);
        try {
            const result = await window.api.daemonRestart();
            if (result.success) {
                console.log('Daemon restarted successfully');
            } else {
                console.error('Failed to restart daemon:', result.error);
            }
        } catch (error) {
            console.error('Failed to restart daemon:', error);
        } finally {
            setTimeout(() => setRestarting(false), 3000);
        }
    };

    return (
        <div className={`background-modal-container ${isClosing ? 'closing' : ''}`} onClick={(e) => e.stopPropagation()}>
            <div className="background-modal-header">
                <div className="background-modal-title">
                    <span className={`status-indicator ${getStatusClass()}`}></span>
                    <h2>{t('background_modal.title')}</h2>
                </div>
                <button className="background-modal-close" onClick={onClose}><Icon name="close" size="sm" /></button>
            </div>

            <div className="background-modal-content">
                <div className="background-status-section">
                    <span className="status-label">{t('background_modal.daemon_label')}</span>
                    <span className={`status-value ${daemonStatus === 'running' ? 'status-running' : 'status-stopped'}`}>
                        {getStatusText()}
                    </span>
                </div>

                <div className="background-info-section">
                    <div className="info-row">
                        <span className="info-label"><Icon name="plug" size="sm" /> {t('background_modal.ipc_host_label')}</span>
                        <span className="info-value">{t('background_modal.ipc_host_value')}</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label"><Icon name="hash" size="sm" /> {t('background_modal.ipc_port_label')}</span>
                        <span className="info-value">{ipcPort || 57474}</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label"><Icon name="broadcast" size="sm" /> {t('background_modal.protocol_label')}</span>
                        <span className="info-value">{t('background_modal.protocol_value')}</span>
                    </div>
                    <div className="info-row">
                        <span className="info-label"><Icon name="clock" size="sm" /> {t('background_modal.uptime_label')}</span>
                        <span className="info-value">{uptime}</span>
                    </div>
                </div>

                {(daemonStatus === 'stopped' || daemonStatus === 'error') && (
                    <div className="background-restart-section">
                        <button 
                            className="background-restart-btn"
                            onClick={handleRestartDaemon}
                            disabled={restarting}
                        >
                            {restarting ? t('background_modal.restarting_button') : <><Icon name="refresh" size="sm" /> {t('background_modal.restart_button')}</>}
                        </button>
                    </div>
                )}

                <div className="background-info-box">
                    <h4><Icon name="lightbulb" size="sm" style={{ marginRight: '8px' }} /> {t('background_modal.about_title')}</h4>
                    <p>{t('background_modal.about_description')}</p>
                    <p className="info-note">
                        {t('background_modal.about_note')}
                    </p>
                </div>
            </div>
        </div>
    );
}

export default BackgroundModal;
