import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import { AgentLogo } from '../AgentLogo';
import { Icon } from '../Icon';
import { failed } from '../../utils/apiResult';

export const agentLinkAvailable = () =>
    typeof window.api?.relayMcpGrant === 'function' && typeof window.api?.relayMcpGrantRevoke === 'function';

/**
 * 원격 MCP 에이전트 연결 (데몬 GET·DELETE /api/relay/mcp/grant).
 * 연결은 이 PC 주인의 디스코드 계정에 묶이고, 그 계정의 모든 PC를 다룬다. 여기서는 목록으로 보고 끊기만 한다.
 * 설정 › 연결에서 PC 카드와 같은 모양의 카드로 보인다. 끊기 버튼은 늘 보이지 않고, 카드 머리를 누르면 그 카드에만 나온다.
 */
export function AgentLink({ className }: { className?: string } = {}) {
    const { t, i18n } = useTranslation('gui');
    // undefined = 아직 모름(못 받았으면 줄을 그리지 않는다)
    const [grants, setGrants] = useState<ApiBody[] | undefined>(undefined);
    // 에이전트에 넣을 주소 (릴레이가 준다)
    const [url, setUrl] = useState('');
    // 끊기 버튼을 펼친 줄
    const [open, setOpen] = useState<number | null>(null);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let alive = true;
        window.api
            .relayMcpGrant()
            .then((res) => {
                // 릴레이는 계정마다 여러 연결을 목록(`grants`)으로 준다
                if (!alive || !Array.isArray(res?.grants)) return;
                setGrants(res.grants);
                setUrl(typeof res.url === 'string' ? res.url : '');
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    const revoke = async (index: number) => {
        setBusy(true);
        try {
            // 그 연결만 끊는다
            const res = await window.api.relayMcpGrantRevoke(String(grants?.[index]?.id));
            if (failed(res)) throw new Error(res.error || 'failed');
            setGrants((prev) => (prev ?? []).filter((_, i) => i !== index));
            setOpen(null);
            safeShowToast(t('discord.agent_revoked'), 'success', 4000);
        } catch (e) {
            safeShowToast(t('discord.agent_revoke_failed', { error: (e as Error).message }), 'error', 5000);
        } finally {
            setBusy(false);
        }
    };

    if (grants === undefined) return null;
    const when = (iso: unknown) => (typeof iso === 'string' && iso ? new Date(iso).toLocaleString(i18n.language) : '-');
    return (
        <section className={clsx('saba-card', className)}>
            <h4 className="saba-card-title">
                <Icon name="plug" size="sm" /> {t('discord.agent_title')}
            </h4>
            <p className="setting-description">{grants.length === 0 ? t('discord.agent_none', { url }) : t('discord.agent_list_desc')}</p>
            {grants.length > 0 && (
                <ul className="remote-pc-cards" aria-label={t('discord.agent_title')}>
                    {grants.map((grant, i) => {
                        const expanded = open === i;
                        return (
                            <li key={String(grant.id ?? `${grant.clientName}-${grant.createdAt}`)} className="remote-pc-card agent-card">
                                <button
                                    type="button"
                                    className="remote-pc-top agent-card-head"
                                    aria-expanded={expanded}
                                    onClick={() => setOpen(expanded ? null : i)}
                                >
                                    <span className="remote-pc-os">
                                        <AgentLogo name={grant.clientName} />
                                    </span>
                                    <span className="remote-pc-who">
                                        <b className="remote-pc-name">{grant.clientName || '-'}</b>
                                        <span className="remote-pc-sub">MCP</span>
                                    </span>
                                    <Icon name={expanded ? 'chevronUp' : 'chevronDown'} size="sm" />
                                </button>
                                <dl className="remote-pc-kv">
                                    <dt>{t('discord.agent_created')}</dt>
                                    <dd>{when(grant.createdAt)}</dd>
                                    <dt>{t('discord.agent_last_used')}</dt>
                                    <dd>{when(grant.lastUsedAt)}</dd>
                                </dl>
                                {expanded && (
                                    <div className="remote-pc-acts">
                                        <button type="button" className="btn btn-ghost danger" onClick={() => revoke(i)} disabled={busy}>
                                            <Icon name="cloudOff" size="sm" /> {t('discord.agent_revoke')}
                                        </button>
                                    </div>
                                )}
                            </li>
                        );
                    })}
                </ul>
            )}
        </section>
    );
}
