import { portCheckHintKey, portCheckHost, portCheckShown, portHelpUrl, portLabel } from '@gui/utils/portCheck';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Button, failed } from './ui';
import type { ApiBody, Server, WebApi } from '../types';

/** 포트 확인 결과 한 줄 (port_check::Report 의 ports) */
interface PortRow {
    protocol?: string;
    port: number;
    field?: string;
    status: string;
}

/** 포트 확인 결과 (port_check::Report) */
interface PortReport {
    verdict: string;
    address?: string;
    ports?: PortRow[];
}

/** 포트포워딩 확인: PC의 데몬이 릴레이에 부탁해 밖에서 게임 포트에 접속해 본다 */
export function PortCheck({ api, server }: { api: WebApi; server: Pick<Server, 'id'> }) {
    const { t, i18n } = useTranslation();
    const [report, setReport] = useState<PortReport | null>(null);
    const [busy, setBusy] = useState(false);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        setReport(null);
        setError(null);
        let live = true;
        api.instancePortCheckLast(server.id).then((res: ApiBody) => {
            if (live && res?.verdict) setReport(res as PortReport);
        });
        return () => {
            live = false;
        };
    }, [api, server.id]);

    const run = async () => {
        setBusy(true);
        setError(null);
        const res = await api.instancePortCheck(server.id);
        setBusy(false);
        if (!failed(res) && res?.verdict) setReport(res as PortReport);
        else setError(t(`port_check.error.${res?.error_code}`, { defaultValue: t('port_check.error.failed') }));
    };

    return (
        <section className="port-check">
            <div className="port-check-head">
                <Button busy={busy} onClick={run}>
                    {busy
                        ? t('port_check.running')
                        : portCheckShown(report)
                          ? t('port_check.retry')
                          : t('port_check.button')}
                </Button>
                {report?.verdict === 'no_ports' && <span className="hint">{t('port_check.verdict.no_ports')}</span>}
            </div>
            {error && <p className="banner banner-error">{error}</p>}
            {report && portCheckShown(report) && (
                <div className={`port-check-result verdict-${report.verdict}`}>
                    <p className="port-check-verdict">{t(`port_check.verdict.${report.verdict}`)}</p>
                    {portCheckHost(report) && (
                        <p className="hint">{t('port_check.host', { host: portCheckHost(report), address: report.address || '-' })}</p>
                    )}
                    {!portCheckHost(report) && report.address && <p className="hint">{t('port_check.address', { address: report.address })}</p>}
                    <ul className="port-check-ports">
                        {(report.ports || []).map((p) => (
                            <li key={`${p.protocol}-${p.port}-${p.field}`} className={`port-${p.status}`}>
                                {portLabel(p)} · {t(`port_check.status.${p.status}`)}
                            </li>
                        ))}
                    </ul>
                    {report.verdict !== 'open' && (
                        <p className="hint">
                            {t(portCheckHintKey(report))}{' '}
                            <a href={portHelpUrl(i18n.language)} target="_blank" rel="noreferrer">
                                {t('port_check.help')}
                            </a>
                        </p>
                    )}
                </div>
            )}
        </section>
    );
}
