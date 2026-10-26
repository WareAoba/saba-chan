import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { portCheckNeedsAttention, portCheckShown, portHelpUrl, portLabel } from '../../utils/portCheck';
import { Icon } from '../Icon';

/** 데몬에 포트포워딩 확인이 있을 때만 (예전 브리지면 숨긴다) */
export const portCheckAvailable = () => typeof window.api?.instancePortCheck === 'function';

const STATUS_ICON = { open: 'checkCircle', closed: 'alertCircle', unknown: 'info', in_use: 'alertTriangle' };

/** 마지막 확인 결과. 자동 확인이 끝나면(`instance.port_check`) 다시 읽는다 */
function useLastPortCheck(serverId) {
    const [report, setReport] = useState(null);
    const reload = useCallback(async () => {
        if (typeof window.api?.instancePortCheckLast !== 'function') return;
        const res = await window.api.instancePortCheckLast(serverId);
        if (res?.verdict) setReport(res);
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
    const open = (e) => {
        const openExternal = window.electron?.shell?.openExternal;
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

/** 정보 탭: 확인 버튼과 결과 (공인 주소, 포트마다 상태, 닫혔으면 안내) */
export function PortCheck({ server }) {
    const { t } = useTranslation('gui');
    const [report, setReport] = useLastPortCheck(server.id);
    const [running, setRunning] = useState(false);
    const [error, setError] = useState(null);

    if (!portCheckAvailable()) return null;

    const run = async () => {
        setRunning(true);
        setError(null);
        try {
            const res = await window.api.instancePortCheck(server.id);
            if (res?.verdict) setReport(res);
            else setError(t(`port_check.error.${res?.error_code}`, { defaultValue: res?.error || t('port_check.error.failed') }));
        } catch (err) {
            setError(err.message);
        } finally {
            setRunning(false);
        }
    };

    return (
        <section className="server-check port-check">
            <div className="server-check-actions">
                <button type="button" className="btn btn-ghost" disabled={running} onClick={run}>
                    <Icon name={running ? 'refresh' : 'globe'} size="sm" className={running ? 'spin' : undefined} />{' '}
                    {running ? t('port_check.running') : t('port_check.button')}
                </button>
            </div>
            {error && (
                <div className="notice tone-stop" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
            {report?.verdict === 'no_ports' && <p className="muted">{t('port_check.verdict.no_ports')}</p>}
            {portCheckShown(report) && (
                <div className="port-check-result">
                    <p className={clsx('port-check-verdict', `verdict-${report.verdict}`)}>
                        <Icon name={report.verdict === 'open' ? 'checkCircle' : 'alertTriangle'} size="sm" />{' '}
                        {t(`port_check.verdict.${report.verdict}`)}
                    </p>
                    {report.address && (
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
                            {t(`port_check.hint.${report.verdict}`)} <HelpLink />
                        </p>
                    )}
                </div>
            )}
        </section>
    );
}

/** 상세 머리 아래: 자동 확인에서 닫혔다고 나왔으면 조용히 알린다 */
export function PortCheckNotice({ server }) {
    const { t } = useTranslation('gui');
    const [report] = useLastPortCheck(server.id);
    if (!portCheckAvailable() || !portCheckNeedsAttention(report)) return null;
    return (
        <div className="notice tone-warn" role="status">
            <Icon name="globe" size="sm" />
            <span>
                <strong>{t(`port_check.verdict.${report.verdict}`)}</strong> {t(`port_check.hint.${report.verdict}`)}{' '}
                <HelpLink />
            </span>
        </div>
    );
}
