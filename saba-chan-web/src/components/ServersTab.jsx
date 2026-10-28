import { installChoices, installFields, installSettings, nameTaken, suggestName } from '@gui/utils/installFields';
import { gameNameOf, playersText, primaryAction, viewStatus } from '@gui/utils/serverView';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { art } from '../art';
import { useApp, useData } from '../store';
import { Dropdown } from './Dropdown';
import { Icon } from './icons';
import { ServerDetail } from './ServerDetail';
import { FieldInput, patchOf } from './SettingsPanel';
import { Button, Empty, errorText, failed, Loading, Section, Sheet, StatusDot } from './ui';
import { useProvision } from './useProvision';
import { useServerControl } from './useServerControl';

/** 새 서버 만들기: GUI 새 서버 화면과 같은 것을 묻는다 (게임·이름·만들 때 고르는 설정·추천 설정·약관).
 * 익스텐션이 묻는 칸(`extension_data`)과 폴더는 PC에서만 정한다 (src/remote/gate.rs CREATE_PC_ONLY) */
export function AddServerSheet({ api, onClose }) {
    const { t } = useTranslation();
    const modules = useData((s) => s.modules);
    const servers = useData((s) => s.servers);
    const toast = useApp((s) => s.toast);
    const firstPlayable = modules.find((m) => m.platform_supported !== false)?.name || '';
    const [module, setModule] = useState(firstPlayable);
    const [name, setName] = useState(() => (firstPlayable ? suggestName(firstPlayable, servers) : ''));
    const [installValues, setInstallValues] = useState({});
    const [preset, setPreset] = useState('');
    const [agreed, setAgreed] = useState({});
    const [busy, setBusy] = useState(false);

    const chosen = modules.find((m) => m.name === module);
    const modNs = `mod_${module}`;
    const agreements = chosen?.agreements || [];
    const duplicate = nameTaken(servers, name);
    const ready = name.trim() && module && !duplicate && agreements.every((a) => agreed[a.id]);

    const pickModule = (next) => {
        setModule(next);
        setName(suggestName(next, servers));
        setInstallValues({});
        setPreset('');
        setAgreed({});
    };

    const create = async () => {
        if (!ready) return;
        setBusy(true);
        const body = { name: name.trim(), module_name: module };
        // 비운 칸은 빼서 데몬이 모듈 기본값·추천 설정을 쓰게 한다 (GUI와 같다)
        const settings = installSettings(
            patchOf(installFields(chosen), installValues, new Set(Object.keys(installValues))),
        );
        if (Object.keys(settings).length) body.module_settings = settings;
        if (preset) body.preset = preset;
        if (agreements.length) body.accept_agreements = agreements.map((a) => a.id);
        const res = await api.instanceCreate(body);
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
                            onChange={pickModule}
                            ariaLabel={t('servers.game')}
                            options={modules.map((m) => {
                                const unsupported = m.platform_supported === false;
                                const game = gameNameOf(t, m.name, m);
                                return {
                                    value: m.name,
                                    label: unsupported ? `${game} · ${t('servers.platform_unsupported')}` : game,
                                    disabled: unsupported,
                                };
                            })}
                        />
                    </div>
                    {['unverified', 'modified'].includes(chosen?.trust) && (
                        <p className="banner banner-warn" role="note">
                            {t(`servers.trust_${chosen.trust}`)}
                        </p>
                    )}
                    <label className="field">
                        <span>{t('servers.name')}</span>
                        <input
                            value={name}
                            onChange={(e) => setName(e.target.value)}
                            maxLength={64}
                            required
                            aria-invalid={duplicate || undefined}
                        />
                    </label>
                    {duplicate && (
                        <p className="hint hint-error" role="alert">
                            {t('servers.name_taken', { name: name.trim() })}
                        </p>
                    )}
                    {installFields(chosen).map((f) => (
                        <FieldInput
                            key={f.name}
                            field={{ ...f, options: installChoices(f) }}
                            value={installValues[f.name] ?? f.default ?? ''}
                            onChange={(v) => setInstallValues((prev) => ({ ...prev, [f.name]: v }))}
                            t={t}
                            modNs={modNs}
                        />
                    ))}
                    {chosen?.presets?.length > 0 && (
                        <div className="field">
                            <span>{t('servers.preset')}</span>
                            <Dropdown
                                value={preset}
                                onChange={setPreset}
                                ariaLabel={t('servers.preset')}
                                options={[
                                    { value: '', label: t('servers.preset_none') },
                                    ...chosen.presets.map((p) => ({
                                        value: p.id,
                                        label: t(`${modNs}:presets.${p.id}.label`, { defaultValue: p.label || p.id }),
                                    })),
                                ]}
                            />
                            <small className="hint">
                                {preset
                                    ? t(`${modNs}:presets.${preset}.description`, {
                                          defaultValue: chosen.presets.find((p) => p.id === preset)?.description || '',
                                      })
                                    : t('servers.preset_none_hint')}
                            </small>
                        </div>
                    )}
                    {agreements.length > 0 && (
                        <fieldset className="field agreements">
                            <legend>{t('servers.agreements')}</legend>
                            {agreements.map((a) => {
                                const label = t(`${modNs}:agreements.${a.id}`, { defaultValue: a.label || a.id });
                                return (
                                    <div key={a.id} className="field-row">
                                        <label className="agreement">
                                            <input
                                                type="checkbox"
                                                checked={!!agreed[a.id]}
                                                onChange={(e) =>
                                                    setAgreed((prev) => ({ ...prev, [a.id]: e.target.checked }))
                                                }
                                            />
                                            <span>{t('servers.agree', { name: label })}</span>
                                        </label>
                                        {a.url && (
                                            <a href={a.url} target="_blank" rel="noreferrer noopener">
                                                {t('servers.read_agreement')}
                                            </a>
                                        )}
                                    </div>
                                );
                            })}
                        </fieldset>
                    )}
                    <p className="hint">{t('servers.add_hint')}</p>
                    <Button tone="accent" type="submit" busy={busy} disabled={!ready}>
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
