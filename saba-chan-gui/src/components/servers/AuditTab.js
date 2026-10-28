import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { Segmented, StatusChip } from '../ui/StatusChip';

/** 이 화면을 띄울 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const auditAvailable = () => !!window.api?.auditList;

const PAGE = 100;
const MAX = 1000; // 데몬 상한
const SOURCES = ['gui', 'cli', 'api', 'discord', 'schedule', 'daemon', 'web', 'remote', 'mcp', 'restart'];
const ACTIONS = ['start', 'stop', 'restart', 'command', 'stdin', 'backup', 'remote'];
const EXPORT_ERRORS = ['INVALID_PATH', 'WRITE_FAILED']; // data_transfer.err_* 문구를 같이 쓴다
const PERIODS = ['all', 'today', '7d', '30d', 'custom'];
const DAYS_BACK = { today: 0, '7d': 6, '30d': 29 }; // 오늘을 포함해 며칠

// 이 PC 시간대로 그날 0시 (unix 초)
const dayStart = (date) => Math.floor(new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime() / 1000);
// "2026-10-01" → 그날 0시. 비었거나 틀리면 null
const parseDay = (value, plusDays = 0) => {
    const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value || '');
    return m ? dayStart(new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]) + plusDays)) : null;
};

/** 기간 → 데몬 조회 범위 {since?, until?} (until은 그 시각 전까지) */
export function auditRange(period, from, to) {
    if (period in DAYS_BACK) {
        const now = new Date();
        return { since: dayStart(new Date(now.getFullYear(), now.getMonth(), now.getDate() - DAYS_BACK[period])) };
    }
    if (period !== 'custom') return {};
    const since = parseDay(from);
    const until = parseDay(to, 1); // 끝 날짜는 그날 끝까지
    return { ...(since !== null ? { since } : {}), ...(until !== null ? { until } : {}) };
}

// 데몬은 인자를 JSON 문자열로 남긴다 (잘렸을 수 있다): 배열이면 띄어 쓰고, 아니면 그대로
const argsText = (args) => {
    try {
        const v = JSON.parse(args);
        if (Array.isArray(v)) return v.map((x) => (typeof x === 'object' && x ? JSON.stringify(x) : x)).join(' ');
    } catch {}
    return args;
};

/** 한 건 → "무엇을" 한 줄 (명령이면 명령 이름과 인자, 원격 데몬에 중계한 조작이면 그 데몬과 요청) */
export function auditWhat(e, t) {
    const action = ACTIONS.includes(e.action) ? t(`audit.action_${e.action}`) : e.action;
    const detail = [e.command, argsText(e.args)].filter(Boolean).join(' ');
    return [action, e.daemon, detail].filter(Boolean).join(' · ');
}

/** 한 건 → 결과 칩. 예약 백업은 서버가 꺼져 있으면 409로 건너뛴다 */
export function auditResult(e, t) {
    if (e.ok) return { tone: 'ok', label: t('audit.ok') };
    if (e.action === 'backup' && e.status === 409) return { tone: 'idle', label: t('audit.skipped_stopped') };
    return { tone: 'stop', label: t('audit.failed', { status: e.status }) };
}

/** 한 건 → "누가·어디서" 한 줄 */
export function auditWho(e, t) {
    const source = SOURCES.includes(e.source) ? t(`audit.source_${e.source}`) : e.source;
    return [source, e.userName || e.user, e.guildName || e.guild].filter(Boolean).join(' · ');
}

/**
 * 감사 로그 (데몬 GET /api/audit, 최신이 먼저). instanceId가 있으면 그 서버 것만,
 * 없으면 모든 서버를 보이고 서버 이름도 적는다. 머리(새로고침·기간·내보내기)와 더 보기는
 * 스크롤 영역 밖에 둔다. 내보내기는 보고 있는 기간·서버 그대로 데몬이 CSV로 쓴다.
 */
export function AuditLog({ instanceId }) {
    const { t, i18n } = useTranslation('gui');
    const [entries, setEntries] = useState(null);
    const [limit, setLimit] = useState(PAGE);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(null);
    const [period, setPeriod] = useState('all');
    const [from, setFrom] = useState('');
    const [to, setTo] = useState('');
    const [exporting, setExporting] = useState(false);
    const range = auditRange(period, from, to);
    const rangeKey = JSON.stringify(range);
    const alive = useRef(true);
    const seq = useRef(0); // 늦게 온 이전 응답이 새 응답을 덮지 않게
    useEffect(
        () => () => {
            alive.current = false;
        },
        [],
    );

    const load = useCallback(async () => {
        const mine = ++seq.current;
        const current = () => alive.current && mine === seq.current;
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.auditList({
                limit,
                ...(instanceId ? { instance: instanceId } : {}),
                ...JSON.parse(rangeKey),
            });
            if (!current()) return;
            if (!Array.isArray(res?.entries)) throw new Error(res?.error || t('audit.load_failed'));
            setEntries(res.entries);
        } catch (err) {
            if (current()) setError(err.message || String(err));
        } finally {
            if (current()) setLoading(false);
        }
    }, [instanceId, limit, rangeKey, t]);

    useEffect(() => {
        load();
    }, [load]);

    // 기간을 바꾸면 처음 쪽부터 다시
    const changePeriod = (next) => {
        setPeriod(next);
        setLimit(PAGE);
    };

    const doExport = async () => {
        setExporting(true);
        try {
            const res = await window.api.auditExport({ ...(instanceId ? { instance: instanceId } : {}), ...range });
            if (!alive.current || res?.canceled) return;
            if (!res || res.success === false || res.error || res.error_code) {
                // 데몬 문구는 영문이다: 아는 오류 코드만 번역해 덧붙인다
                const known = EXPORT_ERRORS.includes(res?.error_code);
                const detail = known ? ` ${t(`data_transfer.err_${res.error_code.toLowerCase()}`)}` : '';
                safeShowToast(`${t('audit.export_failed')}${detail}`, 'error', 5000);
                return;
            }
            safeShowToast(t('audit.exported', { count: res.count ?? 0 }), 'success', 4000);
        } catch {
            safeShowToast(t('audit.export_failed'), 'error', 5000);
        } finally {
            if (alive.current) setExporting(false);
        }
    };

    const fmt = (s) =>
        new Date(s * 1000).toLocaleString(i18n.language, {
            month: 'numeric',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
            second: '2-digit',
        });
    // 받은 건수가 요청보다 적으면 더 없다
    const more = entries && entries.length >= limit && limit < MAX;

    return (
        <section className="dc-card audit-card" aria-label={t('audit.title')}>
            <div className="dc-card-head">
                <h2>{t('audit.title')}</h2>
                {window.api?.auditExport && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={doExport} disabled={exporting}>
                        {exporting ? <SabaSpinner size="sm" /> : <Icon name="download" size="sm" />} {t('audit.export')}
                    </button>
                )}
                <button type="button" className="btn btn-ghost btn-sm" onClick={load} disabled={loading}>
                    {loading ? <SabaSpinner size="sm" /> : <Icon name="refresh" size="sm" />} {t('audit.refresh')}
                </button>
            </div>
            <p className="dc-card-desc">{instanceId ? t('audit.desc_server') : t('audit.desc_all')}</p>
            <div className="audit-period">
                <Segmented
                    items={PERIODS.map((id) => ({ id, label: t(`audit.period_${id}`) }))}
                    value={period}
                    onChange={changePeriod}
                    ariaLabel={t('audit.period')}
                />
                {period === 'custom' && (
                    <span className="audit-dates">
                        <input
                            type="date"
                            className="input"
                            aria-label={t('audit.from')}
                            value={from}
                            max={to || undefined}
                            onChange={(e) => {
                                setFrom(e.target.value);
                                setLimit(PAGE);
                            }}
                        />
                        <span aria-hidden="true">~</span>
                        <input
                            type="date"
                            className="input"
                            aria-label={t('audit.to')}
                            value={to}
                            min={from || undefined}
                            onChange={(e) => {
                                setTo(e.target.value);
                                setLimit(PAGE);
                            }}
                        />
                    </span>
                )}
            </div>
            {error && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{error}</span>
                </div>
            )}
            <div className="audit-scroll">
                {entries === null ? (
                    !error && <SabaSpinner />
                ) : entries.length === 0 ? (
                    <p className="muted">{t('audit.empty')}</p>
                ) : (
                    <ul className="dc-list audit-list">
                        {entries.map((e, i) => (
                            <li key={`${e.ts}-${i}`} className="dc-list-row">
                                <span className="audit-time">{fmt(e.ts)}</span>
                                <span className="dc-list-text">
                                    <span className="dc-list-name">
                                        {!instanceId && <strong>{e.instance || e.instanceId} </strong>}
                                        {auditWhat(e, t)}
                                    </span>
                                    <span className="dc-list-sub">{auditWho(e, t)}</span>
                                </span>
                                <StatusChip {...auditResult(e, t)} />
                            </li>
                        ))}
                    </ul>
                )}
            </div>
            {more && (
                <button
                    type="button"
                    className="btn btn-ghost btn-sm audit-more"
                    onClick={() => setLimit((l) => Math.min(l + PAGE, MAX))}
                    disabled={loading}
                >
                    {t('audit.more')}
                </button>
            )}
        </section>
    );
}
