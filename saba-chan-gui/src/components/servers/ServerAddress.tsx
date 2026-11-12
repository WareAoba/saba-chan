import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { portCheckHintKey, portCheckNeedsAttention, portCheckShown } from '../../utils/portCheck';
import type { Server } from '../../stores/useServerStore';
import { Icon } from '../Icon';
import { PortCheckButton, useLastPortCheck, usePortCheckRun } from './PortCheck';

/** `주소:포트` (IPv6는 `[주소]:포트`) */
export const hostPort = (host: string, port: number) => (host.includes(':') ? `[${host}]:${port}` : `${host}:${port}`);

interface AddressPort {
    field: string;
    label?: string;
    port: number;
}

/** 서버 상세 머리 아래: 친구가 접속할 주소(PC의 공개 주소 + 밖에 여는 포트)를 크게 보이고 복사한다 (사용자 결정 2026-11-10) */
export function ServerAddress({ server }: { server: Server }) {
    const { t, i18n } = useTranslation('gui');
    const [info, setInfo] = useState<ApiBody | null>(null);
    const [copied, setCopied] = useState<string | null>(null);
    const [report, setReport] = useLastPortCheck(server.id);
    const { run, running, error } = usePortCheckRun(server.id, setReport);

    const load = useCallback(async () => {
        if (typeof window.api?.instanceAddress !== 'function') return;
        const res = await window.api.instanceAddress(server.id).catch(() => null);
        setInfo(res && Array.isArray(res.ports) ? res : null);
    }, [server.id]);
    useEffect(() => {
        load();
    }, [load, server.status]);
    useEffect(
        () =>
            subscribeDaemonEvents((ev) => {
                if (ev.type === 'instance.port_check' && ev.data.instanceId === server.id) load();
            }),
        [server.id, load],
    );

    const ports: AddressPort[] = info?.ports || [];
    if (!info || ports.length === 0) return null;
    const host: string | null = info.address || null;
    const copy = async (text: string) => {
        try {
            await navigator.clipboard.writeText(text);
            setCopied(text);
            setTimeout(() => setCopied((c) => (c === text ? null : c)), 1500);
        } catch {
            // 복사하지 못하면 주소를 직접 고를 수 있다
        }
    };
    const checkedAt = report?.checkedAt ? new Date(report.checkedAt) : null;

    return (
        <section data-spot="server.address" className="server-address" aria-label={t('server_address.title')}>
            <div className="server-address-head">
                <span className="server-address-title">{t('server_address.title')}</span>
            </div>
            {!host && <p className="muted">{t('server_address.unknown')}</p>}
            {host &&
                ports.map((p) => {
                    const text = hostPort(host, p.port);
                    return (
                        <div key={`${p.field}-${p.port}`} className={clsx('server-address-line', ports.length > 1 && 'many')}>
                            {ports.length > 1 && <span className="server-address-label">{p.label || p.field}</span>}
                            <span className={clsx('server-address-value', info.running === false && 'dim')}>{text}</span>
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => copy(text)}>
                                <Icon name={copied === text ? 'check' : 'copy'} size="sm" />{' '}
                                {copied === text ? t('server_address.copied') : t('server_address.copy')}
                            </button>
                        </div>
                    );
                })}
            <div className="server-address-sub">
                {/* 포트포워딩 결과는 이 알약 하나로 알린다. 손볼 것이 있으면 고칠 방법을 알약에 띄우고 바로 다시 확인한다 (사용자 11-12) */}
                {report && portCheckShown(report) && (
                    <span
                        className={clsx('server-address-check', `verdict-${report.verdict}`)}
                        data-tooltip={portCheckNeedsAttention(report) ? t(portCheckHintKey(report)) : undefined}
                    >
                        {t(`port_check.pill.${report.verdict}`)}
                    </span>
                )}
                {report && portCheckShown(report) && checkedAt && !Number.isNaN(checkedAt.getTime()) && (
                    <span className="muted">
                        {t('server_address.checked_at', {
                            time: checkedAt.toLocaleString(i18n.language, { dateStyle: 'short', timeStyle: 'short' }),
                        })}
                    </span>
                )}
                {info.running === false && <span className="muted">{t('server_address.stopped')}</span>}
                {portCheckNeedsAttention(report) && (
                    <PortCheckButton run={run} running={running} checked className="btn btn-ghost btn-sm server-address-retry" />
                )}
                {error && <span className="server-address-error">{error}</span>}
            </div>
        </section>
    );
}
