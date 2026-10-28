import {
    choosesVersion,
    installChoices,
    installFields,
    installSettings,
    nameTaken,
    releaseVersions,
    suggestName,
    versionSettings,
} from '@gui/utils/installFields';
import { gameNameOf, playersText, primaryAction, viewStatus } from '@gui/utils/serverView';
import { useEffect, useState } from 'react';
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

/** 새 서버 만들기: GUI 새 서버 마법사와 같은 것을 묻는다 (게임·이름·만들 때 고르는 설정·추천 설정·버전·실행 방식·약관)
 * 그리고 만들기 전에 다른 서버와 겹치는지 데몬에 묻는다(`instanceCheck`).
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
    const [versions, setVersions] = useState([]);
    const [version, setVersion] = useState(''); // '' = 최신
    const [managed, setManaged] = useState(true);
    const [check, setCheck] = useState(null);
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
        setVersion('');
        setManaged(true);
    };

    // 서버 파일을 받는 모듈은 고른 설정(서버 종류 등)으로 받을 수 있는 릴리스를 묻는다 (못 받으면 최신만).
    // 설정을 바꾸면 다시 묻고, 새 목록에 없는 버전을 골라 두었으면 최신으로 돌린다
    const downloads = choosesVersion(chosen);
    const versionKey = downloads ? JSON.stringify(versionSettings(chosen, installValues, preset)) : '';
    useEffect(() => {
        // 앞 모듈·설정의 목록은 바로 비운다 (새 목록이 오기 전·못 받으면 최신만 고른다)
        setVersions([]);
        if (!versionKey) return;
        let alive = true;
        const timer = setTimeout(() => {
            Promise.resolve(api.moduleListVersions?.(module, { per_page: 50, settings: JSON.parse(versionKey) }))
                .then((res) => {
                    if (!alive) return;
                    const list = releaseVersions(res);
                    setVersions(list);
                    setVersion((picked) => (picked && !list.includes(picked) ? '' : picked));
                })
                .catch(() => alive && setVersion(''));
        }, 250);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [api, module, versionKey]);

    // 만들기와 그 확인에 보내는 몸체. 고르지 않은 것(최신 버전, 사바쨩이 관리)은 보내지 않는다 (GUI createBody와 같다)
    const requestBody = () => {
        const body = { name: name.trim(), module_name: module };
        // 비운 칸은 빼서 데몬이 모듈 기본값·추천 설정을 쓰게 한다 (GUI와 같다)
        const settings = installSettings(
            patchOf(installFields(chosen), installValues, new Set(Object.keys(installValues))),
        );
        if (Object.keys(settings).length) body.module_settings = settings;
        if (preset) body.preset = preset;
        if (version) body.version = version;
        if (!managed) body.managed_start = false;
        return body;
    };
    const checkKey = name.trim() && module ? JSON.stringify(requestBody()) : '';

    // 입력이 멈추면 다른 서버와 겹치는지 묻는다. 확인하지 못해도 만들기가 다시 본다
    useEffect(() => {
        setCheck(null);
        if (!checkKey || !api.instanceCheck) return;
        let alive = true;
        const timer = setTimeout(() => {
            Promise.resolve(api.instanceCheck(JSON.parse(checkKey)))
                .then((res) => alive && !failed(res) && setCheck(res))
                .catch(() => {});
        }, 250);
        return () => {
            alive = false;
            clearTimeout(timer);
        };
    }, [api, checkKey]);
    const portNotes = (check?.ports || []).filter((p) => p.in_use_by?.length);

    const create = async () => {
        if (!ready) return;
        setBusy(true);
        const body = requestBody();
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
                    {downloads && versions.length > 0 && (
                        <div className="field">
                            <span>{t('servers.version')}</span>
                            <Dropdown
                                value={version}
                                onChange={setVersion}
                                ariaLabel={t('servers.version')}
                                options={[
                                    { value: '', label: t('servers.version_latest') },
                                    ...versions.map((v) => ({ value: v, label: v })),
                                ]}
                            />
                            {version && <small className="hint">{t('servers.version_hint')}</small>}
                        </div>
                    )}
                    <div className="field">
                        <span>{t('servers.run')}</span>
                        <Dropdown
                            value={managed ? 'managed' : 'unmanaged'}
                            onChange={(v) => setManaged(v === 'managed')}
                            ariaLabel={t('servers.run')}
                            options={[
                                { value: 'managed', label: t('servers.run_managed') },
                                { value: 'unmanaged', label: t('servers.run_unmanaged') },
                            ]}
                        />
                        <small className="hint">
                            {t(managed ? 'servers.run_managed_hint' : 'servers.run_unmanaged_hint')}
                        </small>
                    </div>
                    <div className="field">
                        <span>{t('servers.location')}</span>
                        <small className="hint">{t('servers.location_pc')}</small>
                    </div>
                    {portNotes.map((p) => (
                        <p key={p.field} className="banner banner-warn" role="note">
                            {t('servers.port_in_use', { servers: p.in_use_by.join(', '), port: p.port })}
                        </p>
                    ))}
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
