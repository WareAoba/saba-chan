import clsx from 'clsx';
import type { ComponentProps, ReactNode } from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import {
    type AuditEntry,
    auditDetails,
    auditMatches,
    auditRange,
    auditReason,
    auditResult,
    auditWhat,
    auditWho,
    FILTERS,
    isInstanceEntry,
    PERIODS,
} from '../../utils/audit';
import { safeShowToast } from '../../utils/helpers';
import { gameNameOf, initials } from '../../utils/serverView';
import { Icon } from '../Icon';
import { DateField } from '../ui/DateTimePicker';
import { Popover } from '../ui/Popover';
import { SabaCheckbox, SabaSpinner } from '../ui/SabaUI';
import { Segmented, StatusChip } from '../ui/StatusChip';

export { auditRange, auditResult, auditWhat, auditWho };

/** 이 화면을 띄울 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const auditAvailable = () => !!window.api?.auditList;

/** 감사 로그 한 줄: 서버를 가리키는 칸은 데몬 버전에 따라 instance 또는 instanceId */
interface AuditRow extends AuditEntry {
    ts: number;
    instance?: string;
    instanceId?: string;
}

const PAGE = 100;
const MAX = 1000; // 데몬 상한
const EXPORT_ERRORS = ['INVALID_PATH', 'WRITE_FAILED']; // data_transfer.err_* 문구를 같이 쓴다

/**
 * 감사 로그 (데몬 GET /api/audit, 최신이 먼저). instanceId가 있으면 그 서버 것만,
 * 없으면 모든 서버를 보이고 서버 이름도 적는다. 머리(새로고침·기간·거르기·내보내기)는 스크롤 영역 밖에 두고,
 * 목록 끝까지 내리면 다음 쪽을 저절로 불러온다. 거르기는 받은 기록 안에서 한다. 내보내기는 보고 있는 기간·서버 그대로 데몬이 CSV로 쓴다.
 */
export function AuditLog({ instanceId }: { instanceId?: string }) {
    const { t, i18n } = useTranslation('gui');
    const [entries, setEntries] = useState<AuditRow[] | null>(null);
    const [rawCount, setRawCount] = useState(0); // 숨긴 줄까지 센 받은 건수 (더 보기 판단)
    const [limit, setLimit] = useState(PAGE);
    // 다른 서버를 고르면 처음 쪽부터 다시
    // biome-ignore lint/correctness/useExhaustiveDependencies: instanceId가 바뀔 때만 되돌린다
    useEffect(() => setLimit(PAGE), [instanceId]);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const [period, setPeriod] = useState('all');
    const [filters, setFilters] = useState<string[]>([]);
    const [open, setOpen] = useState<Set<string>>(() => new Set());
    const toggleRow = (key: string) =>
        setOpen((prev) => {
            const next = new Set(prev);
            if (!next.delete(key)) next.add(key);
            return next;
        });
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
            setRawCount(res.entries.length);
            setEntries(res.entries.filter(isInstanceEntry));
        } catch (err) {
            if (current()) setError((err as Error).message || String(err));
        } finally {
            if (current()) setLoading(false);
        }
    }, [instanceId, limit, rangeKey, t]);

    useEffect(() => {
        load();
    }, [load]);

    // 기간을 바꾸면 처음 쪽부터 다시
    const changePeriod = (next: string) => {
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
                const known = EXPORT_ERRORS.includes(res?.error_code as string);
                const detail = known ? ` ${t(`data_transfer.err_${res.error_code?.toLowerCase()}`)}` : '';
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

    const fmt = (s: number) =>
        new Date(s * 1000).toLocaleString(i18n.language, {
            month: 'numeric',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
            second: '2-digit',
        });
    // 받은 건수가 요청보다 적으면 더 없다
    const more = !!entries && rawCount >= limit && limit < MAX;
    const shown = entries?.filter((e) => auditMatches(e, filters)) ?? null;

    // 목록 끝 표시가 보이면 다음 쪽을 불러온다 (거른 결과가 짧아 끝이 계속 보이면 상한까지 이어서)
    const scrollRef = useRef<HTMLDivElement>(null);
    const endRef = useRef<HTMLDivElement>(null);
    useEffect(() => {
        const end = endRef.current;
        if (!end || !more || loading || typeof IntersectionObserver === 'undefined') return;
        const io = new IntersectionObserver(
            (seen) => {
                if (seen.some((x) => x.isIntersecting)) setLimit((l) => Math.min(l + PAGE, MAX));
            },
            { root: scrollRef.current, rootMargin: '120px' },
        );
        io.observe(end);
        return () => io.disconnect();
    }, [more, loading]);

    return (
        <section className="dc-card audit-card" aria-label={t('audit.title')}>
            <div className="dc-card-head">
                <h2>{t('audit.title')}</h2>
                <AuditFilter value={filters} onChange={setFilters} />
                {typeof window.api?.auditExport === 'function' && (
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
            <div className="audit-scroll" ref={scrollRef}>
                {shown === null ? (
                    !error && <SabaSpinner />
                ) : shown.length === 0 && !more ? (
                    <p className="muted">{t('audit.empty')}</p>
                ) : (
                    <ul className="dc-list audit-list">
                        {shown.map((e, i) => {
                            const key = `${e.ts}-${e.action}-${e.instanceId}-${i}`;
                            const expanded = open.has(key);
                            return (
                                <li key={key} className={clsx('audit-item', { expanded })}>
                                    <button
                                        type="button"
                                        className="dc-list-row audit-row"
                                        aria-expanded={expanded}
                                        onClick={() => toggleRow(key)}
                                    >
                                        <span className="audit-time">{fmt(e.ts)}</span>
                                        <span className="dc-list-text">
                                            <span className="dc-list-name">
                                                {!instanceId && <strong>{e.instance || e.instanceId} </strong>}
                                                {auditWhat(e, t)}
                                            </span>
                                            <span className="dc-list-sub">{auditWho(e, t)}</span>
                                            {auditReason(e, t) && (
                                                <span className="dc-list-sub audit-reason">{auditReason(e, t)}</span>
                                            )}
                                        </span>
                                        <StatusChip {...(auditResult(e, t) as ComponentProps<typeof StatusChip>)} />
                                        <Icon name={expanded ? 'chevronUp' : 'chevronDown'} size="sm" />
                                    </button>
                                    {expanded && (
                                        <dl className="audit-detail">
                                            {auditDetails(e, t, i18n.language).map(([label, value], j) => (
                                                // biome-ignore lint/suspicious/noArrayIndexKey: 같은 이름(인자)이 여러 줄일 수 있다
                                                <div key={j}>
                                                    <dt>{label}</dt>
                                                    <dd>{value}</dd>
                                                </div>
                                            ))}
                                        </dl>
                                    )}
                                </li>
                            );
                        })}
                    </ul>
                )}
                {more && (
                    <div ref={endRef} className="audit-end">
                        {loading && <SabaSpinner size="sm" />}
                    </div>
                )}
            </div>
        </section>
    );
}

/** 거르기 단추: 깔때기 아이콘을 누르면 항목을 체크해 고른다 (여럿 고르면 하나라도 드는 기록). 고른 수를 단추에 적는다 */
function AuditFilter({ value, onChange }: { value: string[]; onChange: (next: string[]) => void }) {
    const { t } = useTranslation('gui');
    const [open, setOpen] = useState(false);
    const toggle = (id: string, on: boolean) => onChange(on ? [...value, id] : value.filter((v) => v !== id));
    return (
        <Popover
            open={open}
            onOpenChange={setOpen}
            trigger={
                <button
                    type="button"
                    className={clsx('btn btn-ghost btn-sm audit-filter-btn', { active: value.length > 0 })}
                >
                    <Icon name="filter" size="sm" /> {t('audit.filter')}
                    {value.length > 0 && <span className="audit-filter-count">{value.length}</span>}
                </button>
            }
            label={t('audit.filter')}
            className="audit-filter-panel"
            align="start"
            sideOffset={4}
        >
            {FILTERS.map((id) => (
                <SabaCheckbox key={id} checked={value.includes(id)} onChange={(on) => toggle(id, on)}>
                    {t(`audit.filter_${id}`)}
                </SabaCheckbox>
            ))}
            <button
                type="button"
                className="btn btn-ghost btn-sm"
                onClick={() => onChange([])}
                disabled={value.length === 0}
            >
                {t('audit.filter_all')}
            </button>
        </Popover>
    );
}

/**
 * 상단 탭의 작업 기록 페이지: 왼쪽에 "모든 서버"와 서버(인스턴스) 목록, 오른쪽에 고른 서버의 기록.
 * 지운 서버의 기록은 "모든 서버"에서만 보인다.
 */
export function ActivityPage() {
    const { t } = useTranslation('gui');
    const servers = useServerStore((s) => s.servers);
    const modules = useServerStore((s) => s.modules);
    const [selected, setSelected] = useState<string | null>(null);
    const current = selected && servers.some((s) => s.id === selected) ? selected : null;

    const row = (key: string, active: boolean, onClick: () => void, icon: ReactNode, label: string) => (
        <button
            key={key}
            type="button"
            className={clsx('server-row dc-nav-row', { selected: active })}
            onClick={onClick}
            aria-current={active ? 'page' : undefined}
        >
            <span className="server-row-icon">{icon}</span>
            <span className="server-row-text">
                <span className="server-row-name">{label}</span>
            </span>
        </button>
    );

    return (
        <div className="page servers-page activity-split">
            <aside className="server-list-pane dc-nav" aria-label={t('audit.nav_label')}>
                <div className="server-list dc-nav-list">
                    {row(
                        'all',
                        !current,
                        () => setSelected(null),
                        <Icon name="server" size="sm" />,
                        t('audit.nav_all'),
                    )}
                    {servers.map((s) => {
                        const mod = modules.find((m) => m.name === s.module);
                        return row(
                            s.id,
                            current === s.id,
                            () => setSelected(s.id),
                            mod?.icon ? (
                                <img src={mod.icon} alt="" />
                            ) : (
                                <span aria-hidden="true">{initials(gameNameOf(t, s.module, mod))}</span>
                            ),
                            s.name,
                        );
                    })}
                </div>
            </aside>
            <main className="server-detail-pane activity-pane">
                <div className="server-detail activity-page">
                    <AuditLog instanceId={current ?? undefined} />
                </div>
            </main>
        </div>
    );
}
