import { useCallback, useEffect, useState } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import type { ApiBody } from '../../types/daemon';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { OsLogo, osFamily, type Platform } from '../OsLogo';
import { Icon } from '../Icon';
import { SabaNumberInput, SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { usePendingChange } from '../ui/SaveBar';
import { REMOTE_DAEMONS_CHANGED, reloadWindow, useDaemonTarget } from '../shell/DaemonSwitcher';
import { PublicAddressRow } from './PublicAddress';
import { InstallDialog, RemoveDialog } from './RemotePcDialogs';
import { failed } from '../../utils/apiResult';

export const remoteDaemonAvailable = () => typeof window.api?.remoteSettings === 'function';

// 데몬 원격 데몬 오류 코드 중 문구가 있는 것 (01 §1.4.2)
const ERRORS = [
    'certificate_failed',
    'port_in_use',
    'bind_failed',
    'invalid_port',
    'invalid_address',
    'remote_not_found',
    'remote_no_endpoint',
    'remote_unreachable',
    'remote_rejected',
    'remote_unknown',
    'relay_not_paired',
    'relay_unreachable',
    'remote_timeout',
    'remote_ambiguous',
    'remote_bad_path',
    'remote_unauthorized',
    'remote_local_only',
    'pair_code_not_found',
    'pair_code_used',
    'pair_code_expired',
    'remote_not_ready',
    'ssh_missing',
    'ssh_spawn_failed',
    'ssh_failed',
    'install_failed',
    'pair_code_missing',
    'unsupported',
    'cli_missing',
    'remote_uninstall_failed',
    'invalid_host',
    'invalid_user',
    'invalid_code',
];

/** 화면에 보일 PC 이름: 데몬이 정한 `디스코드 이름(PC 이름)`, 옛 데몬이면 PC 이름 */
export const pcName = (pc?: ApiBody | null) => pc?.label || pc?.name || pc?.hostId || '';

/** 실패 → 한 줄 (아는 코드는 번역, 나머지는 데몬 문구) */
export function remoteErrorText(res: ApiBody | null | undefined, t: TFunction) {
    const code = String(res?.error_code || res?.error || '');
    if (ERRORS.includes(code)) return t(`remote_daemon.err_${code}`);
    return res?.error || t('remote_daemon.err_generic');
}

/** 판 비교: a가 b보다 옛 판인가 (`0.1.12` < `0.1.13`). 읽지 못하면 false */
export function olderVersion(a?: string | null, b?: string | null) {
    const parse = (v: string) => v.split('-')[0].split('.').map((n) => Number.parseInt(n, 10));
    if (!a || !b) return false;
    const [x, y] = [parse(a), parse(b)];
    if (x.some(Number.isNaN) || y.some(Number.isNaN)) return false;
    for (let i = 0; i < Math.max(x.length, y.length); i++) {
        const d = (x[i] || 0) - (y[i] || 0);
        if (d !== 0) return d < 0;
    }
    return false;
}

/** 카드 한 장: 이 PC가 연결한 원격 데몬(`daemon`)이나 서버쨩의 내 다른 PC(`peer`), 또는 둘 다 */
interface PcCard {
    hostId: string;
    daemon?: ApiBody;
    peer?: ApiBody;
}

/**
 * 설정 > 연결: 원격 데몬 (데몬 `/api/remote/*`, 01 §1.4.2). 이 PC를 내 다른 PC에 열기, 내 PC 카드(이 PC, 연결한 PC, 연결하지 않은 내 PC,
 * 리눅스에 설치), 이 PC에 붙은 PC. 카드의 별이 메인 데몬(`gui.mainDaemon`, 앱을 열면 먼저 보일 데몬)이다. CLI `remote …`와 같은 기능이다.
 * 이 경로들은 고른 데몬과 상관없이 늘 이 PC의 데몬이 답한다.
 */
interface RemoteDaemonSectionProps {
    onTargetGone?: () => void;
    onSwitched?: () => void;
}

export function RemoteDaemonSection({ onTargetGone = reloadWindow, onSwitched = reloadWindow }: RemoteDaemonSectionProps) {
    const { t, i18n } = useTranslation('gui');
    const [settings, setSettings] = useState<ApiBody | null>(null);
    const [draft, setDraft] = useState<{ enabled: boolean; port: string } | null>(null); // {enabled, port}. 주소는 카드의 게임 서버 주소 줄에서 (PublicAddress)
    const [clients, setClients] = useState<ApiBody[]>([]);
    const [peers, setPeers] = useState<ApiBody[] | null>(null); // null = 못 읽음
    const [peersError, setPeersError] = useState<string | null>(null);
    const [daemons, setDaemons] = useState<ApiBody[]>([]);
    // 원격 데몬마다 닿는지 (`/api/remote/servers`의 ok·error_code). 아직 모르면 비어 있다
    const [reach, setReach] = useState<Record<string, string>>({});
    const [busy, setBusy] = useState<string | null>(null); // 진행 중인 동작 이름
    const [error, setError] = useState<string | null>(null);
    const [notice, setNotice] = useState<string | null>(null);
    const [removing, setRemoving] = useState<PcCard | null>(null);
    const [installing, setInstalling] = useState(false);
    const daemonView = useSettingsStore((st) => st.daemonView);
    const mainDaemon = useSettingsStore((st) => st.mainDaemon);
    const daemonTarget = useDaemonTarget();
    const updateSettings = useSettingsStore((st) => st.update);

    const loadSettings = useCallback(async () => {
        const res = await window.api.remoteSettings();
        if (failed(res)) return;
        setSettings(res);
        setDraft({ enabled: Boolean(res.enabled), port: String(res.port ?? '') });
    }, []);
    const loadClients = useCallback(async () => {
        const res = await window.api.remoteClients();
        if (!failed(res)) setClients(res.clients || []);
    }, []);
    const loadPeers = useCallback(async () => {
        const res = await window.api.remotePeers();
        if (failed(res)) {
            setPeers(null);
            setPeersError(res?.error_code === 'relay_not_paired' ? t('remote_daemon.peers_need_relay') : remoteErrorText(res, t));
            return;
        }
        setPeersError(null);
        setPeers(res.peers || []);
    }, [t]);
    const loadDaemons = useCallback(async () => {
        const res = await window.api.remoteDaemons();
        if (failed(res)) return;
        setDaemons(res.daemons || []);
        if (!res.daemons?.length || typeof window.api.remoteServers !== 'function') return;
        // 그 PC의 데몬이 지워졌거나 다시 설치돼 짝을 잊었는지 본다 (닿지 않는 데몬은 늦게 답한다)
        const all = await window.api.remoteServers().catch(() => null);
        const next: Record<string, string> = {};
        for (const d of (all?.daemons || []) as ApiBody[]) if (d.hostId) next[d.hostId] = d.ok ? 'ok' : d.error_code || 'remote_unreachable';
        setReach(next);
    }, []);

    useEffect(() => {
        loadSettings();
        loadClients();
        loadPeers();
        loadDaemons();
    }, [loadSettings, loadClients, loadPeers, loadDaemons]);

    const run = async (name: string, fn: () => Promise<ApiBody>) => {
        setBusy(name);
        setError(null);
        setNotice(null);
        try {
            const res = await fn();
            if (failed(res)) setError(remoteErrorText(res, t));
            return res;
        } catch (e) {
            setError((e as Error).message);
            return null;
        } finally {
            setBusy(null);
        }
    };

    const reloadPcs = useCallback(async () => {
        await Promise.all([loadPeers(), loadDaemons()]);
        window.dispatchEvent(new Event(REMOTE_DAEMONS_CHANGED));
    }, [loadPeers, loadDaemons]);

    // 켬은 바로 저장하고, 포트는 다 친 뒤 저장 막대로 묻는다 (사용자 11-11)
    const save = async (next: { enabled: boolean; port: string }) => {
        const port = Number(next.port);
        if (!Number.isInteger(port) || port < 1 || port > 65535) {
            setError(t('remote_daemon.err_invalid_port'));
            return;
        }
        const res = await run('save', () => window.api.remoteSettingsSet({ enabled: next.enabled, port }));
        if (res && !failed(res)) {
            setSettings((old) => ({ ...res, thisPc: res.thisPc || old?.thisPc }));
            setDraft({ enabled: Boolean(res.enabled), port: String(res.port ?? port) });
        }
    };
    const savedPort = String(settings?.port ?? '');
    const draftPort = Number(draft?.port);
    usePendingChange('remote-port', {
        dirty: !!draft && draft.port !== savedPort,
        blocked: !Number.isInteger(draftPort) || draftPort < 1 || draftPort > 65535,
        save: () => draft && save({ enabled: Boolean(settings?.enabled), port: draft.port }),
        revert: () => draft && setDraft({ ...draft, port: savedPort }),
    });

    const connect = async (hostId: string) => {
        const res = await run(`connect:${hostId}`, () => window.api.remoteConnect(hostId));
        if (res && !failed(res)) await reloadPcs();
    };
    const revoke = async (hostId: string) => {
        const res = await run(`revoke:${hostId}`, () => window.api.remoteRevoke(hostId));
        if (res && !failed(res)) await loadClients();
    };

    const remove = async (card: PcCard, choice: { uninstall: boolean; keepSettings: boolean; keepInstances: boolean; removePc: boolean }) => {
        const res = await run(`remove:${card.hostId}`, () => window.api.remoteRemove(card.hostId, choice));
        if (!res || failed(res)) return;
        setRemoving(null);
        // 지운 데몬은 메인일 수 없다: 다음에 열 때 이 PC부터 보인다
        if (mainDaemon === card.hostId) updateSettings({ mainDaemon: null });
        // 지금 다루던 데몬이면 이 PC로 돌아간다 (남겨 두면 모든 요청이 remote_unknown이다)
        if (card.daemon) {
            const current = await window.api.daemonTarget?.().catch(() => null);
            if (current?.target === card.hostId) {
                const back = await window.api.daemonTargetSet(null).catch(() => null);
                if (back?.success) {
                    onTargetGone();
                    return;
                }
            }
        }
        await reloadPcs();
    };

    const update = async (hostId: string) => {
        const res = await run(`update:${hostId}`, () => window.api.remoteDaemonUpdate(hostId));
        if (res && !failed(res)) setNotice(t('remote_daemon.update_started'));
    };

    // 저장한 원격 데몬을 고른다 (상단 바와 같다). null이면 이 PC
    const open = async (hostId: string | null) => {
        const res = await run(`open:${hostId || 'local'}`, () => window.api.daemonTargetSet(hostId));
        if (res?.success) onSwitched();
    };

    // 저장한 원격 데몬이 지금 못 쓰는 까닭: 서버쨩에서 지워진 PC(gone), 그 PC가 짝을 잊음(unpaired, 데몬을 지우고
    // 다시 설치했을 때. 다시 연결하면 된다), 닿지 않음(unreachable, 꺼졌거나 데몬이 지워졌다). 쓸 수 있으면 null
    const daemonState = (d: ApiBody) => {
        if (peers && !peers.some((p) => p.hostId === d.hostId)) return 'gone';
        const r = reach[d.hostId];
        if (r === 'remote_unauthorized') return 'unpaired';
        if (r && r !== 'ok') return 'unreachable';
        return null;
    };

    // 카드 차례: 연결한 PC, 아직 연결하지 않은 내 PC
    const cards: PcCard[] = [
        ...daemons.map((d) => ({ hostId: d.hostId, daemon: d, peer: peers?.find((p) => p.hostId === d.hostId) })),
        ...(peers || []).filter((p) => !daemons.some((d) => d.hostId === p.hostId)).map((p) => ({ hostId: p.hostId, peer: p })),
    ];
    const mainIsRemote = daemons.some((d) => d.hostId === mainDaemon);

    const thisPc = settings?.thisPc || {};
    const thisVersion: string | undefined = thisPc.agentVersion;
    const when = (iso?: string | null) => {
        if (!iso) return null;
        const d = new Date(iso);
        return Number.isNaN(d.getTime()) ? null : d.toLocaleString(i18n.language, { dateStyle: 'medium', timeStyle: 'short' });
    };
    const ago = (iso?: string | null) => {
        if (!iso) return null;
        const ms = Date.now() - new Date(iso).getTime();
        if (Number.isNaN(ms)) return null;
        const min = Math.round(ms / 60000);
        if (min < 1) return t('remote_daemon.just_now');
        const rtf = new Intl.RelativeTimeFormat(i18n.language, { numeric: 'auto' });
        if (min < 60) return rtf.format(-min, 'minute');
        if (min < 60 * 24) return rtf.format(-Math.round(min / 60), 'hour');
        return rtf.format(-Math.round(min / 1440), 'day');
    };
    const osLine = (platform?: Platform | null, version?: string | null) => {
        const family = osFamily(platform);
        const arch = String(platform?.os || '').split(' ')[1];
        return [
            platform?.distro?.name || (family ? t(`remote_daemon.os_${family}`) : ''),
            arch,
            version,
        ]
            .filter(Boolean)
            .join(' · ');
    };

    const status = settings?.status || {};
    const statusError = settings?.enabled && status.error ? remoteErrorText({ error_code: status.error }, t) : null;
    const button = (name: string, label: string, onClick: () => void, icon: string, kind = 'btn-ghost', disabled = false) => (
        <button type="button" className={`btn ${kind} btn-sm`} onClick={onClick} disabled={busy !== null || disabled}>
            {busy === name ? <SabaSpinner size="xs" /> : <Icon name={icon} size="sm" />} {label}
        </button>
    );
    const star = (on: boolean, onClick: () => void) => (
        <button
            type="button"
            className={`remote-pc-star${on ? ' on' : ''}`}
            aria-pressed={on}
            aria-label={on ? t('remote_daemon.star_on') : t('remote_daemon.star_set')}
            data-tooltip={on ? t('remote_daemon.star_on') : t('remote_daemon.star_set')}
            onClick={onClick}
        >
            <Icon name="star" size="sm" />
        </button>
    );
    const pill = (kind: string, text: string) => <span className={`remote-pc-pill ${kind}`}>{text}</span>;

    const renderCard = (card: PcCard) => {
        const { daemon, peer } = card;
        const name = pcName(daemon || peer);
        const platform = peer?.platform as Platform | undefined;
        const online = peer?.status === 'online';
        const state = daemon ? daemonState(daemon) : null;
        const needsUpdate = Boolean(daemon) && olderVersion(peer?.agentVersion, thisVersion);
        const route = daemon ? peer?.route || (daemon.address ? 'direct' : 'relay') : null;
        const lastSeen = online ? t('remote_daemon.just_now') : ago(peer?.lastSeenAt);
        return (
            <li key={card.hostId} className={`remote-pc-card${online || state === null ? '' : ' off'}`}>
                <div className="remote-pc-top">
                    <span className="remote-pc-os">
                        <OsLogo platform={platform} />
                    </span>
                    <span className="remote-pc-who">
                        <b className="remote-pc-name">{name}</b>
                        <span className="remote-pc-sub">{osLine(platform, peer?.agentVersion)}</span>
                    </span>
                    {state
                        ? pill('warn', t(`remote_daemon.state_${state}`))
                        : peer && pill(online ? 'on' : 'off', online ? t('remote_daemon.online') : t('remote_daemon.offline'))}
                    {daemon && star(mainDaemon === card.hostId, () => updateSettings({ mainDaemon: card.hostId }))}
                </div>
                <dl className="remote-pc-kv">
                    <dt>{t('remote_daemon.paired_at')}</dt>
                    <dd>{daemon ? when(daemon.pairedAt) || '-' : <span className="muted">{t('remote_daemon.not_paired_yet')}</span>}</dd>
                    <dt>{t('remote_daemon.last_seen')}</dt>
                    <dd>{lastSeen || '-'}</dd>
                    <dt>{t('remote_daemon.public_ip')}</dt>
                    <dd>{peer?.publicIp ? <code>{peer.publicIp}</code> : <span className="muted">{t('remote_daemon.ip_unknown')}</span>}</dd>
                    {daemon && state === null && online && <PublicAddressRow hostId={card.hostId} name={name} />}
                    <dt>{t('remote_daemon.route')}</dt>
                    <dd>
                        {route === 'direct' && t('remote_daemon.route_direct', { port: daemon?.port })}
                        {route === 'relay' && t('remote_daemon.via_relay')}
                        {!daemon && (peer?.hasEndpoint ? t('remote_daemon.route_open') : <span className="muted">{t('remote_daemon.no_endpoint')}</span>)}
                    </dd>
                </dl>
                <div className="remote-pc-acts">
                    {needsUpdate && <span className="remote-pc-tag warn">{t('remote_daemon.update_needed', { version: thisVersion })}</span>}
                    {needsUpdate && button(`update:${card.hostId}`, t('remote_daemon.update'), () => update(card.hostId), 'download', 'btn-ghost', !online)}
                    {daemon && daemonTarget === card.hostId && <span className="remote-pc-tag">{t('remote_daemon.viewing')}</span>}
                    {button(`remove:${card.hostId}`, t('remote_daemon.delete'), () => setRemoving(card), 'trash', 'btn-ghost danger')}
                    {daemon && state === 'unpaired' && button(`connect:${card.hostId}`, t('remote_daemon.reconnect'), () => connect(card.hostId), 'refresh', 'btn-primary')}
                    {daemon && state !== 'unpaired' && daemonTarget !== card.hostId &&
                        button(`open:${card.hostId}`, t('remote_daemon.open'), () => open(card.hostId), 'externalLink')}
                    {!daemon && peer?.hasEndpoint && button(`connect:${card.hostId}`, t('remote_daemon.connect'), () => connect(card.hostId), 'plug', 'btn-primary')}
                </div>
            </li>
        );
    };

    const removingOnline = removing?.peer?.status === 'online';
    const removingFamily = osFamily(removing?.peer?.platform);

    return (
        <>
        <section className="saba-card">
            <h4 className="saba-card-title">
                <Icon name="cloud" size="sm" /> {t('remote_daemon.title')}
            </h4>
            <p className="setting-description">{t('remote_daemon.description')}</p>

            {draft && (
                <>
                    <div className="setting-item">
                        <label className="setting-label" htmlFor="remote-daemon-enabled">
                            <span className="setting-title">
                                <Icon name="network" size="sm" /> {t('remote_daemon.enabled')}
                            </span>
                            <span className="setting-description">{t('remote_daemon.enabled_description')}</span>
                            <span className="setting-description remote-daemon-status" role="status">
                                {busy === 'save' && <SabaSpinner size="xs" />}
                                {!settings!.enabled && t('remote_daemon.status_off')}
                                {settings!.enabled &&
                                    status.listening &&
                                    t('remote_daemon.status_listening', { port: settings!.port })}
                            </span>
                            {settings!.enabled && status.published && (
                                <span className="setting-description">{t('remote_daemon.status_published')}</span>
                            )}
                        </label>
                        <SabaToggle
                            id="remote-daemon-enabled"
                            aria-label={t('remote_daemon.enabled')}
                            checked={draft.enabled}
                            disabled={busy !== null}
                            onChange={(enabled) => save({ enabled, port: savedPort })}
                        />
                    </div>
                    <div className="setting-item">
                        <label className="setting-label" htmlFor="remote-daemon-port">
                            <span className="setting-title">
                                <Icon name="hash" size="sm" /> {t('remote_daemon.port')}
                            </span>
                        </label>
                        <SabaNumberInput
                            id="remote-daemon-port"
                            aria-label={t('remote_daemon.port')}
                            className="input setting-num"
                            value={draft.port}
                            min={1}
                            max={65535}
                            onChange={(e) => setDraft({ ...draft, port: e.target.value })}
                        />
                    </div>
                    {statusError && (
                        <div className="setting-validation-error" role="alert">
                            <Icon name="alertCircle" size="sm" /> {statusError}
                        </div>
                    )}
                </>
            )}

        </section>

            {/* 내 PC 카드들은 카드 안의 카드가 되지 않게 따로 둔다 */}
            <h5 className="remote-daemon-subtitle">{t('remote_daemon.my_pcs')}</h5>
            <p className="setting-description">{t('remote_daemon.my_pcs_description')}</p>
            {peersError && <p className="setting-description">{peersError}</p>}
            <ul className="remote-pc-cards remote-pc-cards-even" aria-label={t('remote_daemon.my_pcs')}>
                <li className="remote-pc-card me">
                    <div className="remote-pc-top">
                        <span className="remote-pc-os">
                            <OsLogo platform={thisPc.platform} />
                        </span>
                        <span className="remote-pc-who">
                            <b className="remote-pc-name">
                                {t('daemon_switcher.local')} <span className="remote-pc-me">{t('remote_daemon.this_pc_tag')}</span>
                            </b>
                            <span className="remote-pc-sub">{osLine(thisPc.platform, thisVersion)}</span>
                        </span>
                        {pill('on', t('remote_daemon.online'))}
                        {star(!mainIsRemote, () => updateSettings({ mainDaemon: null }))}
                    </div>
                    <dl className="remote-pc-kv">
                        <dt>{t('remote_daemon.public_ip')}</dt>
                        <dd>{thisPc.publicIp ? <code>{thisPc.publicIp}</code> : <span className="muted">{t('remote_daemon.ip_unknown')}</span>}</dd>
                        <PublicAddressRow hostId={null} name={t('daemon_switcher.local')} />
                        <dt>{t('remote_daemon.route')}</dt>
                        <dd>
                            {settings?.enabled && status.listening
                                ? t('remote_daemon.status_listening', { port: settings.port })
                                : t('remote_daemon.status_off')}
                        </dd>
                    </dl>
                    <div className="remote-pc-acts">
                        {daemonTarget ? (
                            button('open:local', t('remote_daemon.open'), () => open(null), 'externalLink')
                        ) : (
                            <span className="remote-pc-tag">{t('remote_daemon.this_pc_here')}</span>
                        )}
                    </div>
                </li>
                {cards.map(renderCard)}
                <li className="remote-pc-add">
                    <button type="button" className="btn btn-ghost" onClick={() => setInstalling(true)}>
                        <Icon name="plus" size="lg" />
                        <span>{t('remote_daemon.install_linux')}</span>
                    </button>
                </li>
            </ul>

            <h5 className="remote-daemon-subtitle">{t('remote_daemon.clients')}</h5>
            <ul className="remote-daemon-list" aria-label={t('remote_daemon.clients')}>
                {clients.length === 0 && <li className="muted">{t('remote_daemon.no_clients')}</li>}
                {clients.map((c) => (
                    <li key={c.hostId} className="remote-daemon-row">
                        <Icon name="desktop" size="sm" />
                        <span className="remote-daemon-name">{pcName(c)}</span>
                        {button(`revoke:${c.hostId}`, t('remote_daemon.revoke'), () => revoke(c.hostId), 'close')}
                    </li>
                ))}
            </ul>

            {daemons.length > 0 && (
                <div className="setting-item">
                    <span className="setting-label">
                        <span className="setting-title">
                            <Icon name="server" size="sm" /> {t('remote_daemon.view')}
                        </span>
                        <span className="setting-description">{t('remote_daemon.view_description')}</span>
                    </span>
                    <CustomDropdown
                        ariaLabel={t('remote_daemon.view')}
                        value={daemonView === 'unified' ? 'unified' : 'switch'}
                        onChange={(value) => updateSettings({ daemonView: value })}
                        options={[
                            { value: 'switch', label: t('remote_daemon.view_switch') },
                            { value: 'unified', label: t('remote_daemon.view_unified') },
                        ]}
                    />
                </div>
            )}

            {notice && (
                <p className="setting-description" role="status">
                    {notice}
                </p>
            )}
            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}

            {removing && (
                <RemoveDialog
                    name={pcName(removing.daemon || removing.peer)}
                    saved={Boolean(removing.daemon)}
                    canUninstall={removingOnline && (removingFamily === 'linux' || removingFamily === 'macos')}
                    summary={[
                        removingFamily ? t(`remote_daemon.os_${removingFamily}`) : '',
                        removingOnline ? t('remote_daemon.online') : t('remote_daemon.offline'),
                        removing.peer?.publicIp || '',
                    ]
                        .filter(Boolean)
                        .join(' · ')}
                    busy={busy === `remove:${removing.hostId}`}
                    onCancel={() => setRemoving(null)}
                    onConfirm={(choice) => remove(removing, choice)}
                />
            )}
            {installing && (
                <InstallDialog onClose={() => setInstalling(false)} onPaired={reloadPcs} errorText={(res) => remoteErrorText(res, t)} />
            )}
        </>
    );
}
