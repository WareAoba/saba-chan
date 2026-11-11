import clsx from 'clsx';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { portCheckShown } from '../../utils/portCheck';
import type { Server } from '../../stores/useServerStore';
import { Icon } from '../Icon';
import { useLastPortCheck } from './PortCheck';

/** 공인 IP 가리기 (방송 화면용, 이 PC의 이 앱에서만 기억한다) */
const HIDE_KEY = 'saba.hideServerIp';
const readHidden = () => {
    try {
        return localStorage.getItem(HIDE_KEY) === '1';
    } catch {
        return false;
    }
};

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
    const [hidden, setHidden] = useState(readHidden);
    const [copied, setCopied] = useState<string | null>(null);
    const [report] = useLastPortCheck(server.id);

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
    // 정한 공개 주소(도메인)는 가리지 않는다. 공인 IP만 가릴 수 있다
    const maskable = Boolean(host) && !info.custom;
    const masked = maskable && hidden;
    const toggleHidden = () => {
        const next = !hidden;
        setHidden(next);
        try {
            localStorage.setItem(HIDE_KEY, next ? '1' : '0');
        } catch {
            // 기억하지 못해도 지금 화면은 바뀐다
        }
    };
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
                {maskable && (
                    <button type="button" className="btn btn-ghost btn-sm" onClick={toggleHidden}>
                        {masked ? t('server_address.show') : t('server_address.hide')}
                    </button>
                )}
            </div>
            {!host && <p className="muted">{t('server_address.unknown')}</p>}
            {host &&
                ports.map((p) => {
                    const text = hostPort(host, p.port);
                    return (
                        <div key={`${p.field}-${p.port}`} className={clsx('server-address-line', ports.length > 1 && 'many')}>
                            {ports.length > 1 && <span className="server-address-label">{p.label || p.field}</span>}
                            {masked ? (
                                <span className="server-address-value masked" aria-label={t('server_address.hidden')}>
                                    {'•'.repeat(8)}:{p.port}
                                </span>
                            ) : (
                                <span className={clsx('server-address-value', info.running === false && 'dim')}>{text}</span>
                            )}
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => copy(text)}>
                                <Icon name={copied === text ? 'check' : 'copy'} size="sm" />{' '}
                                {copied === text ? t('server_address.copied') : t('server_address.copy')}
                            </button>
                        </div>
                    );
                })}
            <div className="server-address-sub">
                {report && portCheckShown(report) && (
                    <span className={clsx('server-address-check', `verdict-${report.verdict}`)}>
                        {t(`port_check.verdict.${report.verdict}`)}
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
            </div>
        </section>
    );
}
