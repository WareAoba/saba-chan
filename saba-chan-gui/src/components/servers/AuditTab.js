import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { auditRange, auditResult, auditWhat, auditWho, PERIODS } from '../../utils/audit';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';
import { DateField } from '../ui/DateTimePicker';
import { SabaSpinner } from '../ui/SabaUI';
import { Segmented, StatusChip } from '../ui/StatusChip';

export { auditRange, auditResult, auditWhat, auditWho };

/** 이 화면을 띄울 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const auditAvailable = () => !!window.api?.auditList;

const PAGE = 100;
const MAX = 1000; // 데몬 상한
const EXPORT_ERRORS = ['INVALID_PATH', 'WRITE_FAILED']; // data_transfer.err_* 문구를 같이 쓴다

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
                        <DateField
                            clearable
                            ariaLabel={t('audit.from')}
                            value={from}
                            max={to || undefined}
                            onChange={(v) => {
                                setFrom(v);
                                setLimit(PAGE);
                            }}
                        />
                        <span aria-hidden="true">~</span>
                        <DateField
                            clearable
                            ariaLabel={t('audit.to')}
                            value={to}
                            min={from || undefined}
                            onChange={(v) => {
                                setTo(v);
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
