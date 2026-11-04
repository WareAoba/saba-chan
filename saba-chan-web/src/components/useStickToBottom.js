import { useCallback, useLayoutEffect, useRef } from 'react';

/** 맨 아래에서 이만큼(px) 안이면 맨 아래에 붙어 있다고 본다 (줄 높이 한 줄 남짓) */
const SLACK = 24;

/**
 * 콘솔 상자를 새 줄이 올 때 맨 아래로 내린다. 단, 사람이 위로 올려 읽는 중이면 그대로 둔다.
 * 다시 맨 아래까지 내리면 따라가기를 다시 켠다. `ref`는 상자에, `onScroll`은 상자의 스크롤 이벤트에 붙인다.
 */
export function useStickToBottom(content) {
    const ref = useRef(null);
    const stuck = useRef(true);
    const onScroll = useCallback(() => {
        const el = ref.current;
        if (el) stuck.current = el.scrollHeight - el.scrollTop - el.clientHeight <= SLACK;
    }, []);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 내용이 바뀔 때마다 따라간다
    useLayoutEffect(() => {
        const el = ref.current;
        if (el && stuck.current) el.scrollTop = el.scrollHeight;
    }, [content]);
    return { ref, onScroll };
}
