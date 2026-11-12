import clsx from 'clsx';
import { HighlightedContent } from './ConsoleView';
import './DaemonLogLine.css';

/** GET /api/daemon/console 한 줄 (spans: 데몬이 나눈 낱말 조각) */
export interface DaemonLogEntry {
    id: number;
    level: string;
    timestamp: string | number;
    message: string;
    spans?: unknown;
}

/**
 * 데몬 로그 한 줄: 시각 · 레벨 배지 · 본문 (사용자 2026-11-12 시안 A).
 * 레벨 색은 배지와 왼쪽 띠에만 쓰고, 본문은 데몬 spans(highlight.rs DAEMON_LOG_RULES)로 낱말마다 칠한다. 앱 작업 막대와 웹 PC 탭이 같이 쓴다
 */
export function DaemonLogLine({ line, locale }: { line: DaemonLogEntry; locale?: string }) {
    const level = String(line.level || 'info').toLowerCase();
    return (
        <div className={clsx('daemon-log-line', `level-${level}`)}>
            <span className="daemon-log-time">
                {new Date(line.timestamp).toLocaleTimeString(locale, { hour12: false })}
            </span>
            <span className="daemon-log-level">{level.toUpperCase()}</span>
            <span className="daemon-log-msg">
                <HighlightedContent text={line.message} spans={line.spans} />
            </span>
        </div>
    );
}
