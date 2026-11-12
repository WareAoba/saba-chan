import { HighlightedContent } from '@gui/components/ConsoleView';
import { sourceText } from '@gui/utils/audit';
import { commandOf, needsConfirm } from '@gui/utils/commandRisk';
import { type ConsoleLevel, consoleStamper, levelOf, withoutOwnStamp } from '@gui/utils/consoleLine';
import {
    commandText,
    consoleQuickCommands,
    defaultServerTab,
    gameNameOf,
    interactionOf,
    playersText,
    primaryAction,
    viewStatus,
} from '@gui/utils/serverView';
import { useEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useApp, useData } from '../store';
import { BackupsPanel } from './BackupsPanel';
import { CheckPanel } from './CheckPanel';
import { CommandsPanel } from './CommandsPanel';
import { InfoPanel } from './InfoPanel';
import { InstanceWords } from './InstanceWords';
import { Icon } from './icons';
import { PlayersPanel } from './PlayersPanel';
import { startPoll } from '@gui/utils/poll';
import { SabaSettings } from './SabaSettings';
import { SchedulePanel } from './SchedulePanel';
import { ServerIcon } from './ServerIcon';
import { SettingsPanel } from './SettingsPanel';
import { Button, Card, errorText, failed, Segments, Sheet, StatusDot } from './ui';
import { useProvision } from './useProvision';
import { useServerControl } from './useServerControl';
import { useStickToBottom } from './useStickToBottom';
import type { Module, Server, ServerPanelProps, WebApi } from '../types';
import type { ProvisionView } from './useProvision';

/** 콘솔 한 줄 (managedConsole). source: input은 사바쨩이 보낸 명령, by는 보낸 쪽 */
interface ConsoleLine {
    id: number;
    /** 데몬이 받은 시각 (unix 초) */
    timestamp?: number;
    level?: string;
    source?: string;
    content: string;
    /** 데몬이 나눈 하이라이트 조각 (`?spans=1`) */
    spans?: unknown;
    by?: { source?: string; userName?: string };
    /** 화면 key. 줄 id는 실행마다 0부터 다시 세어 겹친다 */
    key?: number;
}

const MAX_LINES = 500;
let lineSeq = 0;

// 서버별 보낸 명령 기록 (탭을 오가도 남는다. 화면을 다시 열면 비운다, 앱 콘솔과 같다)
const HISTORY_MAX = 50;
const histories = new Map<string, string[]>();

function ConsolePanel({ api, server, moduleData }: Omit<ServerPanelProps, 'running'>) {
    const { t, i18n } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const { toast } = useApp.getState();
    const [lines, setLines] = useState<ConsoleLine[]>([]);
    const [stdin, setStdin] = useState(false);
    const [input, setInput] = useState('');
    const [sending, setSending] = useState(false);
    const [levels, setLevels] = useState<ConsoleLevel[]>([]);
    const [confirmText, setConfirmText] = useState<string | null>(null); // 위험 명령 확인 대기
    const cursor = useRef(-1); // 기록에서 몇 번째를 보고 있는지 (-1 = 새로 입력 중)
    const draft = useRef('');
    const field = useRef<HTMLInputElement>(null);
    const stamp = useMemo(() => consoleStamper('24h', false, i18n.language), [i18n.language]);
    const levelCounts = useMemo(() => {
        const counts = { warn: 0, error: 0 };
        for (const line of lines) {
            const level = levelOf(line);
            if (level) counts[level]++;
        }
        return counts;
    }, [lines]);
    const shown = useMemo(
        () => (levels.length ? lines.filter((line) => levels.includes(levelOf(line) as ConsoleLevel)) : lines),
        [lines, levels],
    );
    const quick = useMemo(
        () => consoleQuickCommands(moduleData?.commands?.fields, moduleData?.command_aliases),
        [moduleData],
    );
    // 확인을 기다리는 위험 명령의 설명 (모듈이 준 것). 문장 끝 부호는 알림 문구가 붙인다
    const confirmCommand = confirmText ? commandOf(moduleData?.commands?.fields, confirmText) : undefined;
    const confirmWhat = confirmCommand
        ? commandText(t, server.module, confirmCommand).description.replace(/[.。!?！？\s]+$/u, '')
        : '';
    const since = useRef(0);
    const pid = useRef(server.pid);
    pid.current = server.pid;
    const box = useStickToBottom<HTMLPreElement>(shown);

    useEffect(() => {
        let stopped = false;
        since.current = 0;
        setLines([]);
        // 마지막으로 본 콘솔 버퍼 세대와 PID. 세대가 바뀌면 줄 번호가 0부터 다시 시작한다 (앱 useMultiConsole과 같은 규칙)
        let seenEpoch: number | null = null;
        let seenPid: number | null = null;
        let replace = false;
        let diskLoaded = false;
        const poll = async (): Promise<void> => {
            const res = await api.managedConsole(server.id, since.current, 200, { spans: true });
            if (stopped || !Array.isArray(res?.lines)) return;
            const pidChanged = pid.current != null && seenPid !== null && pid.current !== seenPid;
            if (pid.current != null && pid.current > 0) seenPid = pid.current;
            if (typeof res.epoch === 'number') {
                const epochChanged = seenEpoch !== null && res.epoch !== seenEpoch;
                seenEpoch = res.epoch;
                // 옛 번호로 받은 줄은 버리고 처음부터 다시 받는다. 새 프로세스면 이어 붙이고, 같은 프로세스에 다시 붙었으면 그 버퍼로 바꾼다
                if (epochChanged) {
                    since.current = 0;
                    replace = !pidChanged && !diskLoaded;
                    return poll();
                }
            }
            setStdin(!!res.stdin_available);
            // 데몬 시각을 줄 앞에 붙이므로, 게임이 스스로 찍은 줄 앞 시각은 뺀다 (시각이 두 번 보이지 않게)
            const received: ConsoleLine[] = res.lines.map((l: ConsoleLine) => ({
                ...l,
                ...(l.timestamp != null && withoutOwnStamp(l.content, l.spans)),
                key: ++lineSeq,
            }));
            if (res.source === 'disk') {
                // 꺼진 서버의 지난 로그는 since와 상관없이 매번 같은 꼬리가 온다: 한 번만 보이고, 줄 번호(0부터)로 since를 올리지 않는다
                if (!diskLoaded && received.length) {
                    diskLoaded = true;
                    setLines(received);
                }
                return;
            }
            if (received.length) {
                diskLoaded = false;
                const swap = replace;
                replace = false;
                since.current = res.lines[res.lines.length - 1].id + 1;
                setLines((prev) => (swap ? received : [...prev, ...received]).slice(-MAX_LINES));
            }
        };
        const stop = startPoll(poll, 1500);
        return () => {
            stopped = true;
            stop();
        };
    }, [api, server.id]);

    const send = async () => {
        const command = input.trim();
        if (!command) return;
        // 되돌리기 어려운 명령은 한 번 더 눌러야 보낸다 (module.toml risk = "danger", 앱 콘솔과 같다)
        if (needsConfirm(moduleData?.commands?.fields, command) && confirmText !== command) {
            setConfirmText(command);
            return;
        }
        setConfirmText(null);
        let history = histories.get(server.id);
        if (!history) histories.set(server.id, (history = []));
        if (history[history.length - 1] !== command) history.push(command);
        if (history.length > HISTORY_MAX) history.shift();
        cursor.current = -1;
        setSending(true);
        setInput('');
        const res = stdin
            ? await api.managedStdin(server.id, command)
            : await api.executeCommand(server.id, { command });
        setSending(false);
        if (failed(res)) toast(errorText(t, res), 'error');
        else if (!stdin && (res?.message || res?.text)) toast(String(res.message || res.text), 'info');
    };

    /** ↑/↓ 로 보냈던 명령을 다시 꺼낸다 */
    const recall = (e: React.KeyboardEvent) => {
        const history = histories.get(server.id) ?? [];
        if (!history.length || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
        e.preventDefault();
        if (cursor.current === -1) draft.current = input;
        const at = cursor.current;
        const next =
            e.key === 'ArrowUp'
                ? at === -1
                    ? history.length - 1
                    : Math.max(0, at - 1)
                : at === -1 || at >= history.length - 1
                  ? -1
                  : at + 1;
        cursor.current = next;
        setInput(next === -1 ? draft.current : history[next]);
    };

    return (
        <div className="console">
            <fieldset className="console-chips" aria-label={tg('console.level_filter')}>
                <button
                    type="button"
                    className={`console-chip${levels.length ? '' : ' on'}`}
                    aria-pressed={!levels.length}
                    onClick={() => setLevels([])}
                >
                    {tg('console.level_all')}
                </button>
                {(['warn', 'error'] as const).map((level) => (
                    <button
                        key={level}
                        type="button"
                        className={`console-chip level-${level}${levels.includes(level) ? ' on' : ''}`}
                        aria-pressed={levels.includes(level)}
                        onClick={() =>
                            setLevels(levels.includes(level) ? levels.filter((l) => l !== level) : [...levels, level])
                        }
                    >
                        {tg(`console.level_${level}`)} {levelCounts[level]}
                    </button>
                ))}
            </fieldset>
            <pre className="console-lines" ref={box.ref} onScroll={box.onScroll} aria-live="polite">
                {lines.length === 0 ? (
                    <span className="console-empty">{t('console.empty')}</span>
                ) : (
                    shown.map((l) =>
                        l.source === 'input' ? (
                            // 보낸 명령 (#18): "› list · 디스코드 · jace"
                            <div key={l.key} className="line line-input">
                                <span className="echo">› {l.content}</span>
                                {l.by?.source && <span className="echo-by"> · {sourceText(l.by, tg)}</span>}
                            </div>
                        ) : (
                            <div key={l.key} className={`line line-${levelOf(l) ?? 'info'}`}>
                                {stamp && l.timestamp != null && <span className="console-time">{stamp(l.timestamp)}</span>}
                                <HighlightedContent text={l.content} spans={l.spans} />
                            </div>
                        ),
                    )
                )}
                {shown.length < lines.length && (
                    <div className="console-empty">{tg('console.lines_hidden', { count: lines.length - shown.length })}</div>
                )}
            </pre>
            {confirmText && confirmText === input.trim() && (
                <div className="banner banner-error console-confirm" role="alert">
                    <Icon name="alertTriangle" size={16} />
                    <span>
                        {confirmWhat
                            ? t('console.danger_again_described', { description: confirmWhat })
                            : t('console.danger_again', { command: confirmText })}
                    </span>
                </div>
            )}
            {stdin && server.status === 'running' && quick.length > 0 && (
                <fieldset className="console-chips" aria-label={tg('shell.console_quick')}>
                    {quick.slice(0, 6).map((c) => (
                        <button
                            key={c.name}
                            type="button"
                            className={`console-chip${c.risk ? ` risk-${c.risk}` : ''}`}
                            title={commandText(t, server.module, c).description || commandText(t, server.module, c).label}
                            onClick={() => {
                                // 바로 보내지 않고 채워만 둔다: 인자가 필요하거나 위험한 명령도 있으니까
                                cursor.current = -1;
                                setInput(c.inputs?.length ? `${c.name} ` : c.name);
                                field.current?.focus();
                            }}
                        >
                            {c.name}
                            {c.inputs?.length ? ' …' : ''}
                        </button>
                    ))}
                </fieldset>
            )}
            <form
                className="console-input"
                onSubmit={(e) => {
                    e.preventDefault();
                    send();
                }}
            >
                <input
                    ref={field}
                    value={input}
                    onChange={(e) => {
                        cursor.current = -1;
                        setInput(e.target.value);
                    }}
                    onKeyDown={recall}
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

/** 정보 탭 아래: 이름 바꾸기, 서버 삭제 (켜져 있으면 삭제는 막는다. 앱 정보 탭처럼, 사용자 11-12) */
interface ManageSectionProps {
    api: WebApi;
    server: Server;
    running: boolean;
    control: ReturnType<typeof useServerControl>;
}

function ManageSection({ api, server, running, control }: ManageSectionProps) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
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
        if (failed(res)) {
            const code = res?.error_code;
            const text = code === 'name_taken' ? t('settings.name_taken') : code === 'invalid_name' ? tg('servers.name_comma') : errorText(t, res);
            return toast(text, 'error');
        }
        toast(t('settings.renamed'), 'ok');
    };

    return (
        <div className="panel">
            <Card icon="edit" tone="1" title={t('settings.rename')}>
                <form
                    className="inline"
                    onSubmit={(e) => {
                        e.preventDefault();
                        rename();
                    }}
                >
                    <input
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        maxLength={64}
                        aria-label={t('servers.name')}
                    />
                    <Button type="submit" busy={busy} disabled={!changed}>
                        {t('settings.rename')}
                    </Button>
                </form>
            </Card>
            <Card icon="trash" tone="2" title={t('settings.danger_zone')} desc={t('settings.delete_hint')}>
                <Button
                    tone="ghost-danger"
                    disabled={running}
                    busy={control.busy}
                    onClick={async () => {
                        if (await control.remove(server)) openServer(null);
                    }}
                >
                    <Icon name="trash" size={16} /> {t('servers.delete')}
                </Button>
                {running && <small className="hint">{t('settings.stop_first')}</small>}
            </Card>
        </div>
    );
}

/**
 * 서버 탭 (사용자 11-06 #25): 앱과 같은 이름·순서. 콘솔·명령은 모듈과 실행 방식이 지원할 때만(@gui interactionOf).
 * 작업 기록은 PC에만 남아 기록 탭은 없다. 접속자는 탭 대신 머리의 접속 칩이 연다
 */
function serverTabs(server: Server, moduleData: Module | undefined) {
    const interaction = interactionOf(server, moduleData);
    return [
        interaction.console && 'console',
        interaction.commands && 'commands',
        'settings',
        'saba',
        'maintenance',
        'discord',
        'info',
    ].filter((x): x is string => !!x);
}

export function ServerDetail({ api, server }: { api: WebApi; server: Server }) {
    const { t } = useTranslation();
    const { t: tg } = useTranslation('gui');
    const openServer = useApp((s) => s.openServer);
    const modules = useData((s) => s.modules);
    const modulesLoaded = useData((s) => s.modulesLoaded);
    const control = useServerControl(api);
    // 화면 주소 링크(server/<id>/<탭>)로 연 탭. 고르지 않았으면(null) 기본 탭
    const requestedTab = useApp((s) => s.serverTab);
    const [view, setView] = useState<string | null>(requestedTab);
    useEffect(() => {
        if (requestedTab) setView(requestedTab);
    }, [requestedTab]);
    const [playersOpen, setPlayersOpen] = useState(false);
    const moduleData = modules.find((m) => m.name === server.module);
    const provision = useProvision(api, server);
    const failure: Extract<ProvisionView, { state: 'failed' }> | null =
        provision.view?.state === 'failed' ? provision.view : null;
    const provisionFailed = !!failure;
    const status = provisionFailed ? 'provision_failed' : viewStatus(server, moduleData, modulesLoaded);
    const action = provisionFailed ? null : primaryAction(status);
    const players = playersText(server);
    const running = status !== 'stopped';
    const tabs = serverTabs(server, moduleData);
    const tab = view && tabs.includes(view) ? view : defaultServerTab(interactionOf(server, moduleData), server.status);

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
                            <button
                                type="button"
                                className="pill pill-button"
                                aria-label={`${t('detail.players')} ${players}`}
                                onClick={() => setPlayersOpen(true)}
                            >
                                <Icon name="user" size={12} /> {players}
                            </button>
                        )}
                        {status === 'running' && <Uptime startTime={server.start_time} />}
                        <span>{gameNameOf(t, server.module, moduleData)}</span>
                        {server.port ? <span className="pill">:{server.port}</span> : null}
                    </p>
                </div>
                {/* 앱 상세 머리처럼: 다시 시작은 아이콘, 시작·정지는 보통 크기 단추 하나 (사용자 11-12) */}
                <div className="detail-actions">
                    {action === 'stop' && (
                        <button
                            type="button"
                            className="icon-btn"
                            disabled={control.busy}
                            onClick={() => control.restart(server)}
                            aria-label={t('servers.restart')}
                            title={t('servers.restart')}
                        >
                            <Icon name="refresh" size={16} />
                        </button>
                    )}
                    {action === 'start' && (
                        <Button tone="accent" busy={control.busy} onClick={() => control.start(server)}>
                            <Icon name="play" size={16} /> {t('servers.start')}
                        </Button>
                    )}
                    {action === 'stop' && (
                        <Button tone="danger" busy={control.busy} onClick={() => control.stop(server)}>
                            <Icon name="stop" size={16} /> {t('servers.stop')}
                        </Button>
                    )}
                    {action === 'busy' && <Button disabled>{t(`status.${status}`)}</Button>}
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
            <Segments
                value={tab}
                onChange={setView}
                scroll
                ariaLabel={t('detail.tabs')}
                options={tabs.map((id) => ({ value: id, label: tg(`shell.tab_${id}`) }))}
            />
            {tab === 'console' && <ConsolePanel api={api} server={server} moduleData={moduleData} />}
            {tab === 'commands' && (
                <CommandsPanel api={api} server={server} moduleData={moduleData} running={status === 'running'} />
            )}
            {tab === 'settings' && <SettingsPanel api={api} server={server} moduleData={moduleData} running={running} />}
            {tab === 'saba' && <SabaSettings api={api} server={server} running={running} />}
            {tab === 'maintenance' && (
                <>
                    <BackupsPanel api={api} server={server} running={running} />
                    <SchedulePanel api={api} server={server} moduleData={moduleData} />
                </>
            )}
            {tab === 'discord' && <InstanceWords api={api} server={server} />}
            {tab === 'info' && (
                <>
                    <InfoPanel api={api} server={server} moduleData={moduleData} />
                    {/* 서버 점검·포트포워딩 확인은 앱과 같이 정보 탭에 (예전 점검 탭) */}
                    <CheckPanel api={api} server={server} moduleData={moduleData} running={running} />
                    <ManageSection api={api} server={server} running={running} control={control} />
                </>
            )}
            {playersOpen && (
                <Sheet title={t('detail.players')} onClose={() => setPlayersOpen(false)}>
                    <PlayersPanel api={api} server={server} running={status === 'running'} />
                </Sheet>
            )}
        </div>
    );
}
