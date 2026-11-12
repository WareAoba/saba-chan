import clsx from 'clsx';
import { type MouseEvent, useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import {
    type PortCheckReport,
    portCheckHintKey,
    portCheckHost,
    portCheckNeedsAttention,
    portCheckShown,
    portHelpUrl,
    portLabel,
} from '../../utils/portCheck';
import type { Server } from '../../stores/useServerStore';
import { Icon } from '../Icon';

/** 데몬에 포트포워딩 확인이 있을 때만 (예전 브리지면 숨긴다) */
export const portCheckAvailable = () => typeof window.api?.instancePortCheck === 'function';

/** 포트 확인 결과 (port_check::Report) */
interface PortCheckResult extends PortCheckReport {
    address?: string;
    ports?: { protocol?: string; port: number; field?: string; status: string }[];
}

type SetReport = (report: PortCheckResult | null) => void;

const STATUS_ICON: Record<string, string> = {
    open: 'checkCircle',
    closed: 'alertCircle',
    unknown: 'info',
    in_use: 'alertTriangle',
    firewall: 'alertCircle',
    router: 'alertCircle',
};

/** 마지막 확인 결과. 자동 확인이 끝나면(`instance.port_check`) 다시 읽는다 (접속 주소 상자도 쓴다) */
export function useLastPortCheck(serverId: string): [PortCheckResult | null, SetReport] {
    const [report, setReport] = useState<PortCheckResult | null>(null);
    const reload = useCallback(async () => {
        if (typeof window.api?.instancePortCheckLast !== 'function') return;
        const res = await window.api.instancePortCheckLast(serverId);
        if (res?.verdict) setReport(res as PortCheckResult);
    }, [serverId]);
    useEffect(() => {
        setReport(null);
        reload();
        return subscribeDaemonEvents((ev) => {
            if (ev.type === 'instance.port_check' && ev.data.instanceId === serverId) reload();
        });
    }, [serverId, reload]);
    return [report, setReport];
}

function HelpLink() {
    const { t, i18n } = useTranslation('gui');
    const url = portHelpUrl(i18n.language);
    const open = (e: MouseEvent<HTMLAnchorElement>) => {
        const openExternal = window.api?.shellOpenExternal;
        if (!openExternal) return;
        e.preventDefault();
        openExternal(url);
    };
    return (
        <a href={url} target="_blank" rel="noreferrer" onClick={open}>
            {t('port_check.help')}
        </a>
    );
}

/** 확인을 실행한다. 결과는 `setReport`로, 실패는 `setError`로 (접속 주소 카드도 쓴다) */
export function usePortCheckRun(serverId: string, setReport: SetReport) {
    const { t } = useTranslation('gui');
    const [running, setRunning] = useState(false);
    const [error, setError] = useState<string | null>(null);
    const run = async () => {
        setRunning(true);
        setError(null);
        try {
            const res = await window.api.instancePortCheck(serverId);
            if (res?.verdict) setReport(res as PortCheckResult);
            else setError(t(`port_check.error.${res?.error_code}`, { defaultValue: res?.error || t('port_check.error.failed') }));
        } catch (err) {
            setError((err as Error).message);
        } finally {
            setRunning(false);
        }
    };
    return { run, running, error };
}

/** 확인 버튼. 결과가 있으면 성공이든 실패든 "다시 확인". 가이드가 가리키는 정보 탭 버튼만 `spot`을 단다 */
export function PortCheckButton({
    run,
    running,
    checked,
    spot = false,
    className = 'btn btn-ghost',
}: {
    run: () => void;
    running: boolean;
    checked: boolean;
    spot?: boolean;
    className?: string;
}) {
    const { t } = useTranslation('gui');
    const label = running ? t('port_check.running') : checked ? t('port_check.retry') : t('port_check.button');
    return (
        <button data-spot={spot ? 'port.check' : undefined} type="button" className={className} disabled={running} onClick={run}>
            <Icon name={running || checked ? 'refresh' : 'globe'} size="sm" className={running ? 'spin' : undefined} /> {label}
        </button>
    );
}

/** 정보 탭: 확인 버튼과 결과 (공인 주소, 포트마다 상태, 닫혔으면 안내) */
export function PortCheck({ server }: { server: Server }) {
    const { t } = useTranslation('gui');
    const [report, setReport] = useLastPortCheck(server.id);
    const { run, running, error } = usePortCheckRun(server.id, setReport);

    if (!portCheckAvailable()) return null;

    return (
        <section className="server-check port-check">
            <div className="server-check-actions">
                <PortCheckButton run={run} running={running} checked={portCheckShown(report)} spot />
            </div>
            {error && (
                <div className="notice tone-stop" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
            {report?.verdict === 'no_ports' && <p className="muted">{t('port_check.verdict.no_ports')}</p>}
            {report && portCheckShown(report) && (
                <div className="port-check-result">
                    <p className={clsx('port-check-verdict', `verdict-${report.verdict}`)}>
                        <Icon name={report.verdict === 'open' ? 'checkCircle' : 'alertTriangle'} size="sm" />{' '}
                        {t(`port_check.verdict.${report.verdict}`)}
                    </p>
                    {portCheckHost(report) && (
                        <p className="muted">{t('port_check.host', { host: portCheckHost(report), address: report.address || '-' })}</p>
                    )}
                    {!portCheckHost(report) && report.address && (
                        <p className="muted">{t('port_check.address', { address: report.address })}</p>
                    )}
                    <ul className="server-check-issues">
                        {(report.ports || []).map((p) => (
                            <li key={`${p.protocol}-${p.port}-${p.field}`} className={clsx('server-check-issue', `port-${p.status}`)}>
                                <Icon name={STATUS_ICON[p.status] || 'info'} size="sm" />
                                <div>
                                    <p className="server-check-message">
                                        {portLabel(p)} · {t(`port_check.status.${p.status}`)}
                                    </p>
                                </div>
                            </li>
                        ))}
                    </ul>
                    {report.verdict !== 'open' && (
                        <p className="server-check-solution">
                            {t(portCheckHintKey(report))} <HelpLink />
                        </p>
                    )}
                </div>
            )}
        </section>
    );
}
