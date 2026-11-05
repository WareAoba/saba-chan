import { type ReactNode, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';

/** GET /api/diagnostics 응답 (데몬이 싣는 칸) */
interface Diagnostics {
    version?: string;
    uptime_secs?: number;
    pid?: number;
    listen_addr?: string;
    data_dir?: string;
    token_file?: { exists?: boolean };
    modules?: { error?: string; count?: number };
    extensions?: { enabled?: string[]; enabled_but_missing?: string[] };
    recent_problems?: { id: string | number; level: string; timestamp: string | number; message: string }[];
}

export const diagnosticsAvailable = () => typeof window.api?.daemonDiagnostics === 'function';

/**
 * 데몬 진단 정보 (GET /api/diagnostics, Q-2). 문제를 물어볼 때 붙여 넣을 수 있게 통째로 복사한다.
 * 비밀은 데몬이 싣지 않는다(토큰은 파일 경로와 존재 여부만).
 */
export function DiagnosticsPanel() {
    const { t } = useTranslation('gui');
    const [data, setData] = useState<Diagnostics | null>(null);
    const [error, setError] = useState<string | null>(null);
    const [loading, setLoading] = useState(false);
    const [copied, setCopied] = useState(false);

    if (!diagnosticsAvailable()) return null;

    const load = async () => {
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.daemonDiagnostics();
            if (!res || res.error) setError(res?.error || t('diagnostics.failed'));
            else setData(res as Diagnostics);
        } catch (err) {
            setError((err as Error).message);
        } finally {
            setLoading(false);
        }
    };

    const copy = async () => {
        try {
            await navigator.clipboard.writeText(JSON.stringify(data, null, 2));
            setCopied(true);
            setTimeout(() => setCopied(false), 2000);
        } catch (err) {
            setError((err as Error).message);
        }
    };

    const uptime = (secs: unknown) =>
        typeof secs === 'number'
            ? t('diagnostics.uptime_value', { h: Math.floor(secs / 3600), m: Math.floor((secs % 3600) / 60) })
            : '-';

    const missing = data?.extensions?.enabled_but_missing || [];
    const problems = data?.recent_problems || [];
    const rows: [string, ReactNode][] | null = data && [
        [t('diagnostics.version'), data.version],
        [t('diagnostics.uptime'), uptime(data.uptime_secs)],
        ['PID', data.pid],
        [t('diagnostics.listen'), data.listen_addr],
        [t('diagnostics.data_dir'), <code key="d">{data.data_dir}</code>],
        [t('diagnostics.token'), data.token_file?.exists ? t('diagnostics.token_ok') : t('diagnostics.token_missing')],
        [
            t('diagnostics.modules'),
            data.modules?.error ? t('diagnostics.modules_error', { error: data.modules.error }) : data.modules?.count,
        ],
        [t('diagnostics.extensions'), (data.extensions?.enabled || []).join(', ') || '-'],
    ];

    return (
        <div className="diagnostics-panel">
            <div className="diagnostics-actions">
                <button type="button" className="btn btn-ghost" onClick={load} disabled={loading}>
                    <Icon name={loading ? 'refresh' : 'tool'} size="sm" className={loading ? 'spin' : undefined} />{' '}
                    {data ? t('diagnostics.refresh') : t('diagnostics.show')}
                </button>
                {data && (
                    <button type="button" className="btn btn-ghost" onClick={copy}>
                        <Icon name={copied ? 'check' : 'copy'} size="sm" />{' '}
                        {copied ? t('diagnostics.copied') : t('diagnostics.copy')}
                    </button>
                )}
            </div>
            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
            {data && (
                <>
                    <dl className="diagnostics-grid">
                        {rows!.map(([k, v]) => (
                            <div key={k} className="diagnostics-row">
                                <dt>{k}</dt>
                                <dd>{v}</dd>
                            </div>
                        ))}
                    </dl>
                    {missing.length > 0 && (
                        <div className="notice tone-warn">
                            <Icon name="alertTriangle" size="sm" />{' '}
                            {t('diagnostics.extensions_missing', { list: missing.join(', ') })}
                        </div>
                    )}
                    <h4 className="diagnostics-subtitle">{t('diagnostics.recent_problems')}</h4>
                    {problems.length === 0 ? (
                        <p className="diagnostics-empty">{t('diagnostics.no_problems')}</p>
                    ) : (
                        <ul className="diagnostics-problems">
                            {problems.map((p) => (
                                <li key={p.id} className={`level-${String(p.level).toLowerCase()}`}>
                                    <span className="diagnostics-time">
                                        {new Date(p.timestamp).toLocaleTimeString()}
                                    </span>{' '}
                                    {p.message}
                                </li>
                            ))}
                        </ul>
                    )}
                </>
            )}
        </div>
    );
}
