import clsx from 'clsx';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { auditWhat } from '../servers/AuditTab';

/** 맨 아래 줄을 띄울 IPC가 있는지 (감사 로그) */
export const activityAvailable = () => !!window.api?.auditList;
/** 펼쳐서 데몬 콘솔을 볼 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const daemonConsoleAvailable = () => !!window.api?.daemonConsole;

export const ACTIVITY_POLL_MS = 3000;
export const CONSOLE_POLL_MS = 1000;
const CONSOLE_MAX = 2000;
const SOURCES = ['gui', 'cli', 'api', 'discord', 'schedule', 'daemon'];

/** 몇 초 전·몇 분 전 (현재 언어로) */
export function ago(ts, now, lang) {
    const s = Math.max(0, Math.round(now / 1000 - ts));
    const rtf = new Intl.RelativeTimeFormat(lang, { numeric: 'auto' });
    if (s < 60) return rtf.format(-s, 'second');
    if (s < 3600) return rtf.format(-Math.floor(s / 60), 'minute');
    if (s < 86400) return rtf.format(-Math.floor(s / 3600), 'hour');
    return rtf.format(-Math.floor(s / 86400), 'day');
}

/** 감사 한 건 → "디스코드 · 김치 · 마크서버 · 정지" (성공 여부·시각은 따로 그린다) */
export function activityText(e, t) {
    const source = SOURCES.includes(e.source) ? t(`audit.source_${e.source}`) : e.source;
    return [source, e.userName || e.user, e.instance || e.instanceId, auditWhat(e, t)].filter(Boolean).join(' · ');
}

/** 펼친 보기: 데몬이 남긴 로그 전체 (GET /api/daemon/console, 새 줄만 이어 받는다) */
function DaemonConsole() {
    const { t, i18n } = useTranslation('gui');
    const [lines, setLines] = useState([]);
    const [error, setError] = useState(null);
    const outputRef = useRef(null);
    const stickRef = useRef(true);

    useEffect(() => {
        let alive = true;
        let timer;
        let next; // 처음엔 최근 것부터, 그 뒤로는 받은 다음 id부터 (since는 그 id를 포함한다)
        const poll = async () => {
            try {
                const res = await window.api.daemonConsole(next === undefined ? {} : { since: next });
                if (!alive) return;
                const got = Array.isArray(res?.lines) ? res.lines : null;
                if (!got) throw new Error(res?.error || t('activity.console_failed'));
                setError(null);
                if (got.length) {
                    next = got[got.length - 1].id + 1;
                    setLines((prev) => {
                        const all = prev.concat(got);
                        return all.length > CONSOLE_MAX ? all.slice(-CONSOLE_MAX) : all;
                    });
                }
            } catch (err) {
                if (alive) setError(err.message || String(err));
            }
            if (alive) timer = setTimeout(poll, CONSOLE_POLL_MS);
        };
        poll();
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [t]);

    // 맨 아래를 보고 있을 때만 새 줄을 따라 내려간다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 줄 수가 바뀔 때만 스크롤
    useLayoutEffect(() => {
        const el = outputRef.current;
        if (el && stickRef.current) el.scrollTop = el.scrollHeight;
    }, [lines.length]);

    return (
        <div
            className="activity-console"
            ref={outputRef}
            role="log"
            aria-label={t('activity.console_title')}
            onScroll={(e) => {
                const el = e.currentTarget;
                stickRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
            }}
        >
            {error && <div className="activity-console-error">{error}</div>}
            {lines.length === 0 && !error && (
                <div className="activity-console-empty">{t('activity.console_empty')}</div>
            )}
            {lines.map((l) => (
                <div key={l.id} className={clsx('activity-console-line', `level-${String(l.level).toLowerCase()}`)}>
                    <span className="activity-console-time">
                        {new Date(l.timestamp).toLocaleTimeString(i18n.language, { hour12: false })}
                    </span>
                    <span className="activity-console-level">{String(l.level).toUpperCase()}</span>
                    <span className="activity-console-msg">{l.message}</span>
                </div>
            ))}
        </div>
    );
}

/**
 * 앱 맨 아래 한 줄: 데몬이 방금 실행한 서버 관련 명령 (감사 로그 최신 1건).
 * 누르면 펼쳐져 데몬 콘솔 전체를 보인다. 이벤트 스트림이 생기기 전까지는 폴링한다.
 */
export function ActivityBar() {
    const { t, i18n } = useTranslation('gui');
    const [last, setLast] = useState(undefined); // undefined = 아직 모름, null = 기록 없음
    const [now, setNow] = useState(() => Date.now());
    const [open, setOpen] = useState(false);
    const canExpand = daemonConsoleAvailable();

    useEffect(() => {
        let alive = true;
        let timer;
        const poll = async () => {
            if (!document.hidden) {
                try {
                    const res = await window.api.auditList({ limit: 1 });
                    if (alive && Array.isArray(res?.entries)) setLast(res.entries[0] || null);
                } catch {
                    // 데몬이 잠깐 안 받아도 마지막 줄은 그대로 둔다
                }
                if (alive) setNow(Date.now());
            }
            if (alive) timer = setTimeout(poll, ACTIVITY_POLL_MS);
        };
        poll();
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, []);

    if (last === undefined) return null;

    const summary = last ? (
        <>
            <span className="activity-text">{activityText(last, t)}</span>
            <span className={clsx('activity-result', last.ok ? 'ok' : 'fail')}>
                {last.ok ? t('audit.ok') : t('audit.failed', { status: last.status })}
            </span>
            <span className="activity-time">{ago(last.ts, now, i18n.language)}</span>
        </>
    ) : (
        <span className="activity-text muted">{t('activity.empty')}</span>
    );

    return (
        <footer className={clsx('activity-bar', open && 'open')}>
            {open && <DaemonConsole />}
            {canExpand ? (
                <button
                    type="button"
                    className="activity-line"
                    onClick={() => setOpen((o) => !o)}
                    aria-expanded={open}
                    title={open ? t('activity.collapse') : t('activity.expand')}
                >
                    <Icon name="terminal" size="xs" />
                    {summary}
                    <Icon name={open ? 'chevronDown' : 'chevronUp'} size="xs" />
                </button>
            ) : (
                <div className="activity-line">
                    <Icon name="terminal" size="xs" />
                    {summary}
                </div>
            )}
        </footer>
    );
}
