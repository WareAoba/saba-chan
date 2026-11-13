import { type AuditEntry, auditRange, auditReason, auditResult, auditWhat, auditWho } from '@gui/utils/audit';
import { CONSOLE_FONT_CHOICES, CONSOLE_FONT_NAMES, CONSOLE_FONT_SLOTS } from '@gui/utils/consoleFonts';
import { type DaemonLogEntry, DaemonLogLine } from '@gui/components/DaemonLogLine';
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
import { startPoll } from '@gui/utils/poll';
import { Icon } from './icons';
import { Button, Card, Empty, Loading, Section, Segments } from './ui';
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
    // 안내를 펼친 원 (한 번에 하나)
    const [note, setNote] = useState<string | null>(null);
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
    const action = (
        <div className="metrics-head">
            {picking && picked && <Button onClick={() => pick(null)}>{tg('monitor.pick_reset')}</Button>}
            <Button onClick={() => setPicking((v) => !v)} aria-expanded={picking}>
                {tg('monitor.pick')}
            </Button>
        </div>
    );
    // 시안 (사용자 2026-11-12): 원·서버별 사용량을 한 카드에, 원마다 칸, 긴 안내는 ⓘ를 눌러 본다
    return (
        <Card icon="monitor" tone="3" title={t('more.metrics')} action={action}>
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
                        <div key={r.key} className="metrics-ring">
                            <MetricRing ring={r} t={tg} size={96} inlineNote={false} />
                            {r.noteKey && (
                                <button
                                    type="button"
                                    className="metrics-note-btn"
                                    aria-expanded={note === r.key}
                                    aria-label={tg(r.noteKey)}
                                    onClick={() => setNote(note === r.key ? null : r.key)}
                                >
                                    <Icon name="info" size={14} />
                                </button>
                            )}
                            {note === r.key && r.noteKey && <p className="hint metrics-note">{tg(r.noteKey)}</p>}
                        </div>
                    ))}
                </div>
            )}
            <div className="metrics-usage">
                <h4 className="sub-title">{t('more.servers_usage')}</h4>
                {running.length === 0 ? (
                    <p className="hint">{t('more.none_running')}</p>
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
            </div>
        </Card>
    );
}

/** 데몬 로그 스트림을 열지 못했거나 끊긴 뒤 다시 열어 보기까지 (ms). 그동안은 주기 조회로 읽는다 */
const STREAM_RETRY_MS = 30_000;

function DaemonLog({ api }: { api: WebApi }) {
    const { t, i18n } = useTranslation();
    const [lines, setLines] = useState<DaemonLogEntry[] | null>(null);
    const next = useRef<number | undefined>(undefined);
    const box = useStickToBottom<HTMLPreElement>(lines);
    useEffect(() => {
        let stopped = false;
        // 데몬 로그 버퍼 세대 (스트림이 받은 자리를 이어 가는 데 쓴다)
        let epoch: number | undefined;
        // 열린 스트림을 닫는 함수. 열려 있는 동안 주기 조회는 쉰다
        let closeStream: (() => void) | null = null;
        let opening = false;
        // 스트림을 열지 못했거나 끊긴 뒤 이때까지는 주기 조회로 읽는다
        let streamRetryAt = 0;
        const add = (got: DaemonLogEntry[], replace: boolean) => {
            if (got.length) next.current = got[got.length - 1].id + 1;
            // 새 줄이 없으면 그대로 둔다
            setLines((prev) =>
                replace ? got.slice(-300) : prev && !got.length ? prev : [...(prev ?? []), ...got].slice(-300),
            );
        };
        /** 받은 다음부터 스트림으로 받는다 (릴레이·PC가 스트림을 모르면 주기 조회를 이어 간다) */
        const openStream = async () => {
            opening = true;
            const close = await Promise.resolve()
                .then(() =>
                    api.daemonConsoleStream(
                        { since: next.current, epoch },
                        (res) => {
                            if (stopped) return;
                            if (typeof res?.epoch === 'number') epoch = res.epoch;
                            if (Array.isArray(res?.lines)) add(res.lines, res.reset === true);
                        },
                        () => {
                            closeStream = null;
                            streamRetryAt = Date.now() + STREAM_RETRY_MS;
                        },
                    ),
                )
                .then(
                    (c) => (typeof c === 'function' ? c : null),
                    () => null,
                );
            opening = false;
            if (stopped || document.hidden) return close?.();
            if (close) closeStream = close;
            else streamRetryAt = Date.now() + STREAM_RETRY_MS;
        };
        const load = async () => {
            if (closeStream || opening) return;
            const res = await api.daemonConsole(next.current === undefined ? {} : { since: next.current });
            if (stopped || !Array.isArray(res?.lines)) return;
            if (typeof res.epoch === 'number') epoch = res.epoch;
            add(res.lines, false);
            if (Date.now() >= streamRetryAt) await openStream();
        };
        // 탭이 가려지면 스트림을 닫는다 (다시 보이면 주기 조회가 읽고 다시 연다)
        const onHidden = () => {
            if (!document.hidden) return;
            closeStream?.();
            closeStream = null;
        };
        document.addEventListener('visibilitychange', onHidden);
        const stop = startPoll(load, 3000);
        return () => {
            stopped = true;
            stop();
            document.removeEventListener('visibilitychange', onHidden);
            closeStream?.();
        };
    }, [api]);

    if (lines === null) return <Loading />;
    return (
        <pre className="console-lines console-tall" ref={box.ref} onScroll={box.onScroll}>
            {lines.length === 0 ? (
                <span className="console-empty">{t('console.empty')}</span>
            ) : (
                lines.map((l) => <DaemonLogLine key={l.id} line={l} locale={i18n.language} />)
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
        <div className="panel side-layout">
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
            <div className="side-main">
                <Section>
                    <View api={api} />
                </Section>
                <p className="hint">{t('more.pc_only_note')}</p>
            </div>
        </div>
    );
}
