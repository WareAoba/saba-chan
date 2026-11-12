import clsx from 'clsx';
import { useRef } from 'react';
import { useUIStore } from '../../stores/useUIStore';
import { Icon } from '../Icon';
import { AppBarPill, PillRing } from './AppBarPill';

/**
 * 서버 시작·설치 같은 작업이 도는 동안 상단 바에 원형 진행과 문구를 띄운다
 * (Python 준비 표시와 같은 모양·움직임, AppBarPill). 예전 하단 진행 바를 대신한다.
 */
export function TaskIndicator() {
    const current = useUIStore((s) => s.progressBar);
    // 사라지는 움직임 동안에는 마지막 작업을 그대로 그린다
    const last = useRef(current);
    if (current) last.current = current;
    const task = current || last.current;
    if (!task) return null;

    const done = task.percent === 100;
    const known = !task.indeterminate && Number.isFinite(task.percent) && (task.percent as number) >= 0;
    const percent = known ? Math.max(0, Math.min(100, task.percent as number)) : 0;

    return (
        <AppBarPill
            visible={!!current}
            className={clsx('appbar-task', { done })}
            aria-live="polite"
            changeKey={done}
            icon={
                done ? (
                    <Icon name="check" size="sm" />
                ) : (
                    <PillRing percent={known ? percent : undefined} />
                )
            }
            label={
                <>
                    {task.message}
                    {known && !done && <span className="appbar-preflight-pct"> {Math.round(percent)}%</span>}
                </>
            }
        />
    );
}
