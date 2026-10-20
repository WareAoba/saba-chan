import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useDiscordStore } from '../../stores/useDiscordStore';
import { subscribeDaemonEvents } from '../../utils/daemonEvents';
import { safeShowToast } from '../../utils/helpers';
import { Icon } from '../Icon';

/** 디스코드 `/사바쨩 연결`(코드 없이)로 온 연결 요청 (J1-b). 데몬 GET·POST /api/relay/link-requests */
export const linkRequestsAvailable = () =>
    typeof window.api?.relayLinkRequests === 'function' && typeof window.api?.relayLinkRequestRespond === 'function';

/** expiresAt: ISO 문자열, 밀리초, 또는 초(epoch seconds) 어느 것이든 */
const expiryMs = (x) => (typeof x === 'number' && x < 1e12 ? x * 1000 : new Date(x).getTime());
const alive = (r) => !r.expiresAt || !(expiryMs(r.expiresAt) <= Date.now());

/**
 * 대기 중인 요청을 카드로 보이고 수락·거절을 데몬에 보낸다. 앱을 늦게 켰을 때를 위해 처음에 목록을 읽고,
 * 그 뒤로는 데몬 이벤트 `relay.link_request`로 더한다.
 */
export function LinkRequests() {
    const { t } = useTranslation('gui');
    const [requests, setRequests] = useState([]);
    const [busy, setBusy] = useState(null);

    useEffect(() => {
        let mounted = true;
        const add = (list) =>
            setRequests((prev) => {
                const known = new Set(prev.map((r) => r.requestId));
                return [...prev, ...list.filter((r) => r?.requestId && !known.has(r.requestId))].filter(alive);
            });
        window.api
            .relayLinkRequests()
            .then((res) => mounted && add(res?.requests || []))
            .catch(() => {});
        const off = subscribeDaemonEvents((ev) => {
            if (ev.type === 'relay.link_request') add([ev.data]);
        });
        return () => {
            mounted = false;
            off();
        };
    }, []);

    const respond = async (req, accept) => {
        setBusy(req.requestId);
        try {
            const res = await window.api.relayLinkRequestRespond(req.requestId, accept);
            if (res?.ok) {
                if (accept) {
                    safeShowToast(
                        t('link_request.accepted', { guild: res.guildName || req.guildName }),
                        'success',
                        4000,
                    );
                    // 새로 묶인 길드가 디스코드 화면 목록에 보이게
                    useDiscordStore.getState().markCloudNodesStale();
                }
            } else {
                const gone = ['EXPIRED', 'NOT_FOUND'].includes(res?.error);
                safeShowToast(t(gone ? 'link_request.expired' : 'link_request.failed'), 'warning', 4000);
                // 릴레이가 ok:false로 답했으면 요청은 이미 꺼내졌다 (410·404·409). 데몬에 닿지 못했을 때만 남긴다
                if (res?.ok !== false) return;
            }
            setRequests((prev) => prev.filter((r) => r.requestId !== req.requestId));
        } catch {
            safeShowToast(t('link_request.failed'), 'warning', 4000);
        } finally {
            setBusy(null);
        }
    };

    const shown = requests.filter(alive);
    if (shown.length === 0) return null;

    return (
        <div className="link-requests" role="region" aria-label={t('link_request.title')}>
            {shown.map((r) => (
                <div className="link-request" key={r.requestId}>
                    <Icon name="discord" size="sm" />
                    <span className="link-request-text">
                        {t('link_request.message', { guild: r.guildName || r.guildId })}
                        {r.userName && <small>{r.userName}</small>}
                    </span>
                    <button
                        type="button"
                        className="btn btn-ghost btn-sm"
                        disabled={busy === r.requestId}
                        onClick={() => respond(r, false)}
                    >
                        {t('link_request.decline')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={busy === r.requestId}
                        onClick={() => respond(r, true)}
                    >
                        {t('link_request.accept')}
                    </button>
                </div>
            ))}
        </div>
    );
}
