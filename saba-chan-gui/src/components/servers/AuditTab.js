import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { SabaSpinner } from '../ui/SabaUI';
import { StatusChip } from '../ui/StatusChip';

/** 이 화면을 띄울 IPC가 있는지 (로드맵 쪽 preload가 먼저 들어와야 한다) */
export const auditAvailable = () => !!window.api?.auditList;

const PAGE = 100;
const MAX = 1000; // 데몬 상한
const SOURCES = ['gui', 'cli', 'api', 'discord', 'schedule', 'daemon', 'web', 'remote', 'restart'];
const ACTIONS = ['start', 'stop', 'restart', 'command', 'stdin', 'backup', 'remote'];

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
 * 없으면 모든 서버를 보이고 서버 이름도 적는다.
 */
export function AuditLog({ instanceId }) {
    const { t, i18n } = useTranslation('gui');
    const [entries, setEntries] = useState(null);
    const [limit, setLimit] = useState(PAGE);
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState(null);
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
            const res = await window.api.auditList({ limit, ...(instanceId ? { instance: instanceId } : {}) });
            if (!current()) return;
            if (!Array.isArray(res?.entries)) throw new Error(res?.error || t('audit.load_failed'));
            setEntries(res.entries);
        } catch (err) {
            if (current()) setError(err.message || String(err));
        } finally {
            if (current()) setLoading(false);
        }
    }, [instanceId, limit, t]);

    useEffect(() => {
        load();
    }, [load]);

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
        <section className="dc-card" aria-label={t('audit.title')}>
            <div className="dc-card-head">
                <h2>{t('audit.title')}</h2>
                <button type="button" className="btn btn-ghost btn-sm" onClick={load} disabled={loading}>
                    {loading ? <SabaSpinner size="sm" /> : <Icon name="refresh" size="sm" />} {t('audit.refresh')}
                </button>
            </div>
            <p className="dc-card-desc">{instanceId ? t('audit.desc_server') : t('audit.desc_all')}</p>
            {error && (
                <div className="notice tone-warn" role="alert">
                    <Icon name="alertTriangle" size="sm" />
                    <span>{error}</span>
                </div>
            )}
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
