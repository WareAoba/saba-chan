import clsx from 'clsx';
import { useUIStore } from '../../stores/useUIStore';
import { Icon } from '../Icon';

/**
 * 서버 시작·설치 같은 작업이 도는 동안 상단 바에 원형 진행과 문구를 띄운다
 * (Python 준비 표시와 같은 모양). 예전 하단 진행 바를 대신한다.
 */
export function TaskIndicator() {
    const task = useUIStore((s) => s.progressBar);
    if (!task) return null;

    const done = task.percent === 100;
    const known = !task.indeterminate && Number.isFinite(task.percent) && task.percent > 0;
    const percent = known ? Math.max(0, Math.min(100, task.percent)) : 0;
    const r = 7;
    const c = 2 * Math.PI * r;

    return (
        <output className={clsx('appbar-preflight appbar-task', { done })} aria-live="polite">
            {done ? (
                <Icon name="check" size="sm" />
            ) : (
                <svg viewBox="0 0 18 18" width="18" height="18" aria-hidden="true">
                    <circle className="preflight-track" cx="9" cy="9" r={r} fill="none" strokeWidth="2.5" />
                    <circle
                        className={clsx('preflight-fill', { indeterminate: !known })}
                        cx="9"
                        cy="9"
                        r={r}
                        fill="none"
                        strokeWidth="2.5"
                        strokeLinecap="round"
                        strokeDasharray={c}
                        strokeDashoffset={known ? c * (1 - percent / 100) : c * 0.75}
                        transform="rotate(-90 9 9)"
                    />
                </svg>
            )}
            <span className="appbar-preflight-text">
                {task.message}
                {known && !done && <span className="appbar-preflight-pct"> {Math.round(percent)}%</span>}
            </span>
        </output>
    );
}
