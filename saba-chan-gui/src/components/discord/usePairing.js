import { useCallback, useEffect, useRef, useState } from 'react';

const POLL_MS = 3000;
const SUCCESS_HOLD_MS = 2000;

/** 짝지어 받은 노드 토큰을 데몬에 저장한다. 실패하면 한 번 더 시도하고, 그래도 안 되면 false */
export async function persistNodeToken(token) {
    for (let attempt = 0; attempt < 2; attempt++) {
        try {
            if ((await window.api.saveNodeToken(token)) !== false) return true;
        } catch (e) {
            console.error('[Pairing] node token save failed:', e);
        }
    }
    return false;
}

/**
 * 짝짓기가 끝났을 때 이 PC에 노드 토큰이 저장됐는지. 데몬은 저장한 결과를 `tokenSaved`로 알려 준다
 * (수령 기한이 지나 토큰이 오지 않았거나 쓰지 못하면 false). 예전 데몬은 이 값이 없어 받은 토큰을 직접 저장한다
 */
export async function claimSavedToken(s) {
    if (typeof s?.tokenSaved === 'boolean') return s.tokenSaved;
    if (!s?.nodeToken) return false;
    return window.api?.saveNodeToken ? persistNodeToken(s.nodeToken) : true;
}

/**
 * 릴레이 페어링 흐름: 코드 발급 → 디스코드에서 `/사바쨩 연결` 입력 → 폴링으로 확인.
 * status: idle | issuing | waiting | success | expired | unsaved | error
 */
export function usePairing({ relayUrl, onClaimed, beforeStart }) {
    const [status, setStatus] = useState('idle');
    const [code, setCode] = useState('');
    const [expiresAt, setExpiresAt] = useState(null);
    const [remaining, setRemaining] = useState(0);
    const pollRef = useRef(null);
    const holdRef = useRef(null);
    // start·reset·화면 이탈마다 올린다: 지난 흐름의 늦은 응답이 상태를 바꾸지 않게
    const sessionRef = useRef(0);
    // 만료 시각에 한 번 더 확인하는 함수 (그 직전에 연결됐을 수 있다)
    const finalCheckRef = useRef(null);
    const claimedRef = useRef(onClaimed);
    claimedRef.current = onClaimed;

    const stopPolling = useCallback(() => {
        if (pollRef.current) clearInterval(pollRef.current);
        pollRef.current = null;
    }, []);

    const reset = useCallback(() => {
        sessionRef.current++;
        finalCheckRef.current = null;
        stopPolling();
        if (holdRef.current) clearTimeout(holdRef.current);
        setStatus('idle');
        setCode('');
        setExpiresAt(null);
        setRemaining(0);
    }, [stopPolling]);

    useEffect(
        () => () => {
            sessionRef.current++;
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
            if (left > 0) return false;
            stopPolling();
            const finalCheck = finalCheckRef.current;
            finalCheckRef.current = null;
            if (finalCheck) finalCheck();
            else setStatus('expired');
            return true;
        };
        if (tick()) return;
        const id = setInterval(() => {
            if (tick()) clearInterval(id);
        }, 1000);
        return () => clearInterval(id);
    }, [status, expiresAt, stopPolling]);

    const start = useCallback(async () => {
        stopPolling();
        const session = ++sessionRef.current;
        const live = () => sessionRef.current === session;
        finalCheckRef.current = null;
        setStatus('issuing');
        try {
            // 데몬은 설정(cloud.relay_url)에 저장된 릴레이로만 짝짓는다: 바꾼 주소를 먼저 저장한다
            if (beforeStart && (await beforeStart()) === false) throw new Error('relay url not saved');
            const data = await window.api.relayInitiatePairing({ relayUrl });
            if (!live()) return;
            if (!data || data.error) throw new Error(data?.error || 'no response');
            let settled = false;
            // final: 만료 시각 뒤의 마지막 확인. 그때도 연결되지 않았으면 만료로 끝낸다
            const check = async (final) => {
                let s = null;
                try {
                    s = await window.api.relayPollPairingStatus(data.code, data.pollSecret, relayUrl);
                } catch {
                    /* 네트워크 오류 — 계속 기다린다 */
                }
                if (settled || !live()) return;
                if (s?.status === 'claimed') {
                    settled = true;
                    stopPolling();
                    // 마무리(노드 토큰 저장)가 실패하면 다시 짝지어야 한다: 성공으로 보이지 않는다
                    const ok = (await claimedRef.current?.(s)) !== false;
                    if (!live()) return;
                    if (!ok) {
                        setStatus('unsaved');
                        return;
                    }
                    setStatus('success');
                    holdRef.current = setTimeout(() => {
                        setStatus('idle');
                        setCode('');
                    }, SUCCESS_HOLD_MS);
                } else if (s?.status === 'expired' || final) {
                    settled = true;
                    stopPolling();
                    setStatus('expired');
                }
            };
            finalCheckRef.current = () => check(true);
            setCode(data.code);
            setExpiresAt(data.expiresAt);
            setStatus('waiting');
            pollRef.current = setInterval(() => check(false), POLL_MS);
        } catch (e) {
            if (!live()) return;
            console.error('[Pairing] initiate failed:', e);
            setStatus('error');
        }
    }, [relayUrl, stopPolling, beforeStart]);

    return { status, code, remaining, start, reset, active: status !== 'idle' };
}
