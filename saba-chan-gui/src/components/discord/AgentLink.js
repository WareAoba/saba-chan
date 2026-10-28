import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

/** 에이전트에 넣는 원격 MCP 주소 (릴레이 `<issuer>/mcp`) */
const MCP_URL = 'https://saba-chan.online/mcp';

export const agentLinkAvailable = () =>
    typeof window.api?.relayMcpGrant === 'function' && typeof window.api?.relayMcpGrantRevoke === 'function';

const failed = (res) => Boolean(res?.error) || res?.success === false;

/**
 * 원격 MCP 에이전트 연결 (데몬 GET·DELETE /api/relay/mcp/grant).
 * 연결은 이 PC 주인의 디스코드 계정마다 하나이고, 그 계정의 모든 PC를 다룬다. 여기서는 보고 끊기만 한다.
 */
export function AgentLink() {
    const { t, i18n } = useTranslation('gui');
    // undefined = 아직 모름(못 받았으면 줄을 그리지 않는다), null = 연결 없음
    const [grant, setGrant] = useState(undefined);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let alive = true;
        window.api
            .relayMcpGrant()
            .then((res) => {
                if (alive && res && 'grant' in res) setGrant(res.grant);
            })
            .catch(() => {});
        return () => {
            alive = false;
        };
    }, []);

    const revoke = async () => {
        setBusy(true);
        try {
            const res = await window.api.relayMcpGrantRevoke();
            if (failed(res)) throw new Error(res.error || 'failed');
            setGrant(null);
            safeShowToast(t('discord.agent_revoked'), 'success', 4000);
        } catch (e) {
            safeShowToast(t('discord.agent_revoke_failed', { error: e.message }), 'error', 5000);
        } finally {
            setBusy(false);
        }
    };

    if (grant === undefined) return null;
    if (!grant) {
        return (
            <div className="dc-row-actions">
                <span className="muted">{t('discord.agent_none', { url: MCP_URL })}</span>
            </div>
        );
    }
    const when = grant.lastUsedAt ? new Date(grant.lastUsedAt).toLocaleString(i18n.language) : '-';
    return (
        <div className="dc-row-actions">
            <span className="muted">{t('discord.agent_linked', { name: grant.clientName || '-', when })}</span>
            <button type="button" className="btn btn-danger-text" onClick={revoke} disabled={busy}>
                <Icon name="cloudOff" size="sm" /> {t('discord.agent_revoke')}
            </button>
        </div>
    );
}
