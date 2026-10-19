import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { TITLEBAR_HEIGHT } from '../constants/layout';
import { useConsoleStore } from '../stores/useConsoleStore';
import { useServerStore } from '../stores/useServerStore';
import { useSettingsStore } from '../stores/useSettingsStore';
import { commandReplyLines, isNoCommandPath } from '../utils/commandReply';
import { createTranslateError, safeShowToast } from '../utils/helpers';

/** @constant 서버 Running 시 polling 간격 (ms) */
const POLL_ACTIVE_MS = 500;
/** @constant 서버 비활성 시 polling 간격 (ms) */
const POLL_IDLE_MS = 5000;

// Map of instanceId -> console state. 이 훅을 쓰는 App이 줄마다 다시 그려지지 않도록 구독하지 않고 쓰기만 한다
const setConsoles = (consolesOrUpdater) => useConsoleStore.getState().setConsoles(consolesOrUpdater);

/**
 * Manages multiple simultaneous console instances with individual state.
 *
 * Each console instance tracks its own lines, input, polling, window position/size,
 * minimized state, pinned state, and z-index for focus ordering.
 *
 * @param {Object} params
 * @param {boolean} params.isPopoutMode - Whether the app is in popout console mode
 * @param {Object|null} params.popoutParams - { instanceId, name } if popout mode
 * @param {React.MutableRefObject<number>} params.consoleBufferRef - Max console lines ref
 * @returns {Object} Multi-console state and handlers
 */
export function useMultiConsole({ isPopoutMode, popoutParams, consoleBufferRef }) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    // Global z-index counter for window stacking
    const zCounterRef = useRef(100);
    // Map of instanceId -> polling interval id
    const pollingRefs = useRef({});
    // 따로 띄운 창에 가 있는 콘솔들 (여러 서버를 동시에 띄울 수 있다)
    const [consolePopoutIds, setConsolePopoutIds] = useState([]);

    // ── Helpers ─────────────────────────────────────────────

    const getNextZ = useCallback(() => {
        zCounterRef.current += 1;
        return zCounterRef.current;
    }, []);

    const getDefaultPosition = useCallback((existingCount) => {
        const offset = existingCount * 30;
        return {
            x: 80 + offset,
            y: TITLEBAR_HEIGHT + 20 + offset,
        };
    }, []);

    const getDefaultSize = useCallback(() => ({
        width: 700,
        height: 400,
    }), []);

    // ── Open a console ──────────────────────────────────────

    const openConsole = useCallback((instanceId, serverName) => {
        setConsoles((prev) => {
            // Already open → just restore and focus
            if (prev[instanceId]) {
                return {
                    ...prev,
                    [instanceId]: {
                        ...prev[instanceId],
                        minimized: false,
                        zIndex: zCounterRef.current += 1,
                    },
                };
            }

            const existingCount = Object.keys(prev).length;
            return {
                ...prev,
                [instanceId]: {
                    server: { id: instanceId, name: serverName },
                    lines: [],
                    input: '',
                    stdinDisabled: false,
                    minimized: false,
                    pinned: false,
                    position: getDefaultPosition(existingCount),
                    size: getDefaultSize(),
                    zIndex: zCounterRef.current += 1,
                },
            };
        });

        // Start polling if not already running
        // 서버 상태에 따라 polling 간격을 동적으로 조절합니다:
        //   Running (PID 추적 중) → 500ms (빠른 응답)
        //   Stopped / 기타          → 5000ms (리소스 절약)
        if (!pollingRefs.current[instanceId]) {
            let sinceId = 0;
            // 마지막으로 본 실행 PID. 바뀌면 새 프로세스라 줄 번호가 0부터 다시 시작한다
            let seenPid = null;
            let diskLoaded = false;
            let polling = false;
            // 이 루프의 ref. 닫았다 다시 열면 새 ref가 생기므로, 옛 루프는 자기 ref가 아니면 멈춘다(줄 중복 방지)
            const myRef = { active: true, timer: null };
            const isMine = () => pollingRefs.current[instanceId] === myRef && myRef.active;
            const schedulePoll = () => {
                if (!isMine()) return;
                const servers = useServerStore.getState().servers;
                const server = servers.find((s) => s.id === instanceId);
                const isRunning = server && server.pid > 0;
                const interval = isRunning ? POLL_ACTIVE_MS : POLL_IDLE_MS;
                myRef.timer = setTimeout(async () => {
                    if (polling) { schedulePoll(); return; }
                    polling = true;
                    try {
                        const pid = useServerStore.getState().servers.find((s) => s.id === instanceId)?.pid;
                        if (pid > 0) {
                            if (seenPid !== null && pid !== seenPid) sinceId = 0;
                            seenPid = pid;
                        }
                        // 하이라이트가 켜져 있으면 데몬에게 조각 토큰(spans)도 달라고 한다
                        const opts = useSettingsStore.getState().consoleSyntaxHighlight ? { spans: true } : undefined;
                        const data = await window.api.managedConsole(instanceId, sinceId, 200, opts);
                        if (!isMine()) return;
                        // disk fallback은 since_id를 무시하고 매번 같은 로그를 반환하므로
                        // 이미 한 번 로드했으면 다시 append하지 않음
                        if (data?.source === 'disk') {
                            if (!diskLoaded && data.lines?.length > 0) {
                                diskLoaded = true;
                                setConsoles((prev) => {
                                    if (!prev[instanceId]) return prev;
                                    const maxLines = consoleBufferRef.current || 2000;
                                    const newLines = [...prev[instanceId].lines, ...data.lines];
                                    return {
                                        ...prev,
                                        [instanceId]: {
                                            ...prev[instanceId],
                                            lines: newLines.length > maxLines ? newLines.slice(-maxLines) : newLines,
                                        },
                                    };
                                });
                            }
                        } else if (data?.lines?.length > 0) {
                            diskLoaded = false; // managed process가 복구됨 → 리셋
                            setConsoles((prev) => {
                                if (!prev[instanceId]) return prev;
                                const maxLines = consoleBufferRef.current || 2000;
                                const newLines = [...prev[instanceId].lines, ...data.lines];
                                return {
                                    ...prev,
                                    [instanceId]: {
                                        ...prev[instanceId],
                                        lines: newLines.length > maxLines ? newLines.slice(-maxLines) : newLines,
                                    },
                                };
                            });
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
                    } catch (_err) {
                        // silent — server might not be ready yet
                    } finally {
                        polling = false;
                    }
                    schedulePoll();
                }, interval);
            };
            pollingRefs.current[instanceId] = myRef;
            schedulePoll();
        }
    }, [consoleBufferRef, getDefaultPosition, getDefaultSize]);

    // ── Close a single console ──────────────────────────────

    const closeConsole = useCallback((instanceId) => {
        // If called without id (legacy compat) close all
        if (!instanceId) {
            for (const id of Object.keys(pollingRefs.current)) {
                const ref = pollingRefs.current[id];
                if (ref && typeof ref === 'object') {
                    ref.active = false;
                    clearTimeout(ref.timer);
                } else {
                    clearInterval(ref);
                }
            }
            pollingRefs.current = {};
            setConsoles({});
            return;
        }

        if (pollingRefs.current[instanceId]) {
            const ref = pollingRefs.current[instanceId];
            if (ref && typeof ref === 'object') {
                ref.active = false;
                clearTimeout(ref.timer);
            } else {
                clearInterval(ref);
            }
            delete pollingRefs.current[instanceId];
        }
        setConsoles((prev) => {
            const next = { ...prev };
            delete next[instanceId];
            return next;
        });
    }, []);

    // ── Minimize / Restore ──────────────────────────────────

    const minimizeConsole = useCallback((instanceId) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            return {
                ...prev,
                [instanceId]: { ...prev[instanceId], minimized: true },
            };
        });
    }, []);

    const restoreConsole = useCallback((instanceId) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            const isPinned = prev[instanceId].pinned;
            return {
                ...prev,
                [instanceId]: {
                    ...prev[instanceId],
                    minimized: false,
                    zIndex: (isPinned ? PIN_Z_OFFSET : 0) + (zCounterRef.current += 1),
                },
            };
        });
    }, []);

    // ── Focus (bring to front) ──────────────────────────────

    const PIN_Z_OFFSET = 50000; // pinned windows sit far above normal ones but below modals

    const focusConsole = useCallback((instanceId) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            const isPinned = prev[instanceId].pinned;
            return {
                ...prev,
                [instanceId]: {
                    ...prev[instanceId],
                    zIndex: (isPinned ? PIN_Z_OFFSET : 0) + (zCounterRef.current += 1),
                },
            };
        });
    }, []);

    // ── Pin (always on top) ─────────────────────────────────

    const togglePin = useCallback((instanceId) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            const wasPinned = prev[instanceId].pinned;
            const newPinned = !wasPinned;
            return {
                ...prev,
                [instanceId]: {
                    ...prev[instanceId],
                    pinned: newPinned,
                    zIndex: newPinned
                        ? PIN_Z_OFFSET + (zCounterRef.current += 1)
                        : (zCounterRef.current += 1),
                },
            };
        });
    }, []);

    // ── Popin (bring PiP back into app) ─────────────────────

    const popinConsole = useCallback(async (instanceId) => {
        try {
            await window.api.consolePopin(instanceId);
            // the shell sends console:popinRequest → handled in useEffect below
        } catch (err) {
            console.error('Popin failed:', err);
        }
    }, []);

    // ── Update position / size ──────────────────────────────

    const updatePosition = useCallback((instanceId, position) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            return {
                ...prev,
                [instanceId]: { ...prev[instanceId], position },
            };
        });
    }, []);

    const updateSize = useCallback((instanceId, size) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            return {
                ...prev,
                [instanceId]: { ...prev[instanceId], size },
            };
        });
    }, []);

    // ── Update input ────────────────────────────────────────

    const setConsoleInput = useCallback((instanceId, value) => {
        setConsoles((prev) => {
            if (!prev[instanceId]) return prev;
            return {
                ...prev,
                [instanceId]: { ...prev[instanceId], input: value },
            };
        });
    }, []);

    // ── Send command ────────────────────────────────────────

    const sendConsoleCommand = useCallback(async (instanceId) => {
        const state = useConsoleStore.getState().consoles[instanceId];

        if (!state || !state.input.trim()) return;

        const cmd = state.input.trim();
        // 입력을 즉시 클리어하여 중복 전송/후속 입력 삭제 방지
        setConsoleInput(instanceId, '');
        try {
            const result = await window.api.managedStdin(instanceId, cmd);
            if (result?.error_code === 'ETIMEDOUT') {
                // 데몬이 이미 보냈을 수 있다: 명령 API로 다시 보내면 두 번 실행된다
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
                        isNoCommandPath(cmdResult) ? t('servers.no_command_path') : translateError(cmdResult.error),
                        'error',
                        5000,
                    );
                } else {
                    const lines = [{ id: Date.now(), content: `> ${cmd}`, source: 'STDIN', level: 'INFO' }];
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
                                lines: [...prev[instanceId].lines, ...lines],
                            },
                        };
                    });
                }
            }
        } catch (err) {
            safeShowToast(translateError(err.message), 'error', 3000);
        }
    }, [setConsoleInput, translateError, t]);

    // Check if an instance has an open console (for ServerCard active state)
    const isConsoleOpen = useCallback((instanceId) => {
        return !!useConsoleStore.getState().consoles[instanceId];
    }, []);

    // ── Cleanup on unmount ──────────────────────────────────

    useEffect(() => {
        return () => {
            for (const id of Object.keys(pollingRefs.current)) {
                const ref = pollingRefs.current[id];
                if (ref && typeof ref === 'object') {
                    ref.active = false;
                    clearTimeout(ref.timer);
                } else {
                    clearInterval(ref);
                }
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
            (ids) => {
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
        const handlePopoutOpened = (instanceId) => {
            setConsolePopoutIds((prev) => (prev.includes(instanceId) ? prev : [...prev, instanceId]));
            // 띄운 창이 따로 읽으니 이 창은 읽기를 멈춘다. 돌아오면 상세 화면이 다시 연다
            closeConsole(instanceId);
        };
        const handlePopoutClosed = (instanceId) => {
            setConsolePopoutIds((prev) => prev.filter((id) => id !== instanceId));
        };
        const handlePopinRequest = (instanceId, serverName) => {
            // PiP → 인앱 전환: 인앱 콘솔 창 열기
            openConsole(instanceId, serverName);
        };
        if (window.api.onConsolePopoutOpened) window.api.onConsolePopoutOpened(handlePopoutOpened);
        if (window.api.onConsolePopoutClosed) window.api.onConsolePopoutClosed(handlePopoutClosed);
        if (window.api.onConsolePopinRequest) window.api.onConsolePopinRequest(handlePopinRequest);
        return () => {
            if (window.api.offConsolePopoutOpened) window.api.offConsolePopoutOpened();
            if (window.api.offConsolePopoutClosed) window.api.offConsolePopoutClosed();
            if (window.api.offConsolePopinRequest) window.api.offConsolePopinRequest();
        };
    }, [isPopoutMode, openConsole, closeConsole]);

    return {
        consolePopoutIds,
        openConsole,
        closeConsole,
        minimizeConsole,
        restoreConsole,
        focusConsole,
        togglePin,
        popinConsole,
        updatePosition,
        updateSize,
        setConsoleInput,
        sendConsoleCommand,
        isConsoleOpen,
    };
}
