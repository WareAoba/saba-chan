import { useCallback, useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createTranslateError, safeShowToast } from '../utils/helpers';

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

    // Map of instanceId -> console state
    const [consoles, setConsoles] = useState({});
    // Global z-index counter for window stacking
    const zCounterRef = useRef(100);
    // Map of instanceId -> polling interval id
    const pollingRefs = useRef({});
    // Popout tracking (same as original)
    const [consolePopoutInstanceId, setConsolePopoutInstanceId] = useState(null);

    // ── Helpers ─────────────────────────────────────────────

    const getNextZ = useCallback(() => {
        zCounterRef.current += 1;
        return zCounterRef.current;
    }, []);

    const getDefaultPosition = useCallback((existingCount) => {
        const offset = existingCount * 30;
        return {
            x: 80 + offset,
            y: 60 + offset,
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
        if (!pollingRefs.current[instanceId]) {
            let sinceId = 0;
            let diskLoaded = false;
            pollingRefs.current[instanceId] = setInterval(async () => {
                try {
                    const data = await window.api.managedConsole(instanceId, sinceId, 200);
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
                }
            }, 500);
        }
    }, [consoleBufferRef, getDefaultPosition, getDefaultSize]);

    // ── Close a single console ──────────────────────────────

    const closeConsole = useCallback((instanceId) => {
        // If called without id (legacy compat) close all
        if (!instanceId) {
            for (const id of Object.keys(pollingRefs.current)) {
                clearInterval(pollingRefs.current[id]);
            }
            pollingRefs.current = {};
            setConsoles({});
            return;
        }

        if (pollingRefs.current[instanceId]) {
            clearInterval(pollingRefs.current[instanceId]);
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
            // main.js will send console:popinRequest event → handled in useEffect below
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
        const state = (() => {
            // Read current state synchronously via a ref trick
            let current;
            setConsoles((prev) => { current = prev; return prev; });
            return current?.[instanceId];
        })();

        if (!state || !state.input.trim()) return;

        const cmd = state.input.trim();
        // 입력을 즉시 클리어하여 중복 전송/후속 입력 삭제 방지
        setConsoleInput(instanceId, '');
        try {
            const result = await window.api.managedStdin(instanceId, cmd);
            if (result?.error) {
                console.log('[Console] stdin failed, trying RCON direct:', result.error);
                const rconResult = await window.api.executeCommand(instanceId, {
                    command: cmd,
                    args: {},
                    commandMetadata: { method: 'rcon' },
                });
                if (rconResult?.error) {
                    safeShowToast(translateError(rconResult.error), 'error', 3000);
                } else {
                    const responseText = rconResult?.data?.response || rconResult?.message || '';
                    const lines = [{ id: Date.now(), content: `> ${cmd}`, source: 'STDIN', level: 'INFO' }];
                    if (responseText) {
                        lines.push({ id: Date.now() + 1, content: responseText, source: 'STDOUT', level: 'INFO' });
                    }
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
    }, [setConsoleInput, translateError]);

    // ── Derived: for ServerCard backward compat ─────────────

    // consoleServer → the currently focused (highest z) non-minimized console, or null
    const consoleServer = (() => {
        const entries = Object.entries(consoles);
        if (entries.length === 0) return null;
        // Find highest z-index non-minimized
        let best = null;
        let bestZ = -1;
        for (const [, state] of entries) {
            if (!state.minimized && state.zIndex > bestZ) {
                best = state.server;
                bestZ = state.zIndex;
            }
        }
        return best;
    })();

    // Check if an instance has an open console (for ServerCard active state)
    const isConsoleOpen = useCallback((instanceId) => {
        return !!consoles[instanceId];
    }, [consoles]);

    // ── Cleanup on unmount ──────────────────────────────────

    useEffect(() => {
        return () => {
            for (const id of Object.keys(pollingRefs.current)) {
                clearInterval(pollingRefs.current[id]);
            }
        };
    }, []);

    // ── Popout mode: auto-start on mount ────────────────────

    useEffect(() => {
        if (popoutParams) {
            openConsole(popoutParams.instanceId, popoutParams.name);
        }
    }, [popoutParams, openConsole]);

    // ── Popout open/close events ────────────────────────────

    useEffect(() => {
        if (isPopoutMode) return;
        const handlePopoutOpened = (instanceId) => {
            setConsolePopoutInstanceId(instanceId);
        };
        const handlePopoutClosed = (instanceId) => {
            setConsolePopoutInstanceId((prev) => (prev === instanceId ? null : prev));
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
    }, [isPopoutMode, openConsole]);

    return {
        consoles,
        consoleServer,
        consolePopoutInstanceId,
        setConsolePopoutInstanceId,
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
