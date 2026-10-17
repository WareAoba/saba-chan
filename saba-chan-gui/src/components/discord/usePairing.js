import { useCallback, useEffect, useRef, useState } from 'react';

const POLL_MS = 3000;
const SUCCESS_HOLD_MS = 2000;

/**
 * 릴레이 페어링 흐름: 코드 발급 → 디스코드에서 `/사바쨩 연결` 입력 → 폴링으로 확인.
 * status: idle | issuing | waiting | success | expired | error
 */
export function usePairing({ relayUrl, onClaimed }) {
    const [status, setStatus] = useState('idle');
    const [code, setCode] = useState('');
    const [expiresAt, setExpiresAt] = useState(null);
    const [remaining, setRemaining] = useState(0);
    const pollRef = useRef(null);
    const holdRef = useRef(null);
    const claimedRef = useRef(onClaimed);
    claimedRef.current = onClaimed;

    const stopPolling = useCallback(() => {
        if (pollRef.current) clearInterval(pollRef.current);
        pollRef.current = null;
    }, []);

    const reset = useCallback(() => {
        stopPolling();
        if (holdRef.current) clearTimeout(holdRef.current);
        setStatus('idle');
        setCode('');
        setExpiresAt(null);
        setRemaining(0);
    }, [stopPolling]);

    useEffect(
        () => () => {
            stopPolling();
            if (holdRef.current) clearTimeout(holdRef.current);
        },
        [stopPolling],
    );

    // 남은 시간
    useEffect(() => {
        if (status !== 'waiting' || !expiresAt) return;
        const tick = () => {
            const left = Math.max(0, Math.floor((new Date(expiresAt).getTime() - Date.now()) / 1000));
            setRemaining(left);
            if (left <= 0) {
                stopPolling();
                setStatus('expired');
            }
        };
        tick();
        const id = setInterval(tick, 1000);
        return () => clearInterval(id);
    }, [status, expiresAt, stopPolling]);

    const start = useCallback(async () => {
        stopPolling();
        setStatus('issuing');
        try {
            const data = await window.api.relayInitiatePairing({ relayUrl });
            if (!data || data.error) throw new Error(data?.error || 'no response');
            setCode(data.code);
            setExpiresAt(data.expiresAt);
            setStatus('waiting');
            pollRef.current = setInterval(async () => {
                try {
                    const s = await window.api.relayPollPairingStatus(data.code, data.pollSecret, relayUrl);
                    if (!s || s.error) return;
                    if (s.status === 'claimed') {
                        stopPolling();
                        setStatus('success');
                        await claimedRef.current?.(s);
                        holdRef.current = setTimeout(() => {
                            setStatus('idle');
                            setCode('');
                        }, SUCCESS_HOLD_MS);
                    } else if (s.status === 'expired') {
                        stopPolling();
                        setStatus('expired');
                    }
                } catch {
                    /* 네트워크 오류 — 계속 기다린다 */
                }
            }, POLL_MS);
        } catch (e) {
            console.error('[Pairing] initiate failed:', e);
            setStatus('error');
        }
    }, [relayUrl, stopPolling]);

    return { status, code, remaining, start, reset, active: status !== 'idle' };
}
