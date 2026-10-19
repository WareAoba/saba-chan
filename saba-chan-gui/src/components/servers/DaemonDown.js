import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSabaBubble } from '../../stores/useSabaBubble';
import { useUIStore } from '../../stores/useUIStore';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaArt } from '../ui/SabaArt';

/** 상단바가 확인한 데몬 상태가 꺼짐·오류인지 ('checking'은 아직 모름). 토큰이 어긋나도 API를 못 쓰니 꺼진 것과 같다 */
export const daemonDown = (status) => status === 'stopped' || status === 'error' || status === 'token_invalid';

/** `daemonStatus()` 답 → 상태. 데몬이 떠 있어도 이 앱의 토큰을 받지 않으면 다시 시작으로 풀리지 않아 따로 둔다 */
export const daemonStateOf = (status) =>
    status?.running ? 'running' : status?.error_code === 'token_invalid' ? 'token_invalid' : 'stopped';

/** 데몬 다시 시작 버튼. 실패하면 이유를 토스트로 알린다 */
export function DaemonRestartButton({ className = 'btn btn-primary', wake = false }) {
    const { t } = useTranslation('gui');
    const [restarting, setRestarting] = useState(false);
    const restart = async () => {
        setRestarting(true);
        // 상태를 한번 내려 둬야 데몬을 따라가는 것들이 다시 본다 (BackgroundModal과 같다)
        useUIStore.getState().setBackgroundDaemonStatus('checking');
        useSabaBubble.getState().markWoken();
        try {
            const res = await window.api.daemonRestart();
            if (!res?.success) {
                useSabaBubble.getState().markWoken(false);
                safeShowToast(t('shell.daemon_restart_failed', { error: res?.error || '' }), 'error', 6000);
            }
        } catch (e) {
            useSabaBubble.getState().markWoken(false);
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
    const tokenInvalid = useUIStore((s) => s.backgroundDaemonStatus === 'token_invalid');
    if (tokenInvalid) {
        return (
            <div className="page page-empty">
                <div className="empty-state">
                    <h2>{t('shell.daemon_token_invalid')}</h2>
                    <p>{t('shell.daemon_tip_token_invalid')}</p>
                </div>
            </div>
        );
    }
    return (
        <div className="page page-empty">
            <div className="empty-state">
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
    const tokenInvalid = useUIStore((s) => s.backgroundDaemonStatus === 'token_invalid');
    return (
        <div className="notice tone-warn daemon-down-notice" role="alert">
            <Icon name="alertTriangle" size="sm" />
            <span>{t(tokenInvalid ? 'shell.daemon_tip_token_invalid' : 'shell.daemon_down_stale')}</span>
            {!tokenInvalid && <DaemonRestartButton className="btn btn-ghost btn-sm" wake />}
        </div>
    );
}
