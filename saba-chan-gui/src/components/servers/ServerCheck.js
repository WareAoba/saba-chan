import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useServerStore } from '../../stores/useServerStore';
import { moduleSupports } from '../../utils/helpers';
import { Icon } from '../Icon';

/** 데몬에 점검 채널이 있을 때만 (예전 preload면 버튼을 숨긴다) */
export const checkAvailable = () =>
    typeof window.api?.instanceValidate === 'function' || typeof window.api?.instanceDiagnose === 'function';

const SEVERITY_ORDER = { critical: 0, error: 1, warning: 2, info: 3 };
const SEVERITY_ICON = { critical: 'alertCircle', error: 'alertCircle', warning: 'alertTriangle', info: 'info' };

const sortIssues = (issues) =>
    [...issues].sort((a, b) => (SEVERITY_ORDER[a.severity] ?? 9) - (SEVERITY_ORDER[b.severity] ?? 9));

/**
 * 서버 점검: 모듈 validate(켜기 전 점검)와 diagnose(최근 로그 진단)를 직접 돌려 본다 (Q-2).
 * 문구는 모듈이 현지화해서 준다.
 */
export function ServerCheck({ server }) {
    const { t } = useTranslation('gui');
    const [running, setRunning] = useState(null); // 'validate' | 'diagnose'
    const [result, setResult] = useState(null); // {kind, issues} | {kind, error}
    // 모듈이 구현한 점검만 (MK1)
    const module = useServerStore((s) => s.modules.find((m) => m.name === server.module));
    const canValidate = typeof window.api?.instanceValidate === 'function' && moduleSupports(module, 'validate');
    const canDiagnose = typeof window.api?.instanceDiagnose === 'function' && moduleSupports(module, 'diagnose_log');

    // biome-ignore lint/correctness/useExhaustiveDependencies: 다른 서버로 가면 결과를 지운다
    useEffect(() => {
        setResult(null);
    }, [server.id]);

    if (!canValidate && !canDiagnose) return null;

    const run = async (kind) => {
        const call = kind === 'validate' ? window.api.instanceValidate : window.api.instanceDiagnose;
        setRunning(kind);
        try {
            const res = await call(server.id);
            if (res?.error_code === 'not_supported') setResult({ kind, error: t('errors.not_supported') });
            else if (res?.error && !Array.isArray(res?.issues)) setResult({ kind, error: res.error });
            else setResult({ kind, issues: sortIssues(res?.issues || []) });
        } catch (err) {
            setResult({ kind, error: err.message });
        } finally {
            setRunning(null);
        }
    };

    return (
        <section className="server-check">
            <div className="server-check-actions">
                {canValidate && (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={!!running}
                        onClick={() => run('validate')}
                    >
                        <Icon
                            name={running === 'validate' ? 'refresh' : 'checkCircle'}
                            size="sm"
                            className={running === 'validate' ? 'spin' : undefined}
                        />{' '}
                        {t('shell.check_validate')}
                    </button>
                )}
                {canDiagnose && (
                    <button
                        type="button"
                        className="btn btn-ghost"
                        disabled={!!running}
                        onClick={() => run('diagnose')}
                    >
                        <Icon
                            name={running === 'diagnose' ? 'refresh' : 'tool'}
                            size="sm"
                            className={running === 'diagnose' ? 'spin' : undefined}
                        />{' '}
                        {t('shell.check_diagnose')}
                    </button>
                )}
            </div>
            {result?.error && (
                <div className="notice tone-stop" role="alert">
                    <Icon name="alertCircle" size="sm" /> {t('shell.check_failed', { error: result.error })}
                </div>
            )}
            {result?.issues && result.issues.length === 0 && (
                <p className="server-check-ok">
                    <Icon name="checkCircle" size="sm" />{' '}
                    {result.kind === 'validate' ? t('shell.check_ok') : t('shell.check_log_ok')}
                </p>
            )}
            {result?.issues?.length > 0 && (
                <ul className="server-check-issues">
                    {result.issues.map((issue, i) => (
                        <li key={`${issue.code}-${i}`} className={clsx('server-check-issue', `sev-${issue.severity}`)}>
                            <Icon name={SEVERITY_ICON[issue.severity] || 'info'} size="sm" />
                            <div>
                                <p className="server-check-message">{issue.message || issue.code}</p>
                                {issue.solution && <p className="server-check-solution">{issue.solution}</p>}
                            </div>
                        </li>
                    ))}
                </ul>
            )}
        </section>
    );
}
