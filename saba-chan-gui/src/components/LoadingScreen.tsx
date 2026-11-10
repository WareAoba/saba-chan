import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { WindowControls } from './shell/AppBar';
import { logTokens, MatrixRain, type MatrixPull } from './shell/MatrixRain';
import { fallFrame, type OpeningKind, openingFrame, openingShake } from '../utils/opening';

/**
 * LoadingScreen — displayed while daemon is initializing.
 * failed가 있으면 데몬을 못 띄운 것: 진행 막대 대신 이유와 [다시 시도]·[그대로 열기]를 보인다.
 */
/**
 * 로딩 중 데몬이 남긴 최신 로그 한 줄과, 최근 로그에서 뽑은 매트릭스 낱말 (GET /api/daemon/console).
 * 데몬이 아직 없으면 둘 다 비어 있다
 */
interface DaemonLogLine {
    level?: string;
    message?: string;
}

function useDaemonLog(active: boolean) {
    const [line, setLine] = useState<DaemonLogLine | null>(null);
    const [tokens, setTokens] = useState<string[]>([]);
    useEffect(() => {
        if (!active || !window.api?.daemonConsole) return undefined;
        let alive = true;
        let timer: ReturnType<typeof setTimeout> | undefined;
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
function wordmarkUrl(src: string) {
    try {
        return new URL(src, document.baseURI).href;
    } catch {
        return src;
    }
}
const LOG_TOKEN_LINES = 50;

interface LoadingScreenProps {
    logoSrc: string;
    initStatus?: string;
    initProgress?: number;
    failed?: string | null;
    onRetry?: () => void | Promise<void>;
    onContinue?: () => void;
    leaving?: boolean;
    /**
     * 오프닝 (utils/opening.ts). prepare: 메인 화면을 아래에 그리는 동안 로딩 화면 그대로 덮어 둔다.
     * play: openingKind대로 메인 화면을 연다 (burst: 기동음과 함께 끌어모아 터뜨림, fall: 후두둑). 끝나면 onOpened
     */
    opening?: 'prepare' | 'play' | null;
    openingKind?: OpeningKind;
    onOpened?: () => void;
    /** 화음 순간 메인 화면을 흔들 만큼 (px). 끝나면 0, 0 */
    onShake?: (dx: number, dy: number) => void;
}

/**
 * 오프닝 재생: 시작 시각과 로고 가운데를 정해 매트릭스에 넘기고, 같은 시계로 가운데 묶음·섬광·바탕을 움직인다.
 * burst: 화음(메인 화면이 열리는 때)부터 바탕을 걷어 글자만 메인 화면 위에 흩어지게 두고, 화면을 한 번 흔든다(onShake).
 * fall: 섬광 없이 바탕을 서서히 걷으며 가운데 묶음을 가라앉힌다
 */
function useOpeningPlayback(
    playing: boolean,
    kind: OpeningKind,
    onOpened?: () => void,
    onShake?: (dx: number, dy: number) => void,
) {
    const rootRef = useRef<HTMLDivElement>(null);
    const contentRef = useRef<HTMLDivElement>(null);
    const logoRef = useRef<HTMLDivElement>(null);
    const flashRef = useRef<HTMLElement>(null);
    const [target, setTarget] = useState<MatrixPull | null>(null);
    const onOpenedRef = useRef(onOpened);
    onOpenedRef.current = onOpened;
    const onShakeRef = useRef(onShake);
    onShakeRef.current = onShake;

    useEffect(() => {
        if (!playing) return undefined;
        const box = logoRef.current?.getBoundingClientRect();
        const start = performance.now();
        setTarget({
            start,
            x: box ? box.left + box.width / 2 : window.innerWidth / 2,
            y: box ? box.top + box.height / 2 : window.innerHeight / 2,
            kind,
        });
        let raf = 0;
        const finish = () => {
            onShakeRef.current?.(0, 0);
            onOpenedRef.current?.();
        };
        const tickBurst = (ms: number) => {
            const f = openingFrame(ms);
            const content = contentRef.current;
            if (content) {
                content.style.transform = `scale(${0.15 + 0.85 * f.content})`;
                content.style.opacity = String(f.opened ? 0 : 0.05 + 0.95 * f.content);
            }
            if (flashRef.current) flashRef.current.style.opacity = String(0.95 * f.flash);
            const root = rootRef.current;
            root?.classList.toggle('opened', f.opened);
            // 뒤에 흐리게 깔려 있던 콘솔이 끌려 오면서 또렷해진다 (--loading-matrix-opacity → 1)
            const matrix = root?.querySelector<HTMLElement>('.loading-matrix');
            if (matrix) matrix.style.opacity = f.opened ? '1' : `calc(var(--loading-matrix-opacity) + (1 - var(--loading-matrix-opacity)) * ${Math.min(1, f.pull * 3)})`;
            const shake = openingShake(ms) * 6;
            onShakeRef.current?.((Math.random() - 0.5) * 2 * shake, (Math.random() - 0.5) * 2 * shake);
            return f.done;
        };
        const tickFall = (ms: number) => {
            const f = fallFrame(ms);
            const content = contentRef.current;
            if (content) {
                content.style.transform = `translateY(${f.sink * window.innerHeight * 0.06}px) scale(${1 - 0.04 * f.sink})`;
                content.style.opacity = String(1 - f.sink);
            }
            const root = rootRef.current;
            if (root) {
                // 떨어지는 글자가 보이게 콘솔을 또렷하게 하고, 바탕·비네트·창 버튼은 함께 옅어진다 (App.css .loading-screen.falling)
                const matrix = root.querySelector<HTMLElement>('.loading-matrix');
                if (matrix) matrix.style.opacity = `calc(var(--loading-matrix-opacity) + (1 - var(--loading-matrix-opacity)) * ${0.8 * Math.min(1, ms / 150)})`;
                root.classList.add('falling');
                root.style.setProperty('--opening-cover', String(f.cover));
                root.style.pointerEvents = f.cover < 0.5 ? 'none' : '';
            }
            return f.done;
        };
        const tick = (now: number) => {
            const done = kind === 'fall' ? tickFall(now - start) : tickBurst(now - start);
            if (done) {
                finish();
                return;
            }
            raf = requestAnimationFrame(tick);
        };
        raf = requestAnimationFrame(tick);
        return () => {
            cancelAnimationFrame(raf);
            onShakeRef.current?.(0, 0);
        };
    }, [playing, kind]);

    return { rootRef, contentRef, logoRef, flashRef, target: playing ? target : null };
}

export function LoadingScreen({
    logoSrc,
    initStatus,
    initProgress,
    failed,
    onRetry,
    onContinue,
    leaving = false,
    opening = null,
    openingKind = 'burst',
    onOpened,
    onShake,
}: LoadingScreenProps) {
    const { t } = useTranslation('gui');
    const [retrying, setRetrying] = useState(false);
    const { line: log, tokens } = useDaemonLog(!failed && !leaving && !opening);
    const pull = useOpeningPlayback(opening === 'play', openingKind, onOpened, onShake);

    return (
        <div
            ref={pull.rootRef}
            className={clsx('loading-screen', { leaving, opening: !!opening })}
            aria-hidden={leaving || opening === 'play' || undefined}
        >
            <i ref={pull.flashRef} className="loading-flash" aria-hidden="true" />
            {/* 뒤에서 흐르는 서버 콘솔 (가운데는 비워 둔다) */}
            <MatrixRain className="loading-matrix" tokens={tokens} pull={pull.target} />
            <div className="loading-vignette" aria-hidden="true" />
            <div className="appbar appbar-bare" data-tauri-drag-region>
                <div className="appbar-spacer" data-tauri-drag-region />
                <WindowControls />
            </div>
            <div ref={pull.contentRef} className="loading-content">
                <div ref={pull.logoRef} className="loading-logo-container">
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
                    style={{ '--wordmark': `url("${wordmarkUrl(logoSrc)}")` } as React.CSSProperties}
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
