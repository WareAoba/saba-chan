import { type AuditEntry, auditRange, auditReason, auditResult, auditWhat, auditWho } from '@gui/utils/audit';
import { CONSOLE_FONT_CHOICES, CONSOLE_FONT_NAMES, CONSOLE_FONT_SLOTS } from '@gui/utils/consoleFonts';
import { MetricRing, MonitorPicker } from '@gui/components/monitor/MonitorRing';
import {
    buildRings,
    colorSlots,
    defaultItems,
    type MetricsSnapshot,
    nextPeaks,
    type Peaks,
    pickerGroups,
    wantsGpu,
} from '@gui/components/monitor/systemMetrics';
import { LANGUAGE_OPTIONS } from '@gui/utils/languages';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { setLanguage } from '../i18n';
import { applyScreenFonts, applyTheme, readConsoleFonts, readMonitorItems, readPref, THEMES, writeMonitorItems, writePref } from '../prefs';
import { sizeText } from './BackupsPanel';
import { Dropdown } from './Dropdown';
import { startPoll } from './poll';
import { Button, Empty, Loading, Section, Segments } from './ui';
import { useStickToBottom } from './useStickToBottom';
import type { ApiBody, WebApi } from '../types';

/** 서버 한 대의 자원 사용 (GET /api/system/metrics 의 instances) */
interface MetricInstance {
    id: string;
    name: string;
    running: boolean;
    cpu_percent: number;
    memory_bytes: number;
}

/** 데몬 콘솔 한 줄 */
interface DaemonLine {
    id: number;
    level?: string;
    message: string;
}

/** 활동 기록 한 건 (audit::Event) */
type ActivityEntry = AuditEntry & { ts: number; instance?: string; instanceId?: string };

/** PC 상태: 앱 시스템 모니터와 같은 원과 항목 고르기. 고른 항목은 이 브라우저에 둔다 (앱 설정은 웹에서 읽지 않는다) */
function Metrics({ api }: { api: WebApi }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [state, setState] = useState<{ snap: MetricsSnapshot | null; peaks: Peaks }>({
        snap: null,
        peaks: { diskIo: 0, network: 0 },
    });
    const [picked, setPicked] = useState<string[] | null>(readMonitorItems);
    const [picking, setPicking] = useState(false);
    // GPU는 고른 때만 묻는다 (데몬이 NVIDIA 라이브러리를 올린다)
    const gpu = wantsGpu(picked ?? []);
    useEffect(() => {
        let stopped = false;
        const load = async () => {
            const res = (await api.systemMetrics({ sizes: true, gpu })) as MetricsSnapshot | null;
            if (!stopped && res?.cpu) setState((prev) => ({ snap: res, peaks: nextPeaks(prev.peaks, res) }));
        };
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api, gpu]);

    const { snap, peaks } = state;
    if (!snap) return <Loading />;
    const pick = (next: string[] | null) => {
        setPicked(next);
        writeMonitorItems(next);
    };
    const instances = (snap.instances || []) as MetricInstance[];
    const slots = colorSlots(instances.map((i) => i.id));
    const rings = buildRings(snap, slots, peaks, picked ?? defaultItems(snap));
    const running = instances.filter((i) => i.running);
    return (
        <>
            <div className="metrics-head">
                <Button onClick={() => setPicking((v) => !v)} aria-expanded={picking}>
                    {tg('monitor.pick')}
                </Button>
                {picking && picked && (
                    <Button onClick={() => pick(null)}>
                        {tg('monitor.pick_reset')}
                    </Button>
                )}
            </div>
            {picking && (
                <div className="metrics-picker">
                    <MonitorPicker groups={pickerGroups(snap)} items={picked ?? defaultItems(snap)} onChange={pick} t={tg} />
                </div>
            )}
            {rings.length === 0 ? (
                <Empty>{tg('monitor.nothing_picked')}</Empty>
            ) : (
                <div className="metrics-rings">
                    {rings.map((r) => (
                        <MetricRing key={r.key} ring={r} t={tg} size={96} />
                    ))}
                </div>
            )}
            <h3 className="sub-title">{t('more.servers_usage')}</h3>
            {running.length === 0 ? (
                <Empty>{t('more.none_running')}</Empty>
            ) : (
                <ul className="rows">
                    {running.map((i) => (
                        <li key={i.id} className="row">
                            <span
                                className="metrics-swatch"
                                style={{ background: slots[i.id] ? `var(--series-${slots[i.id]})` : 'var(--text-2)' }}
                                aria-hidden="true"
                            />
                            <div className="row-main">
                                <div className="row-title">{i.name}</div>
                                <div className="row-sub">
                                    CPU {Math.round(i.cpu_percent)}% · {sizeText(i.memory_bytes)}
                                </div>
                            </div>
                        </li>
                    ))}
                </ul>
            )}
        </>
    );
}

function DaemonLog({ api }: { api: WebApi }) {
    const { t } = useTranslation();
    const [lines, setLines] = useState<DaemonLine[] | null>(null);
    const next = useRef<number | undefined>(undefined);
    const box = useStickToBottom<HTMLPreElement>(lines);
    useEffect(() => {
        let stopped = false;
        const load = async () => {
            const res = await api.daemonConsole(next.current === undefined ? {} : { since: next.current });
            if (stopped || !Array.isArray(res?.lines)) return;
            if (res.lines.length) next.current = res.lines[res.lines.length - 1].id + 1;
            // 새 줄이 없으면 그대로 둔다
            setLines((prev) => (prev && !res.lines.length ? prev : [...(prev ?? []), ...res.lines].slice(-300)));
        };
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
        };
    }, [api]);

    if (lines === null) return <Loading />;
    return (
        <pre className="console-lines console-tall" ref={box.ref} onScroll={box.onScroll}>
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
const TONE: Record<string, string> = { ok: 'pill-ok', idle: '', stop: 'pill-stop' };

/** 활동 기록 (GET /api/audit, 최신이 먼저). 데몬이 웹에는 PC 경로를 가려 준다. 문구는 앱 작업 기록과 같다 */
function ActivityLog({ api }: { api: WebApi }) {
    const { t, i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [period, setPeriod] = useState('all');
    const [limit, setLimit] = useState(PAGE);
    const [entries, setEntries] = useState<ActivityEntry[] | null>(null);
    const [error, setError] = useState<string | null>(null);
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

    const fmt = (s: number) =>
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
                        const reason = auditReason(e, tg);
                        return (
                            // biome-ignore lint/suspicious/noArrayIndexKey: 같은 시각의 기록이 여러 개일 수 있어 순번을 붙인다
                            <li key={`${e.ts}-${i}`} className="row activity-row">
                                <span className="activity-time">{fmt(e.ts)}</span>
                                <div className="row-main">
                                    <div className="row-title">
                                        {(e.instance || e.instanceId) && <strong>{e.instance || e.instanceId} </strong>}
                                        {auditWhat(e, tg)}
                                    </div>
                                    <div className="row-sub">{auditWho(e, tg)}</div>
                                    {reason && <div className="row-sub">{reason}</div>}
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

/** 이 화면 설정: 언어·테마·콘솔 글꼴은 이 브라우저에만 기억한다 (PC의 앱 설정은 바꾸지 않는다) */
function ScreenSettings() {
    const { t, i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [theme, setTheme] = useState(() => readPref('theme') || 'system');
    const pickTheme = (value: string) => {
        setTheme(value);
        writePref('theme', value);
        applyTheme(value);
    };
    const [fonts, setFonts] = useState(readConsoleFonts);
    const pickFont = (slot: (typeof CONSOLE_FONT_SLOTS)[number], value: string) => {
        const next = { ...fonts, [slot]: value };
        setFonts(next);
        applyScreenFonts(next, i18n.language);
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
            {/* 콘솔 글꼴: 글자마다 하나 (앱 설정과 같은 후보) */}
            <div className="field">
                <span>{tg('settings_modal.console_fonts_label')}</span>
                <span className="hint">{tg('settings_modal.console_fonts_description')}</span>
            </div>
            {CONSOLE_FONT_SLOTS.map((slot) => (
                <div className="field" key={slot}>
                    <span>{tg(`settings_modal.console_fonts_${slot}`)}</span>
                    <Dropdown
                        value={fonts[slot]}
                        onChange={(value) => pickFont(slot, value)}
                        options={CONSOLE_FONT_CHOICES[slot].map((value) => ({
                            value,
                            label: CONSOLE_FONT_NAMES[value as keyof typeof CONSOLE_FONT_NAMES],
                        }))}
                        ariaLabel={tg(`settings_modal.console_fonts_${slot}`)}
                    />
                </div>
            ))}
            <p className="hint">{t('more.screen_note')}</p>
        </>
    );
}

const VIEWS: Record<string, (props: { api: WebApi }) => React.JSX.Element> = {
    metrics: Metrics,
    activity: ActivityLog,
    log: DaemonLog,
    screen: ScreenSettings,
};

export function MoreTab({ api }: { api: WebApi }) {
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
