import clsx from 'clsx';
import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import type { ApiBody } from '../../types/daemon';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

export const agentLinkAvailable = () =>
    typeof window.api?.relayMcpGrant === 'function' && typeof window.api?.relayMcpGrantRevoke === 'function';

const failed = (res: ApiBody | null | undefined) => Boolean(res?.error) || res?.success === false;

/** 데몬 응답의 연결 목록. 지금 릴레이는 계정마다 하나(`grant`)만 주고, 목록(`grants`)을 주면 그대로 쓴다 */
function grantsOf(res: ApiBody): ApiBody[] {
    if (Array.isArray(res.grants)) return res.grants;
    return res.grant ? [res.grant] : [];
}

/**
 * 원격 MCP 에이전트 연결 (데몬 GET·DELETE /api/relay/mcp/grant).
 * 연결은 이 PC 주인의 디스코드 계정에 묶이고, 그 계정의 모든 PC를 다룬다. 여기서는 목록으로 보고 끊기만 한다.
 * 끊기 버튼은 늘 보이지 않고, 에이전트 줄을 누르면 그 줄에만 나온다.
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
                if (!alive || !res || !('grant' in res || 'grants' in res)) return;
                setGrants(grantsOf(res));
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
            // 그 연결만 끊는다. 옛 릴레이는 id를 주지 않는다: 그때는 연결이 하나뿐이라 모두 끊어도 같다
            const id = grants?.[index]?.id;
            const res = await window.api.relayMcpGrantRevoke(typeof id === 'string' ? id : undefined);
            if (failed(res)) throw new Error(res.error || 'failed');
            setGrants((prev) => (typeof id === 'string' ? (prev ?? []).filter((_, i) => i !== index) : []));
            setOpen(null);
            safeShowToast(t('discord.agent_revoked'), 'success', 4000);
        } catch (e) {
            safeShowToast(t('discord.agent_revoke_failed', { error: (e as Error).message }), 'error', 5000);
        } finally {
            setBusy(false);
        }
    };

    if (grants === undefined) return null;
    if (grants.length === 0) {
        return (
            <div className={clsx('dc-row-actions', className)}>
                <span className="muted">{t('discord.agent_none', { url })}</span>
            </div>
        );
    }
    return (
        <div className={clsx('dc-row-actions agent-list-box', className)}>
            <span className="muted">{t('discord.agent_list_desc')}</span>
            <ul className="agent-list">
                {grants.map((grant, i) => {
                    const when = grant.lastUsedAt ? new Date(grant.lastUsedAt).toLocaleString(i18n.language) : '-';
                    const expanded = open === i;
                    return (
                        <li key={String(grant.id ?? `${grant.clientName}-${grant.createdAt}`)} className={clsx('agent-item', { open: expanded })}>
                            <button
                                type="button"
                                className="agent-item-head"
                                aria-expanded={expanded}
                                onClick={() => setOpen(expanded ? null : i)}
                            >
                                <Icon name="plug" size="sm" />
                                <span className="agent-item-name">{grant.clientName || '-'}</span>
                                <span className="agent-item-when muted">{t('discord.agent_last_used', { when })}</span>
                                <Icon name={expanded ? 'chevronUp' : 'chevronDown'} size="sm" />
                            </button>
                            {expanded && (
                                <button
                                    type="button"
                                    className="btn btn-ghost btn-danger-text agent-item-revoke"
                                    onClick={() => revoke(i)}
                                    disabled={busy}
                                >
                                    <Icon name="cloudOff" size="sm" /> {t('discord.agent_revoke')}
                                </button>
                            )}
                        </li>
                    );
                })}
            </ul>
        </div>
    );
}
