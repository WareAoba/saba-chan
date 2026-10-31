import { auditRange, auditResult, auditWhat, auditWho } from '@gui/utils/audit';
import { LANGUAGE_OPTIONS } from '@gui/utils/languages';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { setLanguage } from '../i18n';
import { applyTheme, readPref, THEMES, writePref } from '../prefs';
import { sizeText } from './BackupsPanel';
import { Dropdown } from './Dropdown';
import { startPoll } from './poll';
import { Button, Empty, Loading, Section, Segments } from './ui';

function Meter({ label, percent, detail }) {
    const p = Math.max(0, Math.min(100, percent || 0));
    return (
        <div className="meter">
            <div className="meter-head">
                <span>{label}</span>
                <strong>{Math.round(p)}%</strong>
            </div>
            <div className="meter-track" aria-hidden="true">
                <div className="meter-fill" style={{ width: `${p}%` }} />
            </div>
            {detail && <small className="hint">{detail}</small>}
        </div>
    );
}

function Metrics({ api }) {
    const { t } = useTranslation();
    const [snap, setSnap] = useState(null);
    useEffect(() => {
        let stopped = false;
        const load = async () => {
            const res = await api.systemMetrics({ sizes: false });
            if (!stopped && res?.cpu) setSnap(res);
        };
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api]);

    if (!snap) return <Loading />;
    const mem = snap.memory || {};
    const running = (snap.instances || []).filter((i) => i.running);
    return (
        <>
            <Meter label={t('more.cpu')} percent={snap.cpu?.total_percent} />
            <Meter
                label={t('more.memory')}
                percent={mem.total_bytes ? (mem.used_bytes / mem.total_bytes) * 100 : 0}
                detail={`${sizeText(mem.used_bytes)} / ${sizeText(mem.total_bytes)}`}
            />
            <h3 className="sub-title">{t('more.servers_usage')}</h3>
            {running.length === 0 ? (
                <Empty>{t('more.none_running')}</Empty>
            ) : (
                <ul className="rows">
                    {running.map((i) => (
                        <li key={i.id} className="row">
                            <div className="row-main">
                                <div className="row-title">{i.name}</div>
                                <div className="row-sub">
                                    CPU {Math.round(i.cpu_percent)}% · {sizeText(i.memory_bytes)}
                                </div>
                            </div>
                            {mem.total_bytes > 0 && (
                                <div className="meter-track meter-mini" aria-hidden="true">
                                    <div
                                        className="meter-fill"
                                        style={{ width: `${Math.min(100, (i.memory_bytes / mem.total_bytes) * 100)}%` }}
                                    />
                                </div>
                            )}
                        </li>
                    ))}
                </ul>
            )}
        </>
    );
}

function DaemonLog({ api }) {
    const { t } = useTranslation();
    const [lines, setLines] = useState(null);
    const next = useRef(undefined);
    const box = useRef(null);
    useEffect(() => {
        let stopped = false;
        const load = async () => {
            const res = await api.daemonConsole(next.current === undefined ? {} : { since: next.current });
            if (stopped || !Array.isArray(res?.lines)) return;
            if (res.lines.length) next.current = res.lines[res.lines.length - 1].id + 1;
            setLines((prev) => [...(prev ?? []), ...res.lines].slice(-300));
        };
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api]);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 새 줄이 올 때마다 맨 아래로 내린다
    useEffect(() => {
        if (box.current) box.current.scrollTop = box.current.scrollHeight;
    }, [lines]);

    if (lines === null) return <Loading />;
    return (
        <pre className="console-lines console-tall" ref={box}>
            {lines.length === 0 ? (
                <span className="console-empty">{t('console.empty')}</span>
            ) : (
                lines.map((l) => (
                    <div key={l.id} className={`line line-${l.level || 'info'}`}>
                        {l.message}
                    </div>
                ))
            )}
        </pre>
    );
}

const PAGE = 100;
const MAX = 1000; // 데몬 상한
const PERIODS = ['all', 'today', '7d', '30d'];
const TONE = { ok: 'pill-ok', idle: '', stop: 'pill-stop' };

/** 활동 기록 (GET /api/audit, 최신이 먼저). 데몬이 웹에는 PC 경로를 가려 준다. 문구는 앱 작업 기록과 같다 */
function ActivityLog({ api }) {
    const { t, i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [period, setPeriod] = useState('all');
    const [limit, setLimit] = useState(PAGE);
    const [entries, setEntries] = useState(null);
    const [error, setError] = useState(null);
    const [loading, setLoading] = useState(false);
    const seq = useRef(0); // 늦게 온 이전 응답이 새 응답을 덮지 않게

    const load = useCallback(async () => {
        const mine = ++seq.current;
        setLoading(true);
        const res = await api.auditList({ limit, ...auditRange(period) });
        if (mine !== seq.current) return;
        setLoading(false);
        if (Array.isArray(res?.entries)) {
            setEntries(res.entries);
            setError(null);
        } else setError(tg('audit.load_failed'));
    }, [api, limit, period, tg]);

    useEffect(() => {
        load();
    }, [load]);

    const fmt = (s) =>
        new Date(s * 1000).toLocaleString(i18n.language, {
            month: 'numeric',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
        });
    // 받은 건수가 요청보다 적으면 더 없다
    const more = entries && entries.length >= limit && limit < MAX;
    return (
        <>
            <div className="activity-head">
                <Segments
                    value={period}
                    onChange={(p) => {
                        setPeriod(p);
                        setLimit(PAGE);
                    }}
                    options={PERIODS.map((p) => ({ value: p, label: tg(`audit.period_${p}`) }))}
                    ariaLabel={tg('audit.period')}
                />
                <Button busy={loading} onClick={load}>
                    {tg('audit.refresh')}
                </Button>
            </div>
            <p className="hint">{t('more.activity_desc')}</p>
            {error && <p className="hint hint-warn">{error}</p>}
            {entries === null ? (
                !error && <Loading />
            ) : entries.length === 0 ? (
                <Empty>{tg('audit.empty')}</Empty>
            ) : (
                <ul className="rows">
                    {entries.map((e, i) => {
                        const result = auditResult(e, tg);
                        return (
                            <li key={`${e.ts}-${i}`} className="row activity-row">
                                <span className="activity-time">{fmt(e.ts)}</span>
                                <div className="row-main">
                                    <div className="row-title">
                                        {(e.instance || e.instanceId) && <strong>{e.instance || e.instanceId} </strong>}
                                        {auditWhat(e, tg)}
                                    </div>
                                    <div className="row-sub">{auditWho(e, tg)}</div>
                                </div>
                                <span className={`pill ${TONE[result.tone] || ''}`}>{result.label}</span>
                            </li>
                        );
                    })}
                </ul>
            )}
            {more && (
                <Button busy={loading} onClick={() => setLimit((l) => Math.min(l + PAGE, MAX))}>
                    {tg('audit.more')}
                </Button>
            )}
        </>
    );
}

/** 이 화면 설정: 언어와 테마는 이 브라우저에만 기억한다 (PC의 앱 설정은 바꾸지 않는다) */
function ScreenSettings() {
    const { t, i18n } = useTranslation();
    const [theme, setTheme] = useState(() => readPref('theme') || 'system');
    const pickTheme = (value) => {
        setTheme(value);
        writePref('theme', value);
        applyTheme(value);
    };
    return (
        <>
            <div className="field">
                <span>{t('more.language')}</span>
                <Dropdown
                    value={i18n.language}
                    onChange={setLanguage}
                    options={LANGUAGE_OPTIONS}
                    ariaLabel={t('more.language')}
                />
            </div>
            <div className="field">
                <span>{t('more.theme')}</span>
                <Segments
                    value={theme}
                    onChange={pickTheme}
                    options={THEMES.map((v) => ({ value: v, label: t(`more.theme_${v}`) }))}
                    ariaLabel={t('more.theme')}
                />
            </div>
            <p className="hint">{t('more.screen_note')}</p>
        </>
    );
}

const VIEWS = {
    metrics: Metrics,
    activity: ActivityLog,
    log: DaemonLog,
    screen: ScreenSettings,
};

export function MoreTab({ api }) {
    const { t } = useTranslation();
    const [view, setView] = useState('metrics');
    const View = VIEWS[view];
    return (
        <div className="panel">
            <Segments
                value={view}
                onChange={setView}
                scroll
                options={[
                    { value: 'metrics', label: t('more.metrics') },
                    { value: 'activity', label: t('more.activity') },
                    { value: 'log', label: t('more.daemon_log') },
                    { value: 'screen', label: t('more.screen') },
                ]}
            />
            <Section>
                <View api={api} />
            </Section>
            <p className="hint">{t('more.pc_only_note')}</p>
        </div>
    );
}
