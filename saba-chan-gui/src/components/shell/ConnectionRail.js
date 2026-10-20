import clsx from 'clsx';
import { useTranslation } from 'react-i18next';

const DAEMON_TONE = { running: 'ok', checking: 'warn', stopped: 'stop', error: 'stop', token_invalid: 'stop' };
const BOT_TONE = { running: 'ok', connecting: 'warn', stopped: 'idle' };

/**
 * 연결 경로 표시: 내 PC의 데몬 ── 디스코드 봇(사바쨩 클라우드 경유).
 * 봇은 데몬을 거쳐서만 붙으므로, 데몬이 꺼지면 봇 구간은 숨기고 데몬만 보여 준다.
 * 어느 구간이 끊겼는지가 곧 사용자가 할 일이다 (gui-design-reference L).
 */
export function ConnectionRail({ daemonStatus, botStatus, onDaemonClick, onBotClick, daemonExpanded }) {
    const { t } = useTranslation('gui');
    const daemonKey = daemonStatus === 'error' ? 'stopped' : daemonStatus || 'checking';
    const daemonTone = DAEMON_TONE[daemonKey] || 'warn';
    const daemonUp = daemonKey === 'running';
    const botKey = BOT_TONE[botStatus] ? botStatus : 'stopped';
    const botTone = BOT_TONE[botKey];

    return (
        <div className={clsx('conn-rail', `daemon-${daemonKey}`)} role="group" aria-label={t('shell.conn_label')}>
            <button
                type="button"
                className={clsx('conn-node', `tone-${daemonTone}`, { pulse: daemonKey === 'checking' })}
                onClick={onDaemonClick}
                aria-expanded={daemonExpanded}
                title={t(`shell.daemon_tip_${daemonKey}`)}
            >
                <span className="conn-dot" aria-hidden="true" />
                <span>{daemonUp ? t('shell.daemon_short') : t(`shell.daemon_${daemonKey}`)}</span>
            </button>
            {daemonUp && (
                <>
                    <span className={clsx('conn-link', `tone-${botTone}`, `link-${botKey}`)} aria-hidden="true" />
                    <button
                        type="button"
                        className={clsx('conn-node', `tone-${botTone}`, { pulse: botKey === 'connecting' })}
                        onClick={onBotClick}
                        title={t(`shell.bot_tip_${botKey}`)}
                    >
                        <span className="conn-dot" aria-hidden="true" />
                        <span>{t(`shell.bot_${botKey}`)}</span>
                    </button>
                </>
            )}
        </div>
    );
}
