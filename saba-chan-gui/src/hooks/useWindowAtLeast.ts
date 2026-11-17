import { useEffect, useState } from 'react';

/**
 * 창 너비가 `px` 이상인가. 한 열/두 열 전환 같은 반응형 레이아웃 판단에 쓴다.
 * 경계를 넘을 때만 값이 바뀌므로, 창 크기를 끄는 동안 화면을 계속 다시 그리지 않는다.
 */
export function useWindowAtLeast(px: number) {
    const [atLeast, setAtLeast] = useState(() => window.innerWidth >= px);

    useEffect(() => {
        const handler = () => setAtLeast(window.innerWidth >= px);
        handler();
        window.addEventListener('resize', handler);
        return () => window.removeEventListener('resize', handler);
    }, [px]);

    return atLeast;
}
