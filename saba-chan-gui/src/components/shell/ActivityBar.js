import clsx from 'clsx';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { auditWhat } from '../../utils/audit';
import { auditEntryOf, eventsAvailable, subscribeDaemonEvents } from '../../utils/daemonEvents';
import { Icon } from '../Icon';

/** 맨 아래 줄을 띄울 IPC가 있는지 (감사 로그) */
export const activityAvailable = () => !!window.api?.auditList;
/** 펼쳐서 데몬 콘솔을 볼 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const daemonConsoleAvailable = () => !!window.api?.daemonConsole;

export const ACTIVITY_POLL_MS = 3000;
/** 이벤트 스트림이 있으면 놓친 것만 메우는 느린 폴링 */
export const ACTIVITY_SLOW_POLL_MS = 30000;
export const CONSOLE_POLL_MS = 1000;
const CONSOLE_MAX = 2000;
const SOURCES = ['gui', 'cli', 'api', 'discord', 'schedule', 'daemon', 'web', 'remote', 'mcp', 'restart'];

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

    // 언어가 바뀌어도 폴링을 다시 시작하지 않는다 (받은 줄이 겹치지 않게)
    const tRef = useRef(t);
    tRef.current = t;

    useEffect(() => {
        let alive = true;
        let timer;
        let next; // 처음엔 최근 것부터, 그 뒤로는 받은 다음 id부터 (since는 그 id를 포함한다)
        let epoch; // 데몬 로그 버퍼 세대. 데몬이 다시 켜지면 id가 1부터라 바뀌면 처음부터 다시 받는다 (L55)
        const poll = async () => {
            try {
                let res = await window.api.daemonConsole(next === undefined ? {} : { since: next });
                if (next !== undefined && res?.epoch !== undefined && res.epoch !== epoch) {
                    next = undefined;
                    setLines([]);
                    res = await window.api.daemonConsole({});
                }
                if (res?.epoch !== undefined) epoch = res.epoch;
                const got = Array.isArray(res?.lines) ? res.lines : null;
                if (!alive) return;
                if (!got) throw new Error(res?.error || tRef.current('activity.console_failed'));
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
    }, []);

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

/** 방금 실행된 명령을 줄에 보여 주는 시간. 지나면 기본 문구로 돌아간다 */
export const RECENT_SHOW_MS = 6000;
const SLIDE_MS = 360;
const CONSOLE_CLOSE_MS = 180;

/** 내용이 바뀔 때 위로 밀려 올라가며 바뀌고, 새 글자 위로 빛이 한 번 지나간다 */
function Ticker({ id, children }) {
    const [items, setItems] = useState([{ id, node: children, state: 'still' }]);
    useEffect(() => {
        setItems((prev) => {
            const cur = prev[prev.length - 1];
            if (cur.id === id) return [...prev.slice(0, -1), { ...cur, node: children }];
            return [{ ...cur, state: 'leaving' }, { id, node: children, state: 'entering' }];
        });
    }, [id, children]);
    useEffect(() => {
        if (!items.some((it) => it.state !== 'still')) return;
        const timer = setTimeout(
            () => setItems((prev) => prev.filter((it) => it.state !== 'leaving').map((it) => ({ ...it, state: 'still' }))),
            SLIDE_MS,
        );
        return () => clearTimeout(timer);
    }, [items]);
    return (
        <span className="activity-ticker">
            {items.map((it) => (
                <span key={it.id} className={clsx('activity-slide', it.state)} aria-hidden={it.state === 'leaving'}>
                    {it.node}
                </span>
            ))}
        </span>
    );
}

/**
 * 앱 맨 아래 한 줄: 데몬이 방금 실행한 서버 관련 명령을 잠깐 보이고 기본 문구로 돌아간다.
 * 누르면 데몬 콘솔 전체가 줄 위로 튀어나오듯 떠서 다른 화면을 밀지 않고 덮는다.
 * 이벤트 스트림(action.run)이 있으면 바로 바꾸고, 폴링은 느리게 메우기만 한다.
 */
export function ActivityBar() {
    const { t, i18n } = useTranslation('gui');
    const [last, setLast] = useState(undefined); // undefined = 아직 모름, null = 기록 없음
    const [now, setNow] = useState(() => Date.now());
    const [open, setOpen] = useState(false);
    const [closing, setClosing] = useState(false);
    const [recent, setRecent] = useState(false);
    const canExpand = daemonConsoleAvailable();
    const barRef = useRef(null);

    useEffect(() => {
        let alive = true;
        let timer;
        const live = eventsAvailable();
        const fetchLast = async () => {
            try {
                const res = await window.api.auditList({ limit: 1 });
                if (alive && Array.isArray(res?.entries)) setLast(res.entries[0] || null);
            } catch {
                // 데몬이 잠깐 안 받아도 마지막 줄은 그대로 둔다
            }
        };
        const poll = async () => {
            if (!document.hidden) {
                await fetchLast();
                if (alive) setNow(Date.now());
            }
            if (alive) timer = setTimeout(poll, live ? ACTIVITY_SLOW_POLL_MS : ACTIVITY_POLL_MS);
        };
        const off = subscribeDaemonEvents((ev) => {
            if (!alive) return;
            if (ev.type === 'action.run') {
                setLast(auditEntryOf(ev.data));
                setNow(Date.now());
            } else if (ev.type === 'resync') {
                fetchLast();
            }
        });
        // 이벤트로 줄이 바뀌지 않아도 "몇 초 전"은 흐르게 한다
        const clock = live ? setInterval(() => !document.hidden && setNow(Date.now()), ACTIVITY_POLL_MS) : null;
        poll();
        return () => {
            alive = false;
            clearTimeout(timer);
            clearInterval(clock);
            off();
        };
    }, []);

    // 새 명령이 방금 실행됐으면 잠깐 보여 주고 기본 문구로 돌아간다
    const lastKey = last ? `${last.ts}|${last.action}|${last.instanceId}|${last.source}` : '';
    // biome-ignore lint/correctness/useExhaustiveDependencies: 명령이 바뀔 때(lastKey)만 다시 잰다
    useEffect(() => {
        if (!last) return setRecent(false);
        const left = RECENT_SHOW_MS - (Date.now() - last.ts * 1000);
        if (left <= 0) return setRecent(false);
        setRecent(true);
        const timer = setTimeout(() => setRecent(false), Math.min(left, RECENT_SHOW_MS));
        return () => clearTimeout(timer);
    }, [lastKey]);

    const closeConsole = () => {
        setClosing(true);
        setTimeout(() => {
            setOpen(false);
            setClosing(false);
        }, CONSOLE_CLOSE_MS);
    };
    const toggle = () => (open && !closing ? closeConsole() : setOpen(true));

    // 콘솔이 떠 있으면 Esc나 바깥 누르기로 닫는다
    // biome-ignore lint/correctness/useExhaustiveDependencies: 열림 상태가 바뀔 때만 건다
    useEffect(() => {
        if (!open || closing) return;
        const onKey = (e) => e.key === 'Escape' && closeConsole();
        const onDown = (e) => barRef.current && !barRef.current.contains(e.target) && closeConsole();
        document.addEventListener('keydown', onKey);
        document.addEventListener('mousedown', onDown);
        return () => {
            document.removeEventListener('keydown', onKey);
            document.removeEventListener('mousedown', onDown);
        };
    }, [open, closing]);

    if (last === undefined) return null;

    // 펼칠 수 있으면 평소엔 안내 문구, 방금 실행된 명령이 있을 때만 그 명령을 보인다
    const showCommand = last && (recent || !canExpand);
    const summary = showCommand ? (
        <Ticker id={`cmd:${lastKey}`}>
            <span className="activity-text activity-shine">{activityText(last, t)}</span>
            <span className={clsx('activity-result', last.ok ? 'ok' : 'fail')}>
                {last.ok ? t('audit.ok') : t('audit.failed', { status: last.status })}
            </span>
            <span className="activity-time">{ago(last.ts, now, i18n.language)}</span>
        </Ticker>
    ) : (
        <Ticker id={canExpand ? 'idle' : 'empty'}>
            <span className="activity-text activity-shine muted">
                {canExpand ? t('activity.idle') : t('activity.empty')}
            </span>
        </Ticker>
    );

    const shown = open && !closing;

    return (
        <footer className={clsx('activity-bar', shown && 'open')} ref={barRef}>
            {open && (
                <div className={clsx('activity-console-pop', { closing })}>
                    <DaemonConsole />
                </div>
            )}
            {canExpand ? (
                <button
                    type="button"
                    className="activity-line"
                    onClick={toggle}
                    aria-expanded={shown}
                    data-tooltip={shown ? t('activity.collapse') : t('activity.expand')}
                >
                    <Icon name="terminal" size="xs" />
                    {summary}
                    <span className={clsx('activity-chevron', { flipped: shown })}>
                        <Icon name="chevronUp" size="xs" />
                    </span>
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
