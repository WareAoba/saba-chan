import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useUIStore } from '../../stores/useUIStore';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaArt } from '../ui/SabaArt';

/** 상단바가 확인한 데몬 상태가 꺼짐·오류인지 ('checking'은 아직 모름) */
export const daemonDown = (status) => status === 'stopped' || status === 'error';

/** 데몬 다시 시작 버튼. 실패하면 이유를 토스트로 알린다 */
export function DaemonRestartButton({ className = 'btn btn-primary', wake = false }) {
    const { t } = useTranslation('gui');
    const [restarting, setRestarting] = useState(false);
    const restart = async () => {
        setRestarting(true);
        // 상태를 한번 내려 둬야 데몬을 따라가는 것들이 다시 본다 (BackgroundModal과 같다)
        useUIStore.getState().setBackgroundDaemonStatus('checking');
        try {
            const res = await window.api.daemonRestart();
            if (!res?.success) {
                safeShowToast(t('shell.daemon_restart_failed', { error: res?.error || '' }), 'error', 6000);
            }
        } catch (e) {
            safeShowToast(t('shell.daemon_restart_failed', { error: e.message }), 'error', 6000);
        }
        setRestarting(false);
    };
    return (
        <button type="button" className={className} onClick={restart} disabled={restarting}>
            <Icon name={wake ? 'sun' : 'refresh'} size="sm" className={restarting ? 'spin' : undefined} />{' '}
            {wake
                ? restarting
                    ? t('saba.waking')
                    : t('saba.wake')
                : restarting
                  ? t('shell.daemon_restarting')
                  : t('shell.daemon_restart')}
        </button>
    );
}

/** 서버가 하나도 안 보이는데 데몬이 꺼져 있을 때: "서버가 없어요" 대신 */
export function DaemonDownState() {
    const { t } = useTranslation('gui');
    return (
        <div className="page page-empty">
            <div className="empty-state daemon-sleep">
                <SabaArt kind="sleep" size={300} />
                <h2>{t('saba.sleep_title')}</h2>
                <p>{t('saba.sleep_body')}</p>
                <DaemonRestartButton className="btn btn-primary btn-lg" wake />
            </div>
        </div>
    );
}

/** 서버 목록은 있지만 데몬이 꺼져 있을 때: 마지막으로 받은 상태라고 알린다 */
export function DaemonDownNotice() {
    const { t } = useTranslation('gui');
    return (
        <div className="notice tone-warn daemon-down-notice" role="alert">
            <Icon name="alertTriangle" size="sm" />
            <span>{t('shell.daemon_down_stale')}</span>
            <DaemonRestartButton className="btn btn-ghost btn-sm" wake />
        </div>
    );
}
