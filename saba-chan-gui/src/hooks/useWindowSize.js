import { useEffect, useState } from 'react';

/**
 * 윈도우 크기를 추적하는 훅.
 * 서버 페이지의 한 열/두 열 전환 등 반응형 레이아웃 판단에 사용.
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
