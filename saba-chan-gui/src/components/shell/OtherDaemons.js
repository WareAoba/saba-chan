import clsx from 'clsx';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { safeShowToast } from '../../utils/helpers';
import { STATUS_TONE, primaryAction } from '../../utils/serverView';
import { Icon } from '../Icon';
import { remoteErrorText } from '../Modals/RemoteDaemonSection';
import { StatusChip } from '../ui/StatusChip';
import { reloadWindow } from './DaemonSwitcher';

const REFRESH_MS = 10_000;

/**
 * 합쳐 보기 (G1c, 설정 `gui.daemonView = unified`): 서버 목록 아래에 고른 데몬이 아닌 데몬(이 PC 포함)의 서버를 데몬별로 보인다.
 * 서버는 id로 그 데몬에 바로 켜고 끈다(`remoteInstanceAction`). 설정·콘솔 같은 나머지는 "이 데몬 열기"로 그 데몬을 고른 뒤 쓴다.
 * 이름으로 서버를 찾는 목록(useServerActions)에 섞지 않는다: 데몬이 다르면 이름이 겹칠 수 있다.
 */
export function OtherDaemons({ currentTarget, onSwitched = reloadWindow }) {
    const { t } = useTranslation('gui');
    const [daemons, setDaemons] = useState([]);
    const [busy, setBusy] = useState(null);

    const loading = useRef(false);

    // 닿지 않는 데몬은 답이 30초까지 늦다: 앞 조회가 끝나기 전에는 다시 묻지 않는다
    const load = useCallback(async () => {
        if (loading.current) return;
        loading.current = true;
        try {
            const res = await window.api.remoteServers().catch(() => null);
            if (Array.isArray(res?.daemons)) setDaemons(res.daemons);
        } finally {
            loading.current = false;
        }
    }, []);

    useEffect(() => {
        load();
        const timer = setInterval(load, REFRESH_MS);
        return () => clearInterval(timer);
    }, [load]);

    const toggle = async (hostId, server, action) => {
        setBusy(server.id);
        try {
            const res = await window.api.remoteInstanceAction(hostId, server.id, action);
            if (!res || res.success === false || res.ok === false || res.error) {
                safeShowToast(res?.error || remoteErrorText(res, t), 'error', 5000);
            }
        } finally {
            setBusy(null);
            await load();
        }
    };

    const open = async (hostId) => {
        const res = await window.api.daemonTargetSet(hostId).catch(() => null);
        if (res?.success) onSwitched();
    };

    const others = daemons.filter((d) => (d.hostId || null) !== (currentTarget || null));
    if (others.length === 0) return null;

    return (
        <div className="other-daemons">
            {others.map((d) => {
                const name = d.hostId ? d.label || d.name || d.hostId : t('daemon_switcher.local');
                return (
                    <section key={d.hostId || 'local'} className="other-daemon" aria-label={name}>
                        <header className="other-daemon-head">
                            <Icon name={d.hostId ? 'cloud' : 'desktop'} size="sm" />
                            <span className="other-daemon-name">{name}</span>
                            <button type="button" className="btn btn-ghost btn-sm" onClick={() => open(d.hostId || null)}>
                                {t('other_daemons.open')}
                            </button>
                        </header>
                        {!d.ok && <p className="muted pad">{remoteErrorText({ error_code: d.error_code }, t)}</p>}
                        {d.ok && d.servers.length === 0 && <p className="muted pad">{t('other_daemons.empty')}</p>}
                        {d.servers.map((s) => {
                            const action = busy === s.id ? 'busy' : primaryAction(s.status);
                            const label = t(`shell.status_${s.status}`, { defaultValue: s.status });
                            return (
                                <div key={s.id} className="server-row other-daemon-row" data-status={s.status}>
                                    <div className="server-row-text">
                                        <div className="server-row-name">{s.name}</div>
                                        <div className="server-row-sub">
                                            <span>{s.module}</span>
                                            <span className="sep">·</span>
                                            <StatusChip tone={STATUS_TONE[s.status]} label={label} className="bare" />
                                        </div>
                                    </div>
                                    <button
                                        type="button"
                                        className={clsx('icon-btn', 'server-row-action', `action-${action}`)}
                                        disabled={action === 'busy'}
                                        onClick={() => toggle(d.hostId || null, s, action)}
                                        data-tooltip={action === 'stop' ? t('shell.stop') : action === 'start' ? t('shell.start') : label}
                                        aria-label={`${s.name} ${action === 'stop' ? t('shell.stop') : t('shell.start')}`}
                                    >
                                        {action === 'busy' ? (
                                            <span className="spinner" aria-hidden="true" />
                                        ) : (
                                            <Icon name={action === 'stop' ? 'stop' : 'play'} size="sm" />
                                        )}
                                    </button>
                                </div>
                            );
                        })}
                    </section>
                );
            })}
        </div>
    );
}
