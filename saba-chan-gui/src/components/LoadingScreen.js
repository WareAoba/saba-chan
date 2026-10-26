import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { WindowControls } from './shell/AppBar';
import { logTokens, MatrixRain } from './shell/MatrixRain';

/**
 * LoadingScreen — displayed while daemon is initializing.
 * failed가 있으면 데몬을 못 띄운 것: 진행 막대 대신 이유와 [다시 시도]·[그대로 열기]를 보인다.
 */
/**
 * 로딩 중 데몬이 남긴 최신 로그 한 줄과, 최근 로그에서 뽑은 매트릭스 낱말 (GET /api/daemon/console).
 * 데몬이 아직 없으면 둘 다 비어 있다
 */
function useDaemonLog(active) {
    const [line, setLine] = useState(null);
    const [tokens, setTokens] = useState([]);
    useEffect(() => {
        if (!active || !window.api?.daemonConsole) return undefined;
        let alive = true;
        let timer;
        const poll = async () => {
            try {
                const res = await window.api.daemonConsole({ count: LOG_TOKEN_LINES });
                const lines = res?.lines || [];
                const last = lines[lines.length - 1];
                if (alive && last?.message) setLine(last);
                const words = logTokens(lines);
                if (alive && words.length) setTokens(words);
            } catch {
                // 데몬이 아직 안 떴으면 다음에
            }
            if (alive) timer = setTimeout(poll, LOG_POLL_MS);
        };
        poll();
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [active]);
    return { line, tokens };
}

const LOG_POLL_MS = 600;

/**
 * 글자 로고 주소를 문서 기준 절대 주소로. CSS 변수 속 url()은 그 변수를 쓰는 CSS 파일 기준으로 풀려서
 * ./logo-kr.png가 /assets/logo-kr.png가 되고, 셸은 없는 파일에 HTML을 돌려줘 마스크가 투명해진다
 */
function wordmarkUrl(src) {
    try {
        return new URL(src, document.baseURI).href;
    } catch {
        return src;
    }
}
const LOG_TOKEN_LINES = 50;

export function LoadingScreen({ logoSrc, initStatus, initProgress, failed, onRetry, onContinue, leaving = false }) {
    const { t } = useTranslation('gui');
    const [retrying, setRetrying] = useState(false);
    const { line: log, tokens } = useDaemonLog(!failed && !leaving);

    return (
        <div className={clsx('loading-screen', { leaving })} aria-hidden={leaving || undefined}>
            {/* 뒤에서 흐르는 서버 콘솔 (가운데는 비워 둔다) */}
            <MatrixRain className="loading-matrix" tokens={tokens} />
            <div className="loading-vignette" aria-hidden="true" />
            <div className="appbar appbar-bare">
                <div className="appbar-spacer" />
                <WindowControls />
            </div>
            <div className="loading-content">
                <div className="loading-logo-container">
                    <i className="glow-blur"></i>
                    <i className="glow-ring"></i>
                    <i className="glow-mask"></i>
                    <img src="./title.png" alt="" className="loading-logo-img" />
                </div>
                {/* 전용 글자 로고: 그림을 마스크로 써서 테마 글자색으로 칠한다 */}
                <span
                    className="loading-logo-text"
                    role="img"
                    aria-label={t('common:app_name')}
                    style={{ '--wordmark': `url("${wordmarkUrl(logoSrc)}")` }}
                />
                {failed ? (
                    <div className="loading-failed" role="alert">
                        <p className="loading-failed-message">
                            <Icon name="alertTriangle" size="sm" /> {failed}
                        </p>
                        <p className="loading-failed-hint">{t('loading.failed_hint')}</p>
                        <div className="loading-failed-actions">
                            <button
                                type="button"
                                className="btn btn-primary"
                                disabled={retrying}
                                onClick={async () => {
                                    setRetrying(true);
                                    try {
                                        await onRetry?.();
                                    } finally {
                                        setRetrying(false);
                                    }
                                }}
                            >
                                <Icon name="refresh" size="sm" className={retrying ? 'spin' : undefined} />{' '}
                                {t('loading.retry')}
                            </button>
                            <button type="button" className="btn btn-ghost" disabled={retrying} onClick={onContinue}>
                                {t('loading.continue')}
                            </button>
                        </div>
                    </div>
                ) : (
                    <div className="loading-term">
                        {/* 첫 줄: 지금 하는 일을 사람 말로. 둘째 줄: 데몬이 실제로 남긴 마지막 로그 */}
                        <div className="loading-status" aria-live="polite">
                            <span className="loading-prompt" aria-hidden="true">
                                saba@core:~$
                            </span>
                            <span className="loading-status-text">{initStatus || t('loading.step_init')}</span>
                            <span className="loading-caret" aria-hidden="true" />
                        </div>
                        <div className={clsx('loading-log', log?.level && `level-${String(log.level).toLowerCase()}`)}>
                            {log ? (
                                <>
                                    {log.level && (
                                        <span className="loading-log-level">{String(log.level).toUpperCase()}</span>
                                    )}
                                    <span className="loading-log-msg">{log.message}</span>
                                </>
                            ) : (
                                <span className="loading-log-msg">…</span>
                            )}
                        </div>
                        <div className="loading-progress-row">
                            <div className="loading-progress-bar">
                                <div className="loading-progress-fill" style={{ width: `${initProgress}%` }}></div>
                            </div>
                            <span className="loading-progress-pct">{Math.round(initProgress || 0)}%</span>
                        </div>
                    </div>
                )}
                <div className="loading-tips">
                    <Icon name="info" size="sm" /> {t('buttons.loading_tips')}
                </div>
            </div>
        </div>
    );
}
