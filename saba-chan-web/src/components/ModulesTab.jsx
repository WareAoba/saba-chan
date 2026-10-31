import { localizedDescription } from '@gui/utils/localizedText';
import { gameNameOf } from '@gui/utils/serverView';
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { moduleIdOf, moduleUpdates, undeclaredPermissions } from './modules';
import { ServerIcon } from './ServerIcon';
import { Button, Empty, errorText, failed, Loading, Section, Segments } from './ui';

/** "업데이트 N개" 카드: 이름들과 모두 업데이트 */
function UpdatesCard({ items, busy, onUpdateAll }) {
    const { t } = useTranslation();
    if (items.length === 0) return null;
    return (
        <div className="row update-card">
            <div className="row-main">
                <div className="row-title">{t('modules.updates_count', { count: items.length })}</div>
                <div className="row-sub">{items.map((u) => u.label).join(', ')}</div>
            </div>
            <Button tone="accent" busy={busy === '*'} disabled={!!busy} onClick={onUpdateAll}>
                {t('modules.update_all')}
            </Button>
        </div>
    );
}

/** 모듈 검사 결과: module.toml에 선언하지 않은 기능을 코드에서 쓰는지 (GET /api/module/:name/audit) */
function ModuleCheck({ api, name }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const [res, setRes] = useState(null);
    useEffect(() => {
        let alive = true;
        Promise.resolve(api.moduleAudit(name))
            .catch(() => null)
            .then((r) => alive && setRes(r || {}));
        return () => {
            alive = false;
        };
    }, [api, name]);
    if (res === null) return <Loading />;
    if (!Array.isArray(res.undeclared)) return <p className="hint">{t('modules.check_failed')}</p>;
    if (res.undeclared.length === 0) return <p className="hint">{t('modules.check_ok')}</p>;
    const perms = undeclaredPermissions(res.undeclared)
        .map((p) => tg(`module_audit.perm_${p}`, { defaultValue: p }))
        .join(', ');
    return <p className="hint hint-warn">{t('modules.check_undeclared', { perms })}</p>;
}

function GameModules({ api }) {
    const { t, i18n } = useTranslation();
    const toast = useApp((s) => s.toast);
    const { modules, setModules } = useData();
    const [catalog, setCatalog] = useState(null);
    const [busy, setBusy] = useState(null);
    const [open, setOpen] = useState(null);

    useEffect(() => {
        api.moduleManifest().then((res) => {
            const mods = res?.manifest?.modules;
            // 목록을 못 받았으면 "모두 설치했어요"가 아니라 받지 못한 이유를 보인다
            if (failed(res) || !mods || typeof mods !== 'object')
                return setCatalog({ error: t('modules.catalog_failed') });
            setCatalog(Object.entries(mods).map(([id, info]) => ({ id, ...info })));
        });
    }, [api, t]);

    // 설치와 업데이트는 같은 요청이다: 데몬이 공식 목록의 판으로 바꿔 끼운다
    const fetchOne = async (id) => {
        const res = await api.moduleInstallFromManifest(id);
        if (!failed(res)) return null;
        return res?.error_code === 'module_busy' ? t('modules.busy') : errorText(t, res);
    };
    const refresh = async () => {
        const list = await api.moduleRefresh();
        if (Array.isArray(list?.modules)) setModules(list.modules);
    };
    const run = async (key, ids, okText) => {
        setBusy(key);
        const errors = [];
        for (const id of ids) {
            const error = await fetchOne(id);
            if (error) errors.push(error);
        }
        await refresh();
        setBusy(null);
        if (errors.length) toast(errors[0], 'error');
        else toast(okText, 'ok');
    };

    const installed = new Set(modules.map(moduleIdOf));
    const available = Array.isArray(catalog) ? catalog.filter((m) => !installed.has(m.id)) : [];
    const updates = moduleUpdates(modules, catalog);
    const latestOf = new Map(updates.map((u) => [u.id, u.latest]));
    const byName = new Map(modules.map((m) => [m.name, m]));
    return (
        <>
            <UpdatesCard
                items={updates.map((u) => ({ label: gameNameOf(t, u.name, byName.get(u.name)) }))}
                busy={busy}
                onUpdateAll={() =>
                    run(
                        '*',
                        updates.map((u) => u.id),
                        t('modules.updated'),
                    )
                }
            />
            <Section title={t('modules.installed_title')}>
                {modules.length === 0 ? (
                    <Empty>{t('modules.none_installed')}</Empty>
                ) : (
                    <ul className="rows">
                        {modules.map((m) => {
                            const id = moduleIdOf(m);
                            const latest = latestOf.get(id);
                            const expanded = open === m.name;
                            return (
                                <li key={m.name} className="row row-stack">
                                    <div className="row-line">
                                        <ServerIcon module={m.name} moduleData={m} size={36} />
                                        <button
                                            type="button"
                                            className="row-main row-toggle"
                                            aria-expanded={expanded}
                                            onClick={() => setOpen(expanded ? null : m.name)}
                                        >
                                            <span className="row-title">{gameNameOf(t, m.name, m)}</span>
                                            <span className="row-sub">
                                                v{m.version}
                                                {latest && (
                                                    <span className="pill pill-accent">
                                                        {t('modules.update_to', { version: latest })}
                                                    </span>
                                                )}
                                            </span>
                                        </button>
                                        {latest && (
                                            <Button
                                                tone="accent"
                                                busy={busy === id}
                                                disabled={!!busy}
                                                onClick={() => run(id, [id], t('modules.updated'))}
                                            >
                                                {t('modules.update')}
                                            </Button>
                                        )}
                                    </div>
                                    {expanded && (
                                        <div className="row-detail">
                                            <ModuleCheck api={api} name={m.name} />
                                        </div>
                                    )}
                                </li>
                            );
                        })}
                    </ul>
                )}
            </Section>
            <Section title={t('modules.available_title')}>
                {catalog === null ? (
                    <Loading />
                ) : catalog.error ? (
                    <Empty>{catalog.error}</Empty>
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
                                    busy={busy === m.id}
                                    disabled={!!busy}
                                    onClick={() => run(m.id, [m.id], t('modules.installed'))}
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

/** 플러그인 오류: 켜진 서버가 쓰는 플러그인은 바꿔 끼우지 않는다 */
const pluginError = (t, res) => (res?.error_code === 'in_use' ? t('plugins.in_use') : errorText(t, res));

function Plugins({ api }) {
    const { t, i18n } = useTranslation();
    const toast = useApp((s) => s.toast);
    const [list, setList] = useState(null);
    const [catalog, setCatalog] = useState(null);
    const [updates, setUpdates] = useState([]);
    const [busy, setBusy] = useState(null);

    const load = useCallback(async () => {
        const [mine, store, news] = await Promise.all([
            api.extensionList(),
            api.extensionFetchManifest(),
            api.extensionCheckUpdates(),
        ]);
        setList(Array.isArray(mine?.extensions) ? mine.extensions : []);
        setCatalog(Array.isArray(store?.extensions) ? store.extensions : { error: t('modules.catalog_failed') });
        setUpdates(Array.isArray(news?.updates) ? news.updates : []);
    }, [api, t]);

    useEffect(() => {
        load();
    }, [load]);

    const act = async (id, fn, okText) => {
        setBusy(id);
        const res = await fn();
        setBusy(null);
        if (failed(res)) return toast(pluginError(t, res), 'error');
        toast(okText, 'ok');
        load();
    };
    const updateAll = async (ids) => {
        setBusy('*');
        let error = null;
        for (const id of ids) {
            const res = await api.extensionInstall(id);
            if (failed(res) && !error) error = pluginError(t, res);
        }
        setBusy(null);
        toast(error || t('plugins.updated'), error ? 'error' : 'ok');
        load();
    };

    if (list === null) return <Loading />;
    const installed = new Set(list.map((e) => e.id));
    const available = Array.isArray(catalog) ? catalog.filter((e) => !installed.has(e.id)) : [];
    const updateOf = new Map(updates.map((u) => [u.id, u]));
    const nameOf = new Map(list.map((e) => [e.id, e.name || e.id]));
    // 더 새 앱이 필요한 판은 웹에서 받을 수 없다 (앱 업데이트는 PC에서)
    const ready = updates.filter((u) => !u.incompatible);
    return (
        <>
            <UpdatesCard
                items={ready.map((u) => ({ label: nameOf.get(u.id) || u.id }))}
                busy={busy}
                onUpdateAll={() => updateAll(ready.map((u) => u.id))}
            />
            <Section title={t('plugins.installed_title')}>
                {list.length === 0 ? (
                    <Empty>{t('plugins.none_installed')}</Empty>
                ) : (
                    <ul className="rows">
                        {list.map((e) => {
                            const update = updateOf.get(e.id);
                            return (
                                <li key={e.id} className="row">
                                    <div className="row-main">
                                        <div className="row-title">{e.name || e.id}</div>
                                        <div className="row-sub">
                                            {e.version ? `v${e.version} · ` : ''}
                                            {localizedDescription(e, i18n.language)}
                                            {update && (
                                                <span className="pill pill-accent">
                                                    {update.incompatible
                                                        ? t('plugins.needs_app_update')
                                                        : t('modules.update_to', { version: update.latest_version })}
                                                </span>
                                            )}
                                        </div>
                                    </div>
                                    {update && !update.incompatible && (
                                        <Button
                                            tone="accent"
                                            busy={busy === `u:${e.id}`}
                                            disabled={!!busy}
                                            onClick={() =>
                                                act(`u:${e.id}`, () => api.extensionInstall(e.id), t('plugins.updated'))
                                            }
                                        >
                                            {t('modules.update')}
                                        </Button>
                                    )}
                                    <label className="toggle">
                                        <input
                                            type="checkbox"
                                            className="switch"
                                            checked={!!e.enabled}
                                            disabled={!!busy}
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
                            );
                        })}
                    </ul>
                )}
            </Section>
            <Section title={t('plugins.available_title')}>
                {catalog?.error ? (
                    <Empty>{catalog.error}</Empty>
                ) : available.length === 0 ? (
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
            <p className="hint">{t('plugins.settings_pc_only')}</p>
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
