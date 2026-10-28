import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { art } from './art';
import { Icon } from './components/icons';
import { ModulesTab } from './components/ModulesTab';
import { MoreTab } from './components/MoreTab';
import { PermissionsTab } from './components/PermissionsTab';
import { startPoll } from './components/poll';
import { ServersTab } from './components/ServersTab';
import { SessionBar } from './components/SessionBar';
import { ConfirmHost, ToastHost, Wordmark } from './components/ui';
import { registerModuleLocales } from './i18n';
import { useApp, useData } from './store';

const TABS = [
    { id: 'servers', icon: 'servers' },
    { id: 'permissions', icon: 'members' },
    { id: 'modules', icon: 'modules' },
    { id: 'more', icon: 'more' },
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
            if (!Array.isArray(res?.modules)) return;
            setModules(res.modules);
            // 모듈 locales가 붙으면 번역된 게임·설정 이름으로 다시 그린다
            await registerModuleLocales(api, res.modules).catch(() => {});
            setModules([...res.modules]);
        };
        loadModules();
        const stopPoll = startPoll(loadServers, 4000);
        const off = api.onDaemonEvent((e) => {
            if (['action.run', 'resync'].includes(e?.type) || String(e?.type).startsWith('instance.')) loadServers();
            if (String(e?.type).startsWith('module.')) loadModules();
        });
        return () => {
            stopPoll();
            off?.();
        };
    }, [api, ready]);
}

function Gate({ phase, reason }) {
    const { t } = useTranslation();
    const closed = phase === 'closed';
    return (
        // 사이트의 "잘못된 접근" 화면(server-chan website/public/blocked.html)과 같은 모양.
        // Oops! 는 그 화면 몫이고, 여기서는 끝난 까닭을 한 단어로 크게 보인다
        <main className="gate">
            <div className="gate-body">
                <img
                    className={closed ? 'gate-art' : 'gate-art gate-art-float'}
                    src={closed ? art.denied : art.relay}
                    alt=""
                />
                <h1 className="gate-title">
                    {closed ? (
                        <span className="gate-word">
                            {t(`gate_word.${reason}`, { defaultValue: t('gate_word.closed') })}
                        </span>
                    ) : (
                        <Wordmark className="gate-wordmark" />
                    )}
                </h1>
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
                        <span className="tab-icon">
                            <Icon name={x.icon} />
                        </span>
                        <span>{t(`tabs.${x.id}`)}</span>
                    </button>
                ))}
            </nav>
            <main className="content">
                {/* key: 탭을 바꿀 때마다 새로 그려서 들어오는 움직임을 다시 낸다 */}
                <div className="view" key={tab}>
                    {tab === 'servers' && <ServersTab api={api} />}
                    {tab === 'permissions' && <PermissionsTab api={api} />}
                    {tab === 'modules' && <ModulesTab api={api} />}
                    {tab === 'more' && <MoreTab api={api} />}
                </div>
            </main>
            <ConfirmHost />
            <ToastHost />
        </div>
    );
}
