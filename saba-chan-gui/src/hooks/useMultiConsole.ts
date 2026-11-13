import type React from 'react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type ConsoleLine, type Consoles, useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import type { ApiBody } from '../types/daemon';
import { commandErrorText, commandReplyLines } from '../utils/commandReply';
import { createTranslateError, safeShowToast } from '../utils/helpers';

/** @constant 서버 Running이고 콘솔이 화면에 보일 때 polling 간격 (ms) */
const POLL_ACTIVE_MS = 500;
/** @constant 서버 비활성이거나 콘솔이 화면에 없을 때 polling 간격 (ms) */
const POLL_IDLE_MS = 5000;
// 새 줄이 없으면 데몬이 이만큼(초) 붙잡는다. 붙잡는 데몬(답에 waits: true)에는 답을 받고 POLL_ACTIVE_MS 뒤 다시 묻는다
// 원격 데몬 중계(remote_daemon peer)가 다시 쓴 연결의 답을 10초까지만 기다리므로 그보다 짧게 둔다
const CONSOLE_WAIT_S = 8;
/** 콘솔 스트림을 열지 못했거나 끊긴 뒤 다시 열어 보기까지 (ms). 그동안은 기다리는 조회로 읽는다 */
const STREAM_RETRY_MS = 30_000;
/** PIP로 띄울 때 메인 창 콘솔 자리에 흐리게 남기는 줄 수 */
const GHOST_LINES = 40;

// Map of instanceId -> console state. 이 훅을 쓰는 App이 줄마다 다시 그려지지 않도록 구독하지 않고 쓰기만 한다
type ConsolesOrUpdater = Consoles | ((prev: Consoles) => Consoles);
const setConsoles = (consolesOrUpdater: ConsolesOrUpdater) => useConsoleStore.getState().setConsoles(consolesOrUpdater);

// 줄마다 GUI가 매기는 key. 데몬 id는 프로세스·디스크 읽기마다 0부터 다시 세서 겹친다 (G-C1)
let lineSeq = 0;

/** 읽기 루프 하나의 ref (닫을 때 멈춘다) */
interface PollRef {
    active: boolean;
    timer: ReturnType<typeof setTimeout> | null;
    /** 기다리지 않고 지금 읽는다 (화면에 떴을 때, 창이 다시 보일 때) */
    wake: () => void;
    /** 열린 콘솔 스트림을 닫는다 (화면에 보이는 콘솔만 연다) */
    stream: (() => void) | null;
}

const keyed = (lines: ConsoleLine[]): ConsoleLine[] => lines.map((line) => ({ ...line, key: ++lineSeq }));

/** 디스크 줄과 실행 중에 받은 줄을 맞춰 보는 값. 디스크 기록은 내용 앞 공백을 지우고 시각은 초까지 남긴다 */
const lineMark = (line: ConsoleLine) => `${line.timestamp ?? ''}|${line.source ?? ''}|${String(line.content ?? '').trimStart()}`;

/**
 * 디스크 꼬리에서 이미 본 줄 뒤의 것만 (G-C3). 마지막으로 본 실행 줄들(seen)과 겹치는 곳을
 * 앞에서부터 찾는다: 같은 줄이 되풀이된 로그에서 모호하면 못 본 줄을 버리기보다 다시 보이는 쪽을 고른다.
 * 못 찾으면 다른 기록이라 보고 전부 준다
 */
function unseenDiskTail(seen: string[], tail: ConsoleLine[]) {
    if (seen.length === 0) return tail;
    const marks = tail.map(lineMark);
    for (let end = 0; end < tail.length; end++) {
        const span = Math.min(seen.length, end + 1);
        let match = true;
        for (let k = 0; k < span; k++) {
            if (marks[end - k] !== seen[seen.length - 1 - k]) {
                match = false;
                break;
            }
        }
        if (match) return tail.slice(end + 1);
    }
    return tail;
}

/**
 * Manages multiple simultaneous console instances with individual state.
 *
 * Each console instance tracks its own lines, input and polling.
 *
 * @param {Object} params
 * @param {boolean} params.isPopoutMode - Whether the app is in popout console mode
 * @param {Object|null} params.popoutParams - { instanceId, name } if popout mode
 * @param {React.MutableRefObject<number>} params.consoleBufferRef - Max console lines ref
 * @returns {Object} Multi-console state and handlers
 */
interface UseMultiConsoleParams {
    isPopoutMode: boolean;
    popoutParams: { instanceId: string; name: string } | null;
    consoleBufferRef: React.MutableRefObject<number>;
}

export function useMultiConsole({ isPopoutMode, popoutParams, consoleBufferRef }: UseMultiConsoleParams) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // Map of instanceId -> polling interval id
    const pollingRefs = useRef<Record<string, PollRef>>({});
    // 따로 띄운 창에 가 있는 콘솔들 (여러 서버를 동시에 띄울 수 있다)
    const [consolePopoutIds, setConsolePopoutIds] = useState<string[]>([]);
    // 화면에 떠 있는 콘솔 (instanceId → 그리고 있는 곳 수)
    const watching = useRef<Record<string, number>>({});

    // ── Close a single console ──────────────────────────────

    const closeConsole = useCallback((instanceId: string) => {
        const ref = pollingRefs.current[instanceId];
        if (ref) {
            ref.active = false;
            clearTimeout(ref.timer ?? undefined);
            ref.stream?.();
            delete pollingRefs.current[instanceId];
        }
        setConsoles((prev) => {
            const next = { ...prev };
            delete next[instanceId];
            return next;
        });
    }, []);

    // ── Open a console ──────────────────────────────────────

    const openConsole = useCallback((instanceId: string, serverName: string) => {
        setConsoles((prev) => {
            if (prev[instanceId]) return prev;
            return {
                ...prev,
                [instanceId]: {
                    server: { id: instanceId, name: serverName },
                    lines: [],
                    input: '',
                    stdinDisabled: false,
                },
            };
        });

        // Start polling if not already running
        // 서버 상태에 따라 polling 간격을 동적으로 조절합니다:
        //   Running (PID 추적 중)이고 화면에 보임 → 500ms (빠른 응답)
        //   Stopped / 화면에 없음                  → 5000ms (리소스 절약, since로 이어 받는다)
        //   창이 가려짐                            → 쉰다 (다시 보이면 바로 읽는다)
        if (!pollingRefs.current[instanceId]) {
            let sinceId = 0;
            // 마지막으로 본 실행 PID. 바뀌면 새 프로세스라 줄 번호가 0부터 다시 시작한다
            let seenPid: number | null = null;
            // 마지막으로 본 콘솔 버퍼 세대(epoch). PID가 같아도 데몬이 다시 붙으면 바뀌고 줄 번호가 0부터 다시 시작한다
            let seenEpoch: number | null = null;
            // 세대가 바뀐 뒤 처음 받는 줄은 이어 붙이지 않고 그 버퍼로 바꾼다 (다시 붙은 버퍼는 이미 본 줄을 다시 담는다)
            let replaceLines = false;
            let diskLoaded = false;
            // 실행 중에 받은 마지막 몇 줄의 표시(lineMark). 꺼진 뒤 디스크 꼬리와 겹치는 곳을 찾는 데 쓴다
            let seenTail: string[] = [];
            let polling = false;
            // 마지막 답이 데몬이 붙잡았다 온 것인가 (옛 데몬은 wait를 모르고 바로 답한다)
            let daemonWaits = false;
            // 스트림을 여는 중이거나 열려 있다 (그동안은 주기 조회를 쉰다)
            let streaming = false;
            // 스트림을 열지 못했거나 끊긴 뒤 이때까지는 기다리는 조회로 읽는다
            let streamRetryAt = 0;
            // 연 스트림의 차례. 닫은 뒤에 열린 옛 스트림은 바로 닫는다
            let streamGen = 0;
            // 이 루프의 ref. 닫았다 다시 열면 새 ref가 생기므로, 옛 루프는 자기 ref가 아니면 멈춘다(줄 중복 방지)
            const myRef: PollRef = { active: true, timer: null, wake: () => {}, stream: null };
            const isMine = () => pollingRefs.current[instanceId] === myRef && myRef.active;
            // 목록에서 본 적이 있는 서버가 사라지면 지워진 것이다 (목록을 받기 전에 연 콘솔은 닫지 않는다)
            let listed = false;
            /** 지금 PID를 기억하고, 앞서 본 것과 다른 프로세스인지 */
            const notePid = () => {
                const pid = useServerStore.getState().servers.find((s) => s.id === instanceId)?.pid;
                let changed = false;
                if (pid != null && pid > 0) {
                    if (seenPid !== null && pid !== seenPid) {
                        changed = true;
                        // 세대를 주는 데몬이면 세대 비교가 정한다 (PID가 늦게 바뀌어도 줄이 겹치지 않게)
                        if (seenEpoch === null) sinceId = 0;
                    }
                    seenPid = pid;
                }
                return changed;
            };
            const append = (lines: ConsoleLine[], replace: boolean) =>
                setConsoles((prev) => {
                    if (!prev[instanceId]) return prev;
                    const maxLines = consoleBufferRef.current || 2000;
                    const newLines = replace ? lines : [...prev[instanceId].lines, ...lines];
                    return {
                        ...prev,
                        [instanceId]: {
                            ...prev[instanceId],
                            lines: newLines.length > maxLines ? newLines.slice(-maxLines) : newLines,
                        },
                    };
                });
            /**
             * 콘솔 조회·스트림의 답 한 건을 화면에 넣는다. 조회에서 세대가 바뀌었으면 받은 줄을 버리고 false(처음부터 다시 묻는다).
             * 스트림은 세대가 바뀌면 데몬이 이미 처음부터 보낸다 (`reset`)
             */
            const apply = (data: ApiBody | null | undefined, pidChanged: boolean, fromStream: boolean) => {
                if (!data) return true;
                // epoch가 없는 응답(디스크 기록, 옛 데몬)은 지금처럼 이어 읽는다
                if (typeof data?.epoch === 'number') {
                    const epochChanged = (seenEpoch !== null && data.epoch !== seenEpoch) || data.reset === true;
                    seenEpoch = data.epoch;
                    // 옛 번호로 받은 줄은 버리고 처음부터 다시 받는다. 새 프로세스면 이어 붙이고(지난 실행 기록을 남긴다),
                    // 같은 프로세스에 다시 붙었으면 그 버퍼로 바꾼다
                    if (epochChanged) {
                        sinceId = 0;
                        replaceLines = !pidChanged;
                        if (!fromStream) return false;
                    }
                }
                // disk fallback은 since_id를 무시하고 매번 같은 로그를 반환하므로
                // 이미 한 번 로드했으면 다시 append하지 않음
                if (data?.source === 'disk') {
                    if (!diskLoaded && data.lines?.length > 0) {
                        diskLoaded = true;
                        const unseen = keyed(unseenDiskTail(seenTail, data.lines));
                        seenTail = [];
                        append(unseen, false);
                    }
                } else if (data?.lines?.length > 0) {
                    diskLoaded = false; // managed process가 복구됨 → 리셋
                    const replace = replaceLines;
                    replaceLines = false;
                    seenTail = [...(replace ? [] : seenTail), ...data.lines.map(lineMark)].slice(-5);
                    append(keyed(data.lines), replace);
                    sinceId = data.lines[data.lines.length - 1].id + 1;
                }
                // Track stdin availability from backend
                if (data && typeof data.stdin_available === 'boolean') {
                    setConsoles((prev) => {
                        if (!prev[instanceId]) return prev;
                        const disabled = !data.stdin_available;
                        if (prev[instanceId].stdinDisabled === disabled) return prev;
                        return {
                            ...prev,
                            [instanceId]: {
                                ...prev[instanceId],
                                stdinDisabled: disabled,
                            },
                        };
                    });
                }
                return true;
            };
            const stopStream = () => {
                myRef.stream?.();
                myRef.stream = null;
                streaming = false;
                streamGen++;
            };
            /** 받은 데부터 스트림을 연다. 열지 못하면 한동안 기다리는 조회로 읽는다 */
            const startStream = async () => {
                streaming = true;
                const gen = ++streamGen;
                // 스트림이 없는 브리지(옛 셸·개발용 가짜 API)도 null로 받는다
                const open = () =>
                    window.api.managedConsoleStream(
                        instanceId,
                        {
                            since: sinceId,
                            epoch: seenEpoch ?? undefined,
                            diskSeen: diskLoaded || undefined,
                            spans: useSettingsStore.getState().consoleSyntaxHighlight || undefined,
                        },
                        (data) => {
                            if (isMine()) apply(data, notePid(), true);
                        },
                        () => {
                            if (!isMine() || gen !== streamGen) return;
                            myRef.stream = null;
                            streaming = false;
                            streamRetryAt = Date.now() + STREAM_RETRY_MS;
                            schedulePoll();
                        },
                    );
                const close = await Promise.resolve()
                    .then(open)
                    .then(
                        (c) => (typeof c === 'function' ? c : null),
                        () => null,
                    );
                if (!isMine() || gen !== streamGen) {
                    close?.();
                    return;
                }
                if (!close) {
                    streaming = false;
                    streamRetryAt = Date.now() + STREAM_RETRY_MS;
                    schedulePoll(true);
                    return;
                }
                myRef.stream = close;
            };
            const schedulePoll = (now = false) => {
                if (!isMine()) return;
                clearTimeout(myRef.timer ?? undefined);
                myRef.timer = null;
                // 화면에 보이는 콘솔만 데몬에 붙잡아 달라고 한다 (보이지 않는 콘솔은 5초마다 그냥 읽는다)
                const watched = !!watching.current[instanceId] && !document.hidden;
                if (streaming) {
                    if (watched) return;
                    stopStream();
                }
                if (document.hidden) return;
                const servers = useServerStore.getState().servers;
                const server = servers.find((s) => s.id === instanceId);
                if (server) listed = true;
                else if (listed) {
                    closeConsole(instanceId);
                    return;
                }
                // 화면에 보이면 스트림으로 받는다 (스트림이 없는 데몬·브리지는 아래 기다리는 조회)
                if (watched && !polling && Date.now() >= streamRetryAt) {
                    void startStream();
                    return;
                }
                const isRunning = server && server.pid != null && server.pid > 0;
                const interval = now ? 0 : watched && (isRunning || daemonWaits) ? POLL_ACTIVE_MS : POLL_IDLE_MS;
                const waitFor = now || !watched ? 0 : CONSOLE_WAIT_S;
                myRef.timer = setTimeout(async () => {
                    if (polling) { schedulePoll(); return; }
                    polling = true;
                    try {
                        const pidChanged = notePid();
                        // 하이라이트가 켜져 있으면 데몬에게 조각 토큰(spans)도 달라고 한다
                        const opts = {
                            spans: useSettingsStore.getState().consoleSyntaxHighlight || undefined,
                            wait: waitFor || undefined,
                            epoch: waitFor && seenEpoch !== null ? seenEpoch : undefined,
                            diskSeen: (waitFor && diskLoaded) || undefined,
                        };
                        const data = await window.api.managedConsole(instanceId, sinceId, 200, opts);
                        if (!isMine()) return;
                        daemonWaits = data?.waits === true;
                        if (!apply(data, pidChanged, false)) {
                            polling = false;
                            schedulePoll();
                            return;
                        }
                    } catch (_err) {
                        // silent — server might not be ready yet
                    } finally {
                        polling = false;
                    }
                    schedulePoll();
                }, interval);
            };
            myRef.wake = () => {
                if (!polling) schedulePoll(true);
            };
            pollingRefs.current[instanceId] = myRef;
            // 처음 한 번은 바로 읽는다: 꺼진 서버도 지난 로그가 5초 늦게 뜨지 않게
            schedulePoll(true);
        }
    }, [consoleBufferRef, closeConsole]);

    // ── 화면에 보이는 동안만 빠르게 읽는다 ──────────────────

    const watchConsole = useCallback((instanceId: string) => {
        watching.current[instanceId] = (watching.current[instanceId] ?? 0) + 1;
        pollingRefs.current[instanceId]?.wake();
        return () => {
            watching.current[instanceId] -= 1;
            // 안 보이게 됐으면 스트림을 닫고 느린 조회로 돌아간다
            pollingRefs.current[instanceId]?.wake();
        };
    }, []);

    useEffect(() => {
        // 다시 보이면 바로 읽고, 가려지면 스트림을 닫는다
        const onVisible = () => {
            for (const ref of Object.values(pollingRefs.current)) ref.wake();
        };
        document.addEventListener('visibilitychange', onVisible);
        return () => document.removeEventListener('visibilitychange', onVisible);
    }, []);

    // ── Popin (bring PiP back into app) ─────────────────────

    const popinConsole = useCallback(async (instanceId: string) => {
        try {
            await window.api.consolePopin(instanceId);
            // the shell sends console:popinRequest → handled in useEffect below
        } catch (err) {
            console.error('Popin failed:', err);
        }
    }, []);

    // ── Update input ────────────────────────────────────────

    const setConsoleInput = useCallback((instanceId: string, value: string) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            return {
                ...prev,
                [instanceId]: { ...prev[instanceId], input: value },
            };
        });
    }, []);

    // ── Send command ────────────────────────────────────────

    const sendConsoleCommand = useCallback(async (instanceId: string) => {
        const state = useConsoleStore.getState().consoles[instanceId];

        if (!state || !state.input.trim()) return;

        const cmd = state.input.trim();
        // 입력을 즉시 클리어하여 중복 전송/후속 입력 삭제 방지
        setConsoleInput(instanceId, '');
        try {
            const result = await window.api.managedStdin(instanceId, cmd);
            if (result?.error_code === 'ETIMEDOUT' || result?.error_code === 'EOTHER') {
                // 답을 못 받았다(시간 초과·연결 끊김): 데몬이 이미 보냈을 수 있어 명령 API로 다시 보내면 두 번 실행된다 (G-C4)
                safeShowToast(t('servers.stdin_unconfirmed'), 'warning', 5000);
                return;
            }
            if (result?.error_code === 'console_input_full') {
                // 서버가 앞의 입력을 아직 읽지 않았다: 다른 길로 보내도 같은 대기열이다
                safeShowToast(t('errors.console_input_full'), 'warning', 5000);
                return;
            }
            if (result?.error) {
                console.log('[Console] stdin failed, trying command API:', result.error);
                const cmdResult = await window.api.executeCommand(instanceId, {
                    command: cmd,
                    args: {},
                });
                if (cmdResult?.error) {
                    safeShowToast(
                        commandErrorText(cmdResult, t) ?? translateError(cmdResult.error),
                        'error',
                        5000,
                    );
                } else {
                    // 콘솔 입력이 아니라 명령 API(RCON 등)로 보냈다: 데몬 콘솔에 보낸 줄이 없으니 여기서 같은 모양으로 남긴다
                    const lines: ConsoleLine[] = [{ id: Date.now(), content: cmd, source: 'input', level: 'INFO', by: { source: 'gui' } }];
                    // 데몬이 서버의 답을 data.response로 준다 (여러 줄일 수 있다)
                    commandReplyLines(cmdResult).forEach((content, i) => {
                        lines.push({ id: Date.now() + 1 + i, content, source: 'STDOUT', level: 'INFO' });
                    });
                    setConsoles((prev) => {
                        if (!prev[instanceId]) return prev;
                        return {
                            ...prev,
                            [instanceId]: {
                                ...prev[instanceId],
                                lines: [...prev[instanceId].lines, ...keyed(lines)],
                            },
                        };
                    });
                }
            }
        } catch (err) {
            safeShowToast(translateError((err as Error).message), 'error', 3000);
        }
    }, [setConsoleInput, translateError, t]);

    // ── Cleanup on unmount ──────────────────────────────────

    useEffect(() => {
        return () => {
            for (const ref of Object.values(pollingRefs.current)) {
                ref.active = false;
                clearTimeout(ref.timer ?? undefined);
                ref.stream?.();
            }
        };
    }, []);

    // ── Popout mode: auto-start on mount ────────────────────

    useEffect(() => {
        if (popoutParams) {
            openConsole(popoutParams.instanceId, popoutParams.name);
        }
    }, [popoutParams, openConsole]);

    // ── 메인 창이 새로고침돼도 이미 떠 있는 분리 창을 다시 기억한다 ──

    useEffect(() => {
        if (isPopoutMode || !window.api?.consolePopoutList) return;
        let alive = true;
        window.api.consolePopoutList().then(
            (ids: string[]) => {
                if (!alive || !Array.isArray(ids) || !ids.length) return;
                setConsolePopoutIds((prev) => [...new Set([...prev, ...ids])]);
                for (const id of ids) closeConsole(id);
            },
            () => {},
        );
        return () => {
            alive = false;
        };
    }, [isPopoutMode, closeConsole]);

    // ── Popout open/close events ────────────────────────────

    useEffect(() => {
        if (isPopoutMode) return;
        const handlePopoutOpened = (instanceId: string) => {
            setConsolePopoutIds((prev) => (prev.includes(instanceId) ? prev : [...prev, instanceId]));
            // 메인 창 콘솔 자리에 흐리게 깔 마지막 줄을 남긴다
            const lines = useConsoleStore.getState().consoles[instanceId]?.lines;
            if (Array.isArray(lines) && lines.length) {
                const ghost = lines.slice(-GHOST_LINES).map((l) => String(l?.content ?? ''));
                useConsoleStore.setState((st) => ({ ghosts: { ...st.ghosts, [instanceId]: ghost } }));
            }
            // 띄운 창이 따로 읽으니 이 창은 읽기를 멈춘다. 돌아오면 상세 화면이 다시 연다
            closeConsole(instanceId);
        };
        const handlePopoutClosed = (instanceId: string) => {
            setConsolePopoutIds((prev) => prev.filter((id) => id !== instanceId));
        };
        const handlePopinRequest = (instanceId: string, serverName: string) => {
            // PiP → 인앱 전환: 인앱 콘솔 창 열기
            openConsole(instanceId, serverName);
        };
        const offs = [
            window.api.onConsolePopoutOpened?.(handlePopoutOpened),
            window.api.onConsolePopoutClosed?.(handlePopoutClosed),
            window.api.onConsolePopinRequest?.(handlePopinRequest),
        ];
        return () => {
            for (const off of offs) if (typeof off === 'function') off();
        };
    }, [isPopoutMode, openConsole, closeConsole]);

    return {
        consolePopoutIds,
        openConsole,
        closeConsole,
        watchConsole,
        popinConsole,
        setConsoleInput,
        sendConsoleCommand,
    };
}
