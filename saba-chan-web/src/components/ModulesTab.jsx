import { localizedDescription } from '@gui/utils/localizedText';
import { gameNameOf } from '@gui/utils/serverView';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { Button, Empty, errorText, failed, Loading, Section, Segments } from './ui';

/** 설치된 모듈의 id (폴더 이름. 데몬 목록의 name 과 같을 때가 많지만 path 가 기준이다) */
const moduleIdOf = (m) => (m.path || '').replace(/\\/g, '/').split('/').pop() || m.name;

function GameModules({ api }) {
    const { t, i18n } = useTranslation();
    const toast = useApp((s) => s.toast);
    const { modules, setModules } = useData();
    const [catalog, setCatalog] = useState(null);
    const [installing, setInstalling] = useState(null);

    useEffect(() => {
        api.moduleManifest().then((res) => {
            const mods = res?.manifest?.modules;
            setCatalog(
                mods && typeof mods === 'object' ? Object.entries(mods).map(([id, info]) => ({ id, ...info })) : [],
            );
        });
    }, [api]);

    const install = async (id) => {
        setInstalling(id);
        const res = await api.moduleInstallFromManifest(id);
        setInstalling(null);
        if (failed(res)) {
            return toast(res?.error_code === 'module_busy' ? t('modules.busy') : errorText(t, res), 'error');
        }
        const list = await api.moduleRefresh();
        if (Array.isArray(list?.modules)) setModules(list.modules);
        toast(t('modules.installed'), 'ok');
    };

    const installed = new Set(modules.map(moduleIdOf));
    const available = (catalog ?? []).filter((m) => !installed.has(m.id));
    return (
        <>
            <Section title={t('modules.installed_title')}>
                {modules.length === 0 ? (
                    <Empty>{t('modules.none_installed')}</Empty>
                ) : (
                    <ul className="rows">
                        {modules.map((m) => (
                            <li key={m.name} className="row">
                                <div className="row-main">
                                    <div className="row-title">{gameNameOf(t, m.name, m)}</div>
                                    <div className="row-sub">v{m.version}</div>
                                </div>
                            </li>
                        ))}
                    </ul>
                )}
            </Section>
            <Section title={t('modules.available_title')}>
                {catalog === null ? (
                    <Loading />
                ) : available.length === 0 ? (
                    <Empty>{t('modules.all_installed')}</Empty>
                ) : (
                    <ul className="rows">
                        {available.map((m) => (
                            <li key={m.id} className="row">
                                <div className="row-main">
                                    <div className="row-title">{m.display_name || m.id}</div>
                                    <div className="row-sub">{localizedDescription(m, i18n.language)}</div>
                                </div>
                                <Button
                                    tone="accent"
                                    busy={installing === m.id}
                                    disabled={!!installing}
                                    onClick={() => install(m.id)}
                                >
                                    {t('modules.install')}
                                </Button>
                            </li>
                        ))}
                    </ul>
                )}
            </Section>
        </>
    );
}

function Plugins({ api }) {
    const { t, i18n } = useTranslation();
    const toast = useApp((s) => s.toast);
    const [list, setList] = useState(null);
    const [catalog, setCatalog] = useState(null);
    const [busy, setBusy] = useState(null);

    const load = useCallback(async () => {
        const [mine, store] = await Promise.all([api.extensionList(), api.extensionFetchManifest()]);
        setList(Array.isArray(mine?.extensions) ? mine.extensions : []);
        setCatalog(Array.isArray(store?.extensions) ? store.extensions : []);
    }, [api]);

    useEffect(() => {
        load();
    }, [load]);

    const act = async (id, fn, okText) => {
        setBusy(id);
        const res = await fn();
        setBusy(null);
        if (failed(res)) return toast(errorText(t, res), 'error');
        toast(okText, 'ok');
        load();
    };

    if (list === null) return <Loading />;
    const installed = new Set(list.map((e) => e.id));
    const available = (catalog ?? []).filter((e) => !installed.has(e.id));
    return (
        <>
            <Section title={t('plugins.installed_title')}>
                {list.length === 0 ? (
                    <Empty>{t('plugins.none_installed')}</Empty>
                ) : (
                    <ul className="rows">
                        {list.map((e) => (
                            <li key={e.id} className="row">
                                <div className="row-main">
                                    <div className="row-title">{e.name || e.id}</div>
                                    <div className="row-sub">{localizedDescription(e, i18n.language)}</div>
                                </div>
                                <label className="toggle">
                                    <input
                                        type="checkbox"
                                        className="switch"
                                        checked={!!e.enabled}
                                        disabled={busy === e.id}
                                        onChange={(ev) =>
                                            act(
                                                e.id,
                                                () =>
                                                    ev.target.checked
                                                        ? api.extensionEnable(e.id)
                                                        : api.extensionDisable(e.id),
                                                ev.target.checked ? t('plugins.enabled') : t('plugins.disabled'),
                                            )
                                        }
                                    />
                                    <span className="sr-only">{t('plugins.toggle')}</span>
                                </label>
                            </li>
                        ))}
                    </ul>
                )}
            </Section>
            <Section title={t('plugins.available_title')}>
                {available.length === 0 ? (
                    <Empty>{t('plugins.all_installed')}</Empty>
                ) : (
                    <ul className="rows">
                        {available.map((e) => (
                            <li key={e.id} className="row">
                                <div className="row-main">
                                    <div className="row-title">{e.name || e.id}</div>
                                    <div className="row-sub">{localizedDescription(e, i18n.language)}</div>
                                </div>
                                <Button
                                    tone="accent"
                                    busy={busy === e.id}
                                    disabled={!!busy}
                                    onClick={() => act(e.id, () => api.extensionInstall(e.id), t('plugins.installed'))}
                                >
                                    {t('modules.install')}
                                </Button>
                            </li>
                        ))}
                    </ul>
                )}
            </Section>
        </>
    );
}

export function ModulesTab({ api }) {
    const { t } = useTranslation();
    const [view, setView] = useState('games');
    return (
        <div className="panel">
            <Segments
                value={view}
                onChange={setView}
                options={[
                    { value: 'games', label: t('modules.games') },
                    { value: 'plugins', label: t('modules.plugins') },
                ]}
            />
            {view === 'games' ? <GameModules api={api} /> : <Plugins api={api} />}
            <p className="hint">{t('modules.official_only')}</p>
        </div>
    );
}
