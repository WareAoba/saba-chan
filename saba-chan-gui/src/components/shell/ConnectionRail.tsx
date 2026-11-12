import clsx from 'clsx';
import { useTranslation } from 'react-i18next';

// 붙는 중은 떠나온 상태(꺼짐)의 회색으로 깜빡이고, 붙어 있다가 끊겨 다시 붙는 중만 노랑 (사용자 11-11)
const DAEMON_TONE: Record<string, string> = { running: 'ok', checking: 'idle', stopped: 'stop', error: 'stop', token_invalid: 'stop' };
const BOT_TONE: Record<string, string> = { running: 'ok', connecting: 'idle', stopped: 'idle' };

/**
 * 연결 경로 표시: 내 PC의 데몬 ── 디스코드 봇(사바쨩 클라우드 경유).
 * 봇은 데몬을 거쳐서만 붙으므로, 데몬이 꺼지면 봇 구간은 숨기고 데몬만 보여 준다.
 * 어느 구간이 끊겼는지가 곧 사용자가 할 일이다 (gui-design-reference L).
 */
interface ConnectionRailProps {
    daemonStatus?: string;
    botStatus?: string;
    /** 실행 중이던 봇이 릴레이와 끊겨 다시 붙는 중 (노랑) */
    botReconnecting?: boolean;
    onDaemonClick: () => void;
    onBotClick: () => void;
    daemonExpanded?: boolean;
}

export function ConnectionRail({ daemonStatus, botStatus, botReconnecting, onDaemonClick, onBotClick, daemonExpanded }: ConnectionRailProps) {
    const { t } = useTranslation('gui');
    const daemonKey = daemonStatus === 'error' ? 'stopped' : daemonStatus || 'checking';
    const daemonTone = DAEMON_TONE[daemonKey] || 'warn';
    const daemonUp = daemonKey === 'running';
    const botKey = botStatus && BOT_TONE[botStatus] ? botStatus : 'stopped';
    const botTone = botKey === 'connecting' && botReconnecting ? 'warn' : BOT_TONE[botKey];

    return (
        <div className={clsx('conn-rail', `daemon-${daemonKey}`)} role="group" aria-label={t('shell.conn_label')}>
            <button
                type="button"
                className={clsx('conn-node', `tone-${daemonTone}`, { pulse: daemonKey === 'checking' })}
                onClick={onDaemonClick}
                aria-expanded={daemonExpanded}
                data-tooltip={t(`shell.daemon_tip_${daemonKey}`)}
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
                        data-tooltip={t(`shell.bot_tip_${botKey}`)}
                    >
                        <span className="conn-dot" aria-hidden="true" />
                        <span>{t(`shell.bot_${botKey}`)}</span>
                    </button>
                </>
            )}
        </div>
    );
}
