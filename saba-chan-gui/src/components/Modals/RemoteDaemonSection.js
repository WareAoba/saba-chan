import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useSettingsStore } from '../../stores/useSettingsStore';
import CustomDropdown from '../CustomDropdown/CustomDropdown';
import { Icon } from '../Icon';
import { SabaSpinner, SabaToggle } from '../ui/SabaUI';
import { REMOTE_DAEMONS_CHANGED, reloadWindow, useDaemonTarget } from '../shell/DaemonSwitcher';

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
];

/** 화면에 보일 PC 이름: 데몬이 정한 `디스코드 이름(PC 이름)`, 옛 데몬이면 PC 이름 */
export const pcName = (pc) => pc?.label || pc?.name || pc?.hostId || '';

const failed = (res) => !res || res.ok === false || res.success === false || Boolean(res.error_code);

/** 실패 → 한 줄 (아는 코드는 번역, 나머지는 데몬 문구) */
export function remoteErrorText(res, t) {
    const code = String(res?.error_code || res?.error || '');
    if (ERRORS.includes(code)) return t(`remote_daemon.err_${code}`);
    return res?.error || t('remote_daemon.err_generic');
}

/**
 * 설정 > 고급: 원격 데몬 (데몬 `/api/remote/*`, 01 §1.4.2). 이 PC를 내 다른 PC에 열기, 짝 맺은 PC,
 * 내 다른 PC에 붙기, 붙은 원격 데몬. 상단 바에서 다룰 데몬을 고른다. CLI `remote …`와 같은 기능이다.
 * 이 경로들은 고른 데몬과 상관없이 늘 이 PC의 데몬이 답한다. 메인 데몬(`gui.mainDaemon`)은 앱을 열면 먼저 보일 데몬이다.
 */
export function RemoteDaemonSection({ onTargetGone = reloadWindow, onSwitched = reloadWindow, onAsk = null }) {
    const { t } = useTranslation('gui');
    const [settings, setSettings] = useState(null);
    const [draft, setDraft] = useState(null); // {enabled, port, address}
    const [clients, setClients] = useState([]);
    const [peers, setPeers] = useState(null); // null = 못 읽음
    const [peersError, setPeersError] = useState(null);
    const [daemons, setDaemons] = useState([]);
    // 원격 데몬마다 닿는지 (`/api/remote/servers`의 ok·error_code). 아직 모르면 비어 있다
    const [reach, setReach] = useState({});
    const [busy, setBusy] = useState(null); // 진행 중인 동작 이름
    const [error, setError] = useState(null);
    const daemonView = useSettingsStore((st) => st.daemonView);
    const mainDaemon = useSettingsStore((st) => st.mainDaemon);
    const daemonTarget = useDaemonTarget();
    const updateSettings = useSettingsStore((st) => st.update);

    const loadSettings = useCallback(async () => {
        const res = await window.api.remoteSettings();
        if (failed(res)) return;
        setSettings(res);
        setDraft({ enabled: Boolean(res.enabled), port: String(res.port ?? ''), address: res.address || '' });
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
        const next = {};
        for (const d of all?.daemons || []) if (d.hostId) next[d.hostId] = d.ok ? 'ok' : d.error_code || 'remote_unreachable';
        setReach(next);
    }, []);

    useEffect(() => {
        loadSettings();
        loadClients();
        loadPeers();
        loadDaemons();
    }, [loadSettings, loadClients, loadPeers, loadDaemons]);

    const run = async (name, fn) => {
        setBusy(name);
        setError(null);
        try {
            const res = await fn();
            if (failed(res)) setError(remoteErrorText(res, t));
            return res;
        } catch (e) {
            setError(e.message);
            return null;
        } finally {
            setBusy(null);
        }
    };

    const save = async () => {
        const port = Number(draft.port);
        if (!Number.isInteger(port) || port < 1 || port > 65535) {
            setError(t('remote_daemon.err_invalid_port'));
            return;
        }
        const res = await run('save', () =>
            window.api.remoteSettingsSet({ enabled: draft.enabled, port, address: draft.address.trim() }),
        );
        if (res && !failed(res)) {
            setSettings(res);
            setDraft({ enabled: Boolean(res.enabled), port: String(res.port ?? port), address: res.address || '' });
        }
    };

    const connect = async (hostId) => {
        const res = await run(`connect:${hostId}`, () => window.api.remoteConnect(hostId));
        if (res && !failed(res)) {
            await Promise.all([loadPeers(), loadDaemons()]);
            window.dispatchEvent(new Event(REMOTE_DAEMONS_CHANGED));
        }
    };
    const revoke = async (hostId) => {
        const res = await run(`revoke:${hostId}`, () => window.api.remoteRevoke(hostId));
        if (res && !failed(res)) await loadClients();
    };
    // 잊으면 다시 연결해야 쓸 수 있으니 한 번 묻는다
    const askForget = (d) => {
        if (!onAsk) return forget(d.hostId);
        onAsk({
            type: 'question',
            title: t('remote_daemon.forget_confirm_title'),
            message: t('remote_daemon.forget_confirm_message', { name: pcName(d) }),
            confirmText: t('remote_daemon.forget'),
            cancelText: t('remote_daemon.cancel'),
            onConfirm: () => {
                onAsk(null);
                forget(d.hostId);
            },
            onCancel: () => onAsk(null),
        });
    };
    const forget = async (hostId) => {
        const res = await run(`forget:${hostId}`, () => window.api.remoteForget(hostId));
        if (!res || failed(res)) return;
        // 잊은 데몬은 메인일 수 없다: 다음에 열 때 이 PC부터 보인다
        if (mainDaemon === hostId) updateSettings({ mainDaemon: null });
        // 지금 다루던 데몬이면 이 PC로 돌아간다 (남겨 두면 모든 요청이 remote_unknown이다)
        const current = await window.api.daemonTarget?.().catch(() => null);
        if (current?.target === hostId) {
            const back = await window.api.daemonTargetSet(null).catch(() => null);
            if (back?.success) {
                onTargetGone();
                return;
            }
        }
        await Promise.all([loadPeers(), loadDaemons()]);
        window.dispatchEvent(new Event(REMOTE_DAEMONS_CHANGED));
    };

    // 저장한 원격 데몬을 고른다 (상단 바와 같다)
    const open = async (hostId) => {
        const res = await run(`open:${hostId}`, () => window.api.daemonTargetSet(hostId));
        if (res?.success) onSwitched();
    };

    // 저장한 원격 데몬이 지금 못 쓰는 까닭: 서버쨩에서 지워진 PC(gone), 그 PC가 짝을 잊음(unpaired, 데몬을 지우고
    // 다시 설치했을 때. 다시 연결하면 된다), 닿지 않음(unreachable, 꺼졌거나 데몬이 지워졌다). 쓸 수 있으면 null
    const daemonState = (d) => {
        if (peers && !peers.some((p) => p.hostId === d.hostId)) return 'gone';
        const r = reach[d.hostId];
        if (r === 'remote_unauthorized') return 'unpaired';
        if (r && r !== 'ok') return 'unreachable';
        return null;
    };

    const status = settings?.status || {};
    const statusError = settings?.enabled && status.error ? remoteErrorText({ error_code: status.error }, t) : null;
    const button = (name, label, onClick, icon, disabled = false) => (
        <button type="button" className="btn btn-ghost btn-sm" onClick={onClick} disabled={busy !== null || disabled}>
            {busy === name ? <SabaSpinner size="xs" /> : <Icon name={icon} size="sm" />} {label}
        </button>
    );

    return (
        <div className="data-transfer">
            <h4 className="data-transfer-title">
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
                        </label>
                        <SabaToggle
                            id="remote-daemon-enabled"
                            aria-label={t('remote_daemon.enabled')}
                            checked={draft.enabled}
                            disabled={busy !== null}
                            onChange={(enabled) => setDraft({ ...draft, enabled })}
                        />
                    </div>
                    <div className="setting-item">
                        <label className="setting-label" htmlFor="remote-daemon-port">
                            <span className="setting-title">
                                <Icon name="hash" size="sm" /> {t('remote_daemon.port')}
                            </span>
                        </label>
                        <input
                            id="remote-daemon-port"
                            aria-label={t('remote_daemon.port')}
                            type="number"
                            className="setting-input-number"
                            value={draft.port}
                            min={1}
                            max={65535}
                            onChange={(e) => setDraft({ ...draft, port: e.target.value })}
                        />
                    </div>
                    <div className="setting-item">
                        <label className="setting-label" htmlFor="remote-daemon-address">
                            <span className="setting-title">
                                <Icon name="globe" size="sm" /> {t('remote_daemon.address')}
                            </span>
                            <span className="setting-description">{t('remote_daemon.address_description')}</span>
                        </label>
                        <input
                            id="remote-daemon-address"
                            aria-label={t('remote_daemon.address')}
                            type="text"
                            className="setting-input-text"
                            value={draft.address}
                            placeholder={t('remote_daemon.address_placeholder')}
                            onChange={(e) => setDraft({ ...draft, address: e.target.value })}
                        />
                    </div>
                    <div className="data-transfer-actions remote-daemon-actions">
                        <span className="muted" role="status">
                            {!settings.enabled && t('remote_daemon.status_off')}
                            {settings.enabled && status.listening && t('remote_daemon.status_listening', { port: settings.port })}
                        </span>
                        {settings.enabled && status.published && (
                            <span className="muted">{t('remote_daemon.status_published')}</span>
                        )}
                        <button type="button" className="btn btn-primary" onClick={save} disabled={busy !== null}>
                            {busy === 'save' ? <SabaSpinner size="xs" /> : <Icon name="save" size="sm" />}{' '}
                            {t('remote_daemon.save')}
                        </button>
                    </div>
                    {statusError && (
                        <div className="setting-validation-error" role="alert">
                            <Icon name="alertCircle" size="sm" /> {statusError}
                        </div>
                    )}
                </>
            )}

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

            <h5 className="remote-daemon-subtitle">{t('remote_daemon.peers')}</h5>
            {peersError && <p className="setting-description">{peersError}</p>}
            {peers && (
                <ul className="remote-daemon-list" aria-label={t('remote_daemon.peers')}>
                    {peers.length === 0 && <li className="muted">{t('remote_daemon.no_peers')}</li>}
                    {peers.map((p) => (
                        <li key={p.hostId} className="remote-daemon-row">
                            <Icon name="desktop" size="sm" />
                            <span className="remote-daemon-name">{pcName(p)}</span>
                            {p.saved ? (
                                <>
                                    <span className="remote-daemon-meta">{t('remote_daemon.saved')}</span>
                                    {daemonTarget === p.hostId ? (
                                        <span className="remote-daemon-meta">{t('remote_daemon.viewing')}</span>
                                    ) : (
                                        button(
                                            `open:${p.hostId}`,
                                            t('remote_daemon.open'),
                                            () => open(p.hostId),
                                            'externalLink',
                                        )
                                    )}
                                </>
                            ) : p.hasEndpoint ? (
                                button(`connect:${p.hostId}`, t('remote_daemon.connect'), () => connect(p.hostId), 'plug')
                            ) : (
                                <span className="remote-daemon-meta">{t('remote_daemon.no_endpoint')}</span>
                            )}
                        </li>
                    ))}
                </ul>
            )}

            <h5 className="remote-daemon-subtitle">{t('remote_daemon.daemons')}</h5>
            <ul className="remote-daemon-list" aria-label={t('remote_daemon.daemons')}>
                {daemons.length === 0 && <li className="muted">{t('remote_daemon.no_daemons')}</li>}
                {daemons.map((d) => (
                    <li key={d.hostId} className="remote-daemon-row">
                        <Icon name="cloud" size="sm" />
                        {mainDaemon === d.hostId && (
                            <span className="remote-daemon-star" role="img" aria-label={t('remote_daemon.main_badge')} data-tooltip={t('remote_daemon.main_badge')}>
                                <Icon name="star" size="sm" color="var(--star)" />
                            </span>
                        )}
                        <span className="remote-daemon-name">{pcName(d)}</span>
                        {d.address ? (
                            <code className="remote-daemon-meta">{`${d.address}:${d.port}`}</code>
                        ) : (
                            <span className="remote-daemon-meta">{t('remote_daemon.via_relay')}</span>
                        )}
                        {daemonState(d) && <span className="remote-daemon-meta warn">{t(`remote_daemon.state_${daemonState(d)}`)}</span>}
                        {daemonState(d) === 'unpaired' &&
                            button(`connect:${d.hostId}`, t('remote_daemon.reconnect'), () => connect(d.hostId), 'refresh')}
                        {button(`forget:${d.hostId}`, t('remote_daemon.forget'), () => askForget(d), 'trash')}
                    </li>
                ))}
            </ul>

            {daemons.length > 0 && (
                <div className="setting-item">
                    <span className="setting-label">
                        <span className="setting-title">
                            <Icon name="home" size="sm" /> {t('remote_daemon.main')}
                        </span>
                        <span className="setting-description">{t('remote_daemon.main_description')}</span>
                    </span>
                    <CustomDropdown
                        ariaLabel={t('remote_daemon.main')}
                        value={daemons.some((d) => d.hostId === mainDaemon) ? mainDaemon : ''}
                        onChange={(value) => updateSettings({ mainDaemon: value || null })}
                        options={[
                            { value: '', label: t('daemon_switcher.local'), icon: 'desktop' },
                            ...daemons.map((d) => ({ value: d.hostId, label: pcName(d), icon: 'cloud' })),
                        ]}
                    />
                </div>
            )}

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

            {error && (
                <div className="setting-validation-error" role="alert">
                    <Icon name="alertCircle" size="sm" /> {error}
                </div>
            )}
        </div>
    );
}
