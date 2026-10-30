import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';

import { useSabaBubble } from '../../stores/useSabaBubble';
import { daemonStateOf } from '../servers/DaemonDown';
import { DEFAULT_IPC_PORT } from '../../stores/useSettingsStore';
import { useUIStore } from '../../stores/useUIStore';
import { openWithNotice, safeShowToast } from '../../utils/helpers';

function BackgroundModal({ isOpen, onClose, isClosing, ipcPort }) {
    const { t } = useTranslation('gui');
    const [daemonStatus, setDaemonStatus] = useState('checking');
    const [uptime, setUptime] = useState(t('background_modal.uptime_checking'));
    const [restarting, setRestarting] = useState(false);
    // 원격 데몬을 고른 동안 이 PC의 서비스가 중계하는 데몬 ("이름 · 주소"), 이 PC면 null
    const [remote, setRemote] = useState(null);

    useEffect(() => {
        if (!isOpen || typeof window.api?.daemonTarget !== 'function') return;
        let alive = true;
        Promise.all([window.api.daemonTarget(), window.api.remoteDaemons?.()])
            .then(([current, list]) => {
                const d = current?.target && list?.daemons?.find((x) => x.hostId === current.target);
                if (!alive) return;
                if (!current?.target) return setRemote(null);
                const where = d?.address ? `${d.address}:${d.port}` : t('remote_daemon.via_relay');
                setRemote(`${d?.label || d?.name || current.target} · ${where}`);
            })
            .catch(() => alive && setRemote(null));
        return () => {
            alive = false;
        };
    }, [isOpen, t]);

    useEffect(() => {
        if (!isOpen) return;

        // 데몬 상태 확인 함수
        const checkDaemonStatus = async () => {
            try {
                if (window.api && window.api.daemonStatus) {
                    const status = await window.api.daemonStatus();
                    setDaemonStatus(daemonStateOf(status));
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
    }, [isOpen, t]);

    if (!isOpen) {
        return null;
    }

    const handleRestartDaemon = async () => {
        setRestarting(true);
        // 재시작은 5초 폴링 사이에 끝날 수 있다. 상태를 한번 내려 둬야 데몬을 따라가는 것들(실행 환경 준비 표시 등)이 다시 본다
        useUIStore.getState().setBackgroundDaemonStatus('checking');
        useSabaBubble.getState().markWoken();
        try {
            const result = await window.api.daemonRestart();
            if (result?.success) {
                console.log('Daemon restarted successfully');
            } else {
                useSabaBubble.getState().markWoken(false);
                console.error('Failed to restart daemon:', result?.error);
                // 실패는 화면에도 알린다 (DaemonDown과 같은 문구, G-U7)
                safeShowToast(t('shell.daemon_restart_failed', { error: result?.error || '' }), 'error', 6000);
            }
        } catch (error) {
            useSabaBubble.getState().markWoken(false);
            console.error('Failed to restart daemon:', error);
            safeShowToast(t('shell.daemon_restart_failed', { error: error.message }), 'error', 6000);
        } finally {
            setTimeout(() => setRestarting(false), 3000);
        }
    };

    const tone =
        daemonStatus === 'running'
            ? 'ok'
            : daemonStatus === 'stopped' || daemonStatus === 'error' || daemonStatus === 'token_invalid'
              ? 'stop'
              : 'warn';
    const port = ipcPort || DEFAULT_IPC_PORT;
    const address = `${t('background_modal.ipc_host_value')}:${port}`;

    return (
        <div
            className={clsx('background-modal-container', 'bg-pop', { closing: isClosing })}
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-label={t('background_modal.title')}
        >
            {/* 1. 지금 상태 한 줄 + 이게 뭔지 한 줄 */}
            <div className={clsx('bg-pop-head', `tone-${tone}`)}>
                <span className={clsx('bg-pop-orb', { pulse: tone === 'warn' })} aria-hidden="true">
                    <Icon name="server" size="sm" />
                </span>
                <div className="bg-pop-title">
                    <strong>
                        {t(
                            `shell.daemon_${daemonStatus === 'error' ? 'stopped' : ['running', 'stopped', 'token_invalid'].includes(daemonStatus) ? daemonStatus : 'checking'}`,
                        )}
                    </strong>
                    <span>{t('shell.daemon_about')}</span>
                </div>
                <button type="button" className="icon-btn" onClick={onClose} aria-label={t('shell.dismiss')}>
                    <Icon name="close" size="sm" />
                </button>
            </div>

            {/* 2. 꺼져 있으면 할 일이 먼저. 토큰이 어긋나면 다시 시작으로 풀리지 않으니 이유만 알린다 */}
            {daemonStatus === 'token_invalid' && <p className="bg-pop-note">{t('shell.daemon_tip_token_invalid')}</p>}
            {(daemonStatus === 'stopped' || daemonStatus === 'error') && (
                <button
                    type="button"
                    className="btn btn-primary btn-block bg-pop-cta"
                    onClick={handleRestartDaemon}
                    disabled={restarting}
                >
                    <Icon name="refresh" size="sm" />
                    {restarting ? t('background_modal.restarting_button') : t('background_modal.restart_button')}
                </button>
            )}

            {/* 3. 연결 정보 (개발·문제 해결용) */}
            <dl className="bg-pop-facts">
                {remote && (
                    <div>
                        <dt>{t('shell.daemon_target')}</dt>
                        <dd>{remote}</dd>
                    </div>
                )}
                <div>
                    <dt>{t('shell.daemon_address')}</dt>
                    <dd className="mono">
                        {address}
                        <button
                            type="button"
                            className="icon-btn bg-pop-copy"
                            onClick={() => navigator.clipboard?.writeText(`http://${address}`).catch(() => {})}
                            data-tooltip={t('shell.copy')}
                            aria-label={t('shell.copy')}
                        >
                            <Icon name="copy" size={14} />
                        </button>
                    </dd>
                </div>
                <div>
                    <dt>{t('shell.daemon_protocol')}</dt>
                    <dd className="mono">{t('background_modal.protocol_value')}</dd>
                </div>
                <div>
                    <dt>{t('shell.daemon_link')}</dt>
                    <dd>{uptime}</dd>
                </div>
            </dl>

            {/* 4. 보조 동작 */}
            {window.api?.logsOpenFolder && (
                <button
                    type="button"
                    className="btn btn-ghost btn-block"
                    onClick={() => openWithNotice(() => window.api.logsOpenFolder())}
                >
                    <Icon name="folder" size="sm" /> {t('shell.open_logs_folder')}
                </button>
            )}

            <p className="bg-pop-note">{t('background_modal.about_note')}</p>
        </div>
    );
}

export default BackgroundModal;
