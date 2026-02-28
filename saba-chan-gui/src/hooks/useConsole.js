import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { createTranslateError, safeShowToast } from '../utils/helpers';

/**
 * Manages console panel state and operations (open/close/send, polling, popout).
 *
 * @param {Object} params
 * @param {boolean} params.isPopoutMode - Whether the app is in popout console mode
 * @param {Object|null} params.popoutParams - { instanceId, name } if popout mode
 * @param {React.MutableRefObject<number>} params.consoleBufferRef - Max console lines ref
 * @returns {Object} Console state and handlers
 */
export function useConsole({ isPopoutMode, popoutParams, consoleBufferRef }) {
    const { t } = useTranslation('gui');
    const translateError = createTranslateError(t);

    const [consoleServer, setConsoleServer] = useState(null);
    const [consoleLines, setConsoleLines] = useState([]);
    const [consoleInput, setConsoleInput] = useState('');
    const consoleEndRef = useRef(null);
    const consolePollingRef = useRef(null);
    const [consolePopoutInstanceId, setConsolePopoutInstanceId] = useState(null);

    const openConsole = (instanceId, serverName) => {
        setConsoleServer({ id: instanceId, name: serverName });
        setConsoleLines([]);
        setConsoleInput('');

        // Start polling
        if (consolePollingRef.current) clearInterval(consolePollingRef.current);
        let sinceId = 0;
        consolePollingRef.current = setInterval(async () => {
            try {
                const data = await window.api.managedConsole(instanceId, sinceId, 200);
                if (data?.lines?.length > 0) {
                    setConsoleLines((prev) => {
                        const newLines = [...prev, ...data.lines];
                        const maxLines = consoleBufferRef.current || 2000;
                        return newLines.length > maxLines ? newLines.slice(-maxLines) : newLines;
                    });
                    sinceId = data.lines[data.lines.length - 1].id + 1;
                }
            } catch (_err) {
                // silent — server might not be ready yet
            }
        }, 500);
    };

    const closeConsole = () => {
        if (consolePollingRef.current) {
            clearInterval(consolePollingRef.current);
            consolePollingRef.current = null;
        }
        setConsoleServer(null);
        setConsoleLines([]);
    };

    const sendConsoleCommand = async () => {
        if (!consoleInput.trim() || !consoleServer) return;
        const cmd = consoleInput.trim();
        try {
            // managed process stdin first
            const result = await window.api.managedStdin(consoleServer.id, cmd);
            if (result?.error) {
                // stdin failed → try RCON direct (bypass Python lifecycle)
                console.log('[Console] stdin failed, trying RCON direct:', result.error);
                const rconResult = await window.api.executeCommand(consoleServer.id, {
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
                    setConsoleLines((prev) => [...prev, ...lines]);
                }
            }
            setConsoleInput('');
        } catch (err) {
            safeShowToast(translateError(err.message), 'error', 3000);
        }
    };

    // Auto-scroll console to bottom on new lines
    // biome-ignore lint/correctness/useExhaustiveDependencies: consoleLines triggers scroll even though not directly referenced in body
    useEffect(() => {
        if (consoleEndRef.current) {
            consoleEndRef.current.scrollIntoView({ behavior: 'smooth' });
        }
    }, [consoleLines]);

    // Cleanup polling on unmount
    useEffect(() => {
        return () => {
            if (consolePollingRef.current) clearInterval(consolePollingRef.current);
        };
    }, []);

    // Popout mode: auto-start console on mount
    // biome-ignore lint/correctness/useExhaustiveDependencies: openConsole intentionally omitted — should only run when popoutParams changes
    useEffect(() => {
        if (popoutParams) {
            openConsole(popoutParams.instanceId, popoutParams.name);
        }
    }, [popoutParams]);

    // Popout open/close events from main process
    useEffect(() => {
        if (isPopoutMode) return;
        const handlePopoutOpened = (instanceId) => {
            setConsolePopoutInstanceId(instanceId);
        };
        const handlePopoutClosed = (instanceId) => {
            setConsolePopoutInstanceId((prev) => (prev === instanceId ? null : prev));
        };
        if (window.api.onConsolePopoutOpened) window.api.onConsolePopoutOpened(handlePopoutOpened);
        if (window.api.onConsolePopoutClosed) window.api.onConsolePopoutClosed(handlePopoutClosed);
        return () => {
            if (window.api.offConsolePopoutOpened) window.api.offConsolePopoutOpened();
            if (window.api.offConsolePopoutClosed) window.api.offConsolePopoutClosed();
        };
    }, [isPopoutMode]);

    return {
        consoleServer,
        consoleLines,
        consoleInput,
        setConsoleInput,
        consoleEndRef,
        consolePopoutInstanceId,
        setConsolePopoutInstanceId,
        openConsole,
        closeConsole,
        sendConsoleCommand,
    };
}
