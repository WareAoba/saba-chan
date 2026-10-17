import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

const POLL_MS = 1000;
const FAILED_POLL_MS = 5000; // 실패는 저장되지 않아 다음 모듈 호출이 다시 시도한다. 그 결과를 따라간다

/**
 * 데몬이 부팅 때 Python 실행 환경을 준비하는 동안 상단 바에 진행 고리를 띄운다
 * (GET /api/runtime/preflight). 끝나면 사라지고, 실패하면 경고로 남는다.
 * 데몬이 다시 켜지면(daemonStatus가 running으로 돌아오면) 처음부터 다시 본다.
 */
export function RuntimePreflight({ daemonStatus }) {
    const { t } = useTranslation('gui');
    const [state, setState] = useState(null);

    useEffect(() => {
        setState(null);
        if (!window.api?.runtimePreflight || daemonStatus !== 'running') return undefined;
        let alive = true;
        let timer;
        const poll = async () => {
            let next = null;
            try {
                next = await window.api.runtimePreflight();
            } catch {
                /* 데몬이 아직 안 떴으면 다음에 */
            }
            if (!alive) return;
            if (next && !next.error_code && next.phase) setState(next);
            if (next?.phase === 'preflight-done') return; // 끝: 더 볼 것 없음
            timer = setTimeout(poll, next?.phase === 'preflight-failed' ? FAILED_POLL_MS : POLL_MS);
        };
        poll();
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [daemonStatus]);

    if (!state || state.phase === 'preflight-done' || state.phase === 'preflight-idle') return null;

    const failed = state.phase === 'preflight-failed';
    const percent = Math.max(0, Math.min(100, Number(state.percent) || 0));
    const label = failed
        ? t('shell.preflight_failed', { error: state.error || '-' })
        : t('shell.preflight_python', { percent });
    const r = 7;
    const c = 2 * Math.PI * r;

    return (
        <output className={clsx('appbar-preflight', { failed })} data-tooltip={label} aria-label={label}>
            {failed ? (
                <Icon name="alertTriangle" size="sm" />
            ) : (
                <svg viewBox="0 0 18 18" width="18" height="18" aria-hidden="true">
                    <circle className="preflight-track" cx="9" cy="9" r={r} fill="none" strokeWidth="2.5" />
                    <circle
                        className={clsx('preflight-fill', { indeterminate: percent === 0 })}
                        cx="9"
                        cy="9"
                        r={r}
                        fill="none"
                        strokeWidth="2.5"
                        strokeLinecap="round"
                        strokeDasharray={c}
                        strokeDashoffset={percent === 0 ? c * 0.75 : c * (1 - percent / 100)}
                        transform="rotate(-90 9 9)"
                    />
                </svg>
            )}
            <span className="appbar-preflight-text">
                {failed ? t('shell.preflight_failed_short') : t('shell.preflight_short')}
            </span>
        </output>
    );
}
