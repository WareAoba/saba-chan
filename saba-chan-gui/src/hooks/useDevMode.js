import { useState, useEffect, useCallback, useRef } from 'react';

/**
 * 히든 키보드 시퀀스로 개발자 모드를 토글하는 훅.
 *
 * 시퀀스: A B B A A B → → ←
 * (키보드: a b b a a b ArrowRight ArrowRight ArrowLeft)
 *
 * 활성화 시 토스트로 알려주고, 같은 시퀀스를 다시 입력하면 비활성화.
 *
 * @returns {boolean} devMode — 현재 개발자 모드 활성 여부
 */
export function useDevMode() {
    const [devMode, setDevMode] = useState(false);
    const bufferRef = useRef([]);
    const timerRef = useRef(null);

    const SEQUENCE = [
        'a', 'b', 'b', 'a', 'a', 'b',
        'ArrowRight', 'ArrowRight', 'ArrowLeft',
    ];

    const resetBuffer = useCallback(() => {
        bufferRef.current = [];
        if (timerRef.current) {
            clearTimeout(timerRef.current);
            timerRef.current = null;
        }
    }, []);

    useEffect(() => {
        const handleKeyDown = (e) => {
            // 입력 필드에 포커스가 있으면 무시
            const tag = e.target.tagName;
            if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return;

            const key = e.key;
            bufferRef.current.push(key);

            // 타이머 리셋 — 3초 내에 시퀀스 완성해야 함
            if (timerRef.current) clearTimeout(timerRef.current);
            timerRef.current = setTimeout(resetBuffer, 3000);

            // 버퍼가 시퀀스보다 길면 앞에서 자름
            if (bufferRef.current.length > SEQUENCE.length) {
                bufferRef.current = bufferRef.current.slice(-SEQUENCE.length);
            }

            // 매칭 확인
            if (bufferRef.current.length === SEQUENCE.length) {
                const match = bufferRef.current.every((k, i) => k === SEQUENCE[i]);
                if (match) {
                    resetBuffer();
                    setDevMode(prev => {
                        const next = !prev;
                        if (next) {
                            window.showToast?.('🔧 Developer Mode ON', 'info', 2000);
                        } else {
                            window.showToast?.('🔧 Developer Mode OFF', 'info', 2000);
                        }
                        return next;
                    });
                }
            }
        };

        window.addEventListener('keydown', handleKeyDown);
        return () => {
            window.removeEventListener('keydown', handleKeyDown);
            if (timerRef.current) clearTimeout(timerRef.current);
        };
    }, [resetBuffer]);

    return devMode;
}
