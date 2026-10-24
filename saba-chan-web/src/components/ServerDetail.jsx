import { riskOf } from '@gui/utils/commandRisk';
import { gameNameOf, playersText, primaryAction, viewStatus } from '@gui/utils/serverView';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { BackupsPanel } from './BackupsPanel';
import { SettingsPanel } from './SettingsPanel';
import { Button, errorText, failed, Segments, StatusDot } from './ui';
import { useServerControl } from './useServerControl';

const MAX_LINES = 500;

function ConsolePanel({ api, server, moduleData }) {
    const { t } = useTranslation();
    const { toast, ask } = useApp.getState();
    const [lines, setLines] = useState([]);
    const [stdin, setStdin] = useState(false);
    const [input, setInput] = useState('');
    const [sending, setSending] = useState(false);
    const since = useRef(0);
    const box = useRef(null);

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
        poll();
        const timer = setInterval(poll, 1500);
        return () => {
            stopped = true;
            clearInterval(timer);
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
                    lines.map((l) => (
                        <div key={l.id} className={`line line-${l.level || 'info'}`}>
                            {l.content}
                        </div>
                    ))
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

export function ServerDetail({ api, server }) {
    const { t } = useTranslation();
    const openServer = useApp((s) => s.openServer);
    const { modules, modulesLoaded } = useData();
    const control = useServerControl(api);
    const [view, setView] = useState('console');
    const moduleData = modules.find((m) => m.name === server.module);
    const status = viewStatus(server, moduleData, modulesLoaded);
    const action = primaryAction(status);
    const players = playersText(server);

    return (
        <div className="detail">
            <button type="button" className="back" onClick={() => openServer(null)}>
                ← {t('servers.back')}
            </button>
            <header className="detail-head">
                <div>
                    <h2>{server.name}</h2>
                    <p className="card-sub">
                        {gameNameOf(t, server.module, moduleData)}
                        {server.port ? <span className="pill">:{server.port}</span> : null}
                        {players && <span className="pill">👥 {players}</span>}
                    </p>
                </div>
                <StatusDot status={status} />
            </header>
            <div className="detail-actions">
                {action === 'start' && (
                    <Button tone="accent" busy={control.busy} onClick={() => control.start(server)}>
                        {t('servers.start')}
                    </Button>
                )}
                {action === 'stop' && (
                    <>
                        <Button tone="danger" busy={control.busy} onClick={() => control.stop(server)}>
                            {t('servers.stop')}
                        </Button>
                        <Button busy={control.busy} onClick={() => control.restart(server)}>
                            {t('servers.restart')}
                        </Button>
                    </>
                )}
                {action === 'busy' && <Button disabled>{t(`status.${status}`)}</Button>}
            </div>
            <Segments
                value={view}
                onChange={setView}
                options={[
                    { value: 'console', label: t('detail.console') },
                    { value: 'settings', label: t('detail.settings') },
                    { value: 'backups', label: t('detail.backups') },
                ]}
            />
            {view === 'console' && <ConsolePanel api={api} server={server} moduleData={moduleData} />}
            {view === 'settings' && (
                <SettingsPanel
                    api={api}
                    server={server}
                    moduleData={moduleData}
                    running={status !== 'stopped'}
                    onDeleted={() => openServer(null)}
                    control={control}
                />
            )}
            {view === 'backups' && <BackupsPanel api={api} server={server} running={status !== 'stopped'} />}
        </div>
    );
}
