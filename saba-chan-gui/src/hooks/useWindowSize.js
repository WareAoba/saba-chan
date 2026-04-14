import { useEffect, useState } from 'react';

/**
 * 윈도우 크기를 추적하는 훅.
 * 디스코드 사이드 패널 등 반응형 레이아웃 조건 판단에 사용.
 */
export function useWindowSize() {
    const [size, setSize] = useState({
        width: window.innerWidth,
        height: window.innerHeight,
    });

    useEffect(() => {
        const handler = () => {
            setSize({ width: window.innerWidth, height: window.innerHeight });
        };
        window.addEventListener('resize', handler);
        return () => window.removeEventListener('resize', handler);
    }, []);

    return size;
}

/** 사이드 패널 표시 조건: 서버 카드 2열 + 사이드 패널 여유분 */
export const SIDE_PANEL_MIN_WIDTH = 1700;
export const SIDE_PANEL_MIN_HEIGHT = 600;
