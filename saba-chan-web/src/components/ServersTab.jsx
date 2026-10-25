import { gameNameOf, playersText, primaryAction, viewStatus } from '@gui/utils/serverView';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { art } from '../art';
import { useApp, useData } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { ServerDetail } from './ServerDetail';
import { Button, Empty, errorText, failed, Loading, Section, Sheet, StatusDot } from './ui';
import { useProvision } from './useProvision';
import { useServerControl } from './useServerControl';

function AddServerSheet({ api, onClose }) {
    const { t } = useTranslation();
    const modules = useData((s) => s.modules);
    const toast = useApp((s) => s.toast);
    const [module, setModule] = useState(modules[0]?.name || '');
    const [name, setName] = useState('');
    const [busy, setBusy] = useState(false);

    const create = async () => {
        if (!name.trim() || !module) return;
        setBusy(true);
        const res = await api.instanceCreate({ name: name.trim(), module_name: module });
        setBusy(false);
        if (failed(res)) return toast(errorText(t, res), 'error');
        toast(t('servers.created', { name: name.trim() }), 'ok');
        onClose();
    };

    return (
        <Sheet title={t('servers.add')} onClose={onClose}>
            {modules.length === 0 ? (
                <Empty>{t('servers.no_modules')}</Empty>
            ) : (
                <form
                    className="form"
                    onSubmit={(e) => {
                        e.preventDefault();
                        create();
                    }}
                >
                    <div className="field">
                        <span>{t('servers.game')}</span>
                        <Dropdown
                            value={module}
                            onChange={setModule}
                            ariaLabel={t('servers.game')}
                            options={modules.map((m) => ({ value: m.name, label: gameNameOf(t, m.name, m) }))}
                        />
                    </div>
                    <label className="field">
                        <span>{t('servers.name')}</span>
                        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={64} required />
                    </label>
                    <p className="hint">{t('servers.add_hint')}</p>
                    <Button tone="accent" type="submit" busy={busy} disabled={!name.trim()}>
                        {t('servers.create')}
                    </Button>
                </form>
            )}
        </Sheet>
    );
}

function ServerCard({ api, server, moduleData, modulesLoaded }) {
    const { t } = useTranslation();
    const openServer = useApp((s) => s.openServer);
    const control = useServerControl(api);
    const { view: provision } = useProvision(api, server);
    const provisionFailed = provision?.state === 'failed';
    const status = provisionFailed ? 'provision_failed' : viewStatus(server, moduleData, modulesLoaded);
    const action = provisionFailed ? null : primaryAction(status);
    const players = playersText(server);

    return (
        <article className="card server-card">
            <button type="button" className="card-main" onClick={() => openServer(server.id)}>
                <div className="card-title">{server.name}</div>
                <div className="card-sub">
                    {gameNameOf(t, server.module, moduleData)}
                    {players && (
                        <span className="pill">
                            <Icon name="user" size={12} /> {players}
                        </span>
                    )}
                </div>
                <StatusDot status={status} />
            </button>
            <div className="card-actions">
                {action === 'start' && (
                    <Button tone="accent" busy={control.busy} onClick={() => control.start(server)}>
                        {t('servers.start')}
                    </Button>
                )}
                {action === 'stop' && (
                    <Button tone="danger" busy={control.busy} onClick={() => control.stop(server)}>
                        {t('servers.stop')}
                    </Button>
                )}
                {action === 'busy' && <Button disabled>{t(`status.${status}`)}</Button>}
            </div>
        </article>
    );
}

export function ServersTab({ api }) {
    const { t } = useTranslation();
    const { servers, serversLoaded, serversError, modules, modulesLoaded } = useData();
    const serverId = useApp((s) => s.serverId);
    const [adding, setAdding] = useState(false);

    const selected = servers.find((s) => s.id === serverId);
    if (selected) return <ServerDetail api={api} server={selected} />;

    const moduleOf = (name) => modules.find((m) => m.name === name);
    return (
        <Section
            title={t('tabs.servers')}
            action={
                <Button tone="accent" onClick={() => setAdding(true)}>
                    <Icon name="plus" size={16} /> {t('servers.add')}
                </Button>
            }
        >
            {serversError && <p className="banner banner-error">{serversError}</p>}
            {!serversLoaded ? (
                <Loading />
            ) : servers.length === 0 ? (
                <Empty art={art.sleep}>{t('servers.empty')}</Empty>
            ) : (
                <div className="list">
                    {servers.map((s) => (
                        <ServerCard
                            key={s.id}
                            api={api}
                            server={s}
                            moduleData={moduleOf(s.module)}
                            modulesLoaded={modulesLoaded}
                        />
                    ))}
                </div>
            )}
            {adding && <AddServerSheet api={api} onClose={() => setAdding(false)} />}
        </Section>
    );
}
