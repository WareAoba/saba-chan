import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { ModulesTab } from './components/ModulesTab';
import { MoreTab } from './components/MoreTab';
import { PermissionsTab } from './components/PermissionsTab';
import { ServersTab } from './components/ServersTab';
import { SessionBar } from './components/SessionBar';
import { ConfirmHost, ToastHost } from './components/ui';
import { useApp, useData } from './store';

const TABS = [
    { id: 'servers', icon: '▶' },
    { id: 'permissions', icon: '👥' },
    { id: 'modules', icon: '🧩' },
    { id: 'more', icon: '⋯' },
];

/** 세션이 열린 동안 서버·모듈 목록을 맞춘다 (이벤트가 오면 바로, 아니면 4초마다) */
function useLiveData(api, ready) {
    const busy = useRef(false);
    useEffect(() => {
        if (!api || !ready) return;
        const { setServers, setModules } = useData.getState();
        const loadServers = async () => {
            if (busy.current) return;
            busy.current = true;
            try {
                const res = await api.serverList();
                if (res?.servers) setServers(res.servers);
                else setServers(useData.getState().servers, res?.error || 'error');
            } finally {
                busy.current = false;
            }
        };
        const loadModules = async () => {
            const res = await api.moduleList();
            if (Array.isArray(res?.modules)) setModules(res.modules);
        };
        loadServers();
        loadModules();
        const timer = setInterval(loadServers, 4000);
        const off = api.onDaemonEvent((e) => {
            if (['action.run', 'resync'].includes(e?.type) || String(e?.type).startsWith('instance.')) loadServers();
            if (String(e?.type).startsWith('module.')) loadModules();
        });
        return () => {
            clearInterval(timer);
            off?.();
        };
    }, [api, ready]);
}

function Gate({ phase, reason }) {
    const { t } = useTranslation();
    const closed = phase === 'closed';
    return (
        <main className="gate">
            <div className="gate-card">
                <div className="brand-mark" aria-hidden="true">
                    🐟
                </div>
                <h1>{t('app_name')}</h1>
                {closed ? (
                    <>
                        <p className="gate-text">{t(`gate.${reason}`, { defaultValue: t('gate.closed') })}</p>
                        <p className="gate-hint">{t('gate.again')}</p>
                    </>
                ) : (
                    <p className="gate-text">
                        <span className="spinner" aria-hidden="true" /> {t('gate.connecting')}
                    </p>
                )}
            </div>
        </main>
    );
}

export function App({ api, session }) {
    const { t } = useTranslation();
    const { phase, reason } = useApp((s) => s.session);
    const tab = useApp((s) => s.tab);
    const setTab = useApp((s) => s.setTab);
    const live = phase === 'ready' || phase === 'reconnecting';
    useLiveData(api, live);

    if (!live) return <Gate phase={phase} reason={reason} />;
    return (
        <div className="shell">
            <SessionBar session={session} />
            <nav className="tabs" aria-label={t('tabs.label')}>
                {TABS.map((x) => (
                    <button
                        key={x.id}
                        type="button"
                        className={tab === x.id ? 'on' : ''}
                        aria-current={tab === x.id ? 'page' : undefined}
                        onClick={() => setTab(x.id)}
                    >
                        <span className="tab-icon" aria-hidden="true">
                            {x.icon}
                        </span>
                        <span>{t(`tabs.${x.id}`)}</span>
                    </button>
                ))}
            </nav>
            <main className="content">
                {tab === 'servers' && <ServersTab api={api} />}
                {tab === 'permissions' && <PermissionsTab api={api} />}
                {tab === 'modules' && <ModulesTab api={api} />}
                {tab === 'more' && <MoreTab api={api} />}
            </main>
            <ConfirmHost />
            <ToastHost />
        </div>
    );
}
