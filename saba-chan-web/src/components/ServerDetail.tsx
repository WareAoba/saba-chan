import { sourceText } from '@gui/utils/audit';
import { riskOf } from '@gui/utils/commandRisk';
import { gameNameOf, playersText, primaryAction, viewStatus } from '@gui/utils/serverView';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { BackupsPanel } from './BackupsPanel';
import { CheckPanel } from './CheckPanel';
import { CommandsPanel } from './CommandsPanel';
import { InfoPanel } from './InfoPanel';
import { InstanceWords } from './InstanceWords';
import { Icon } from './icons';
import { PlayersPanel } from './PlayersPanel';
import { startPoll } from './poll';
import { SabaSettings } from './SabaSettings';
import { ControllersPanel } from './ControllersPanel';
import { SchedulePanel } from './SchedulePanel';
import { ServerIcon } from './ServerIcon';
import { SettingsPanel } from './SettingsPanel';
import { Button, errorText, failed, Segments, Sheet, StatusDot } from './ui';
import { useProvision } from './useProvision';
import { useServerControl } from './useServerControl';
import type { Server, ServerPanelProps, WebApi } from '../types';
import type { ProvisionView } from './useProvision';

/** 콘솔 한 줄 (managedConsole). source: input은 사바쨩이 보낸 명령, by는 보낸 쪽 */
interface ConsoleLine {
    id: number;
    level?: string;
    source?: string;
    content: string;
    by?: { source?: string; userName?: string };
}

const MAX_LINES = 500;

function ConsolePanel({ api, server, moduleData }: Omit<ServerPanelProps, 'running'>) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const { toast, ask } = useApp.getState();
    const [lines, setLines] = useState<ConsoleLine[]>([]);
    const [stdin, setStdin] = useState(false);
    const [input, setInput] = useState('');
    const [sending, setSending] = useState(false);
    const since = useRef(0);
    const box = useRef<HTMLPreElement>(null);

    useEffect(() => {
        let stopped = false;
        since.current = 0;
        setLines([]);
        const poll = async () => {
            const res = await api.managedConsole(server.id, since.current, 200);
            if (stopped || !Array.isArray(res?.lines)) return;
            setStdin(!!res.stdin_available);
            if (res.lines.length) {
                since.current = res.lines[res.lines.length - 1].id + 1;
                setLines((prev) => [...prev, ...res.lines].slice(-MAX_LINES));
            }
        };
        const stop = startPoll(poll, 1500);
        return () => {
            stopped = true;
            stop();
        };
    }, [api, server.id]);

    // biome-ignore lint/correctness/useExhaustiveDependencies: 새 줄이 올 때마다 맨 아래로 내린다
    useEffect(() => {
        const el = box.current;
        if (el) el.scrollTop = el.scrollHeight;
    }, [lines]);

    const send = async () => {
        const command = input.trim();
        if (!command) return;
        if (riskOf(moduleData?.commands?.fields, command) === 'danger') {
            const ok = await ask({
                title: t('console.danger_title'),
                message: t('console.danger_message', { command }),
                danger: true,
            });
            if (!ok) return;
        }
        setSending(true);
        setInput('');
        const res = stdin
            ? await api.managedStdin(server.id, command)
            : await api.executeCommand(server.id, { command });
        setSending(false);
        if (failed(res)) toast(errorText(t, res), 'error');
        else if (!stdin && (res?.message || res?.text)) toast(String(res.message || res.text), 'info');
    };

    return (
        <div className="console">
            <pre className="console-lines" ref={box} aria-live="polite">
                {lines.length === 0 ? (
                    <span className="console-empty">{t('console.empty')}</span>
                ) : (
                    lines.map((l) =>
                        l.source === 'input' ? (
                            // 보낸 명령 (#18): "› list · 디스코드 · jace"
                            <div key={l.id} className="line line-input">
                                <span className="echo">› {l.content}</span>
                                {l.by?.source && <span className="echo-by"> · {sourceText(l.by, tg)}</span>}
                            </div>
                        ) : (
                            <div key={l.id} className={`line line-${l.level || 'info'}`}>
                                {l.content}
                            </div>
                        ),
                    )
                )}
            </pre>
            <form
                className="console-input"
                onSubmit={(e) => {
                    e.preventDefault();
                    send();
                }}
            >
                <input
                    value={input}
                    onChange={(e) => setInput(e.target.value)}
                    placeholder={t('console.placeholder')}
                    autoCapitalize="off"
                    autoCorrect="off"
                    spellCheck={false}
                    enterKeyHint="send"
                />
                <Button tone="accent" type="submit" busy={sending} disabled={!input.trim()}>
                    {t('console.send')}
                </Button>
            </form>
        </div>
    );
}

/** 켜진 지 얼마나 (시작 시각 epoch 초 → "2:14:05") */
export function uptimeText(startTime: number | null, nowMs: number) {
    if (!startTime) return null;
    const left = Math.max(0, Math.floor(nowMs / 1000) - startTime);
    const h = Math.floor(left / 3600);
    const m = String(Math.floor((left % 3600) / 60)).padStart(2, '0');
    const sec = String(left % 60).padStart(2, '0');
    return `${h}:${m}:${sec}`;
}

function Uptime({ startTime }: { startTime: number | null }) {
    const { t } = useTranslation();
    const [now, setNow] = useState(Date.now());
    useEffect(() => {
        const timer = setInterval(() => setNow(Date.now()), 1000);
        return () => clearInterval(timer);
    }, []);
    const text = uptimeText(startTime, now);
    return text ? <span className="pill">{t('detail.uptime', { time: text })}</span> : null;
}

/** 더보기: 이름 바꾸기, 서버 삭제 (켜져 있으면 삭제는 막는다) */
interface MoreSheetProps {
    api: WebApi;
    server: Server;
    running: boolean;
    control: ReturnType<typeof useServerControl>;
    onClose: () => void;
}

function MoreSheet({ api, server, running, control, onClose }: MoreSheetProps) {
    const { t } = useTranslation();
    const toast = useApp((s) => s.toast);
    const openServer = useApp((s) => s.openServer);
    const [name, setName] = useState(server.name);
    const [busy, setBusy] = useState(false);
    const changed = name.trim() && name.trim() !== server.name;

    const rename = async () => {
        if (!changed) return;
        setBusy(true);
        const res = await api.instanceRename(server.id, name.trim());
        setBusy(false);
        if (failed(res))
            return toast(res?.error_code === 'name_taken' ? t('settings.name_taken') : errorText(t, res), 'error');
        toast(t('settings.renamed'), 'ok');
        onClose();
    };

    return (
        <Sheet title={server.name} onClose={onClose}>
            <form
                className="form"
                onSubmit={(e) => {
                    e.preventDefault();
                    rename();
                }}
            >
                <label className="field">
                    <span>{t('servers.name')}</span>
                    <div className="inline">
                        <input value={name} onChange={(e) => setName(e.target.value)} maxLength={64} />
                        <Button type="submit" busy={busy} disabled={!changed}>
                            {t('settings.rename')}
                        </Button>
                    </div>
                </label>
                <div className="field">
                    <span>{t('settings.danger_zone')}</span>
                    <small className="hint">{t('settings.delete_hint')}</small>
                    <Button
                        tone="ghost-danger"
                        disabled={running}
                        busy={control.busy}
                        onClick={async () => {
                            if (await control.remove(server)) {
                                onClose();
                                openServer(null);
                            }
                        }}
                    >
                        <Icon name="trash" size={16} /> {t('servers.delete')}
                    </Button>
                    {running && <small className="hint">{t('settings.stop_first')}</small>}
                </div>
            </form>
        </Sheet>
    );
}

const TABS = ['console', 'commands', 'players', 'settings', 'maintenance', 'controllers', 'check', 'info'];

export function ServerDetail({ api, server }: { api: WebApi; server: Server }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const openServer = useApp((s) => s.openServer);
    const modules = useData((s) => s.modules);
    const modulesLoaded = useData((s) => s.modulesLoaded);
    const control = useServerControl(api);
    const [view, setView] = useState('console');
    const [settingsView, setSettingsView] = useState('game');
    const [more, setMore] = useState(false);
    const moduleData = modules.find((m) => m.name === server.module);
    const provision = useProvision(api, server);
    const failure: Extract<ProvisionView, { state: 'failed' }> | null =
        provision.view?.state === 'failed' ? provision.view : null;
    const provisionFailed = !!failure;
    const status = provisionFailed ? 'provision_failed' : viewStatus(server, moduleData, modulesLoaded);
    const action = provisionFailed ? null : primaryAction(status);
    const players = playersText(server);
    const running = status !== 'stopped';
    const label: Record<string, string> = {
        console: tg('shell.tab_console'),
        commands: tg('shell.tab_commands'),
        players: t('detail.players'),
        settings: tg('shell.tab_settings'),
        maintenance: tg('shell.tab_maintenance'),
        controllers: tg('shell.tab_controllers'),
        check: t('detail.check'),
        info: tg('shell.tab_info'),
    };

    return (
        <div className="detail">
            <button type="button" className="back" onClick={() => openServer(null)}>
                <Icon name="back" size={16} /> {t('servers.back')}
            </button>
            <header className="detail-head">
                <ServerIcon module={server.module} moduleData={moduleData} size={48} />
                <div className="detail-title">
                    <h2>{server.name}</h2>
                    <p className="card-sub">
                        <StatusDot status={status} />
                        {players && (
                            <span className="pill">
                                <Icon name="user" size={12} /> {players}
                            </span>
                        )}
                        {status === 'running' && <Uptime startTime={server.start_time} />}
                        <span>{gameNameOf(t, server.module, moduleData)}</span>
                        {server.port ? <span className="pill">:{server.port}</span> : null}
                    </p>
                </div>
            </header>
            {provision.view?.state === 'running' && (
                <div className="banner banner-info">
                    {t('provision.running', { percent: Math.round(provision.view.percent) })}
                    {provision.view.message ? ` · ${provision.view.message}` : ''}
                </div>
            )}
            {failure && (
                <div className="banner banner-error">
                    <p>{t('provision.failed')}</p>
                    {failure.error && <p className="hint">{failure.error}</p>}
                    {failure.pcNeeded && <p>{t('provision.pc_needed')}</p>}
                    <p className="hint">{t('provision.dismiss_hint')}</p>
                    <Button onClick={provision.dismiss}>{t('provision.dismiss')}</Button>
                </div>
            )}
            <div className="detail-actions">
                {action === 'start' && (
                    <Button tone="accent" busy={control.busy} onClick={() => control.start(server)}>
                        <Icon name="play" size={16} /> {t('servers.start')}
                    </Button>
                )}
                {action === 'stop' && (
                    <>
                        <Button tone="danger" busy={control.busy} onClick={() => control.stop(server)}>
                            <Icon name="stop" size={16} /> {t('servers.stop')}
                        </Button>
                        <Button busy={control.busy} onClick={() => control.restart(server)}>
                            <Icon name="refresh" size={16} /> {t('servers.restart')}
                        </Button>
                    </>
                )}
                {action === 'busy' && <Button disabled>{t(`status.${status}`)}</Button>}
                <button
                    type="button"
                    className="icon-btn more-btn"
                    onClick={() => setMore(true)}
                    aria-label={t('detail.more')}
                >
                    <Icon name="more" />
                </button>
            </div>
            <Segments
                value={view}
                onChange={setView}
                scroll
                ariaLabel={t('detail.tabs')}
                options={TABS.map((id) => ({ value: id, label: label[id] }))}
            />
            {view === 'console' && <ConsolePanel api={api} server={server} moduleData={moduleData} />}
            {view === 'commands' && (
                <CommandsPanel api={api} server={server} moduleData={moduleData} running={status === 'running'} />
            )}
            {view === 'players' && <PlayersPanel api={api} server={server} running={status === 'running'} />}
            {view === 'settings' && (
                <>
                    <Segments
                        value={settingsView}
                        onChange={setSettingsView}
                        ariaLabel={tg('shell.tab_settings')}
                        options={[
                            { value: 'game', label: t('detail.settings_game') },
                            { value: 'saba', label: tg('shell.tab_saba') },
                        ]}
                    />
                    {settingsView === 'game' ? (
                        <SettingsPanel api={api} server={server} moduleData={moduleData} running={running} />
                    ) : (
                        <>
                            <SabaSettings api={api} server={server} running={running} />
                            <InstanceWords api={api} server={server} />
                        </>
                    )}
                </>
            )}
            {view === 'maintenance' && (
                <>
                    <BackupsPanel api={api} server={server} running={running} />
                    <SchedulePanel api={api} server={server} moduleData={moduleData} />
                </>
            )}
            {view === 'controllers' && <ControllersPanel api={api} server={server} />}
            {view === 'check' && <CheckPanel api={api} server={server} moduleData={moduleData} running={running} />}
            {view === 'info' && <InfoPanel api={api} server={server} moduleData={moduleData} />}
            {more && (
                <MoreSheet
                    api={api}
                    server={server}
                    running={running}
                    control={control}
                    onClose={() => setMore(false)}
                />
            )}
        </div>
    );
}
