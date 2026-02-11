import { useState, useCallback, useRef } from 'react';

/**
 * 모달 닫기 트랜지션 훅.
 * onClose 호출 시 closing 클래스를 부여하고, 애니메이션 후 실제 닫기를 수행한다.
 *
 * @param {Function} onClose  실제 모달을 언마운트하는 콜백
 * @param {number}   duration 닫기 애니메이션 길이(ms) — CSS와 일치시킬 것
 * @returns {{ isClosing: boolean, requestClose: Function }}
 */
export function useModalClose(onClose, duration = 220) {
    const [isClosing, setIsClosing] = useState(false);
    const timerRef = useRef(null);

    const requestClose = useCallback(() => {
        if (timerRef.current) return; // 이미 닫히는 중
        setIsClosing(true);
        timerRef.current = setTimeout(() => {
            timerRef.current = null;
            setIsClosing(false);
            onClose();
        }, duration);
    }, [onClose, duration]);

    return { isClosing, requestClose };
}
