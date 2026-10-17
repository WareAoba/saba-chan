import { useMemo } from 'react';
import { highlightLine } from '../utils/syntaxHighlight';

/**
 * 단일 콘솔 라인의 content를 하이라이팅하여 렌더링
 */
export function HighlightedContent({ text, compiledRules }) {
    const segments = useMemo(() => highlightLine(text, compiledRules), [text, compiledRules]);

    // 한 줄 전체가 한 규칙에 걸리면 조각이 하나뿐이지만 스타일은 있다
    if (!compiledRules || compiledRules.length === 0 || (segments.length <= 1 && !segments[0]?.style)) {
        return <>{text}</>;
    }

    // 조각의 줄 안 시작 위치를 key로 쓴다 (같은 줄 안에서 겹치지 않는다)
    let offset = 0;
    return (
        <>
            {segments.map((seg) => {
                const at = offset;
                offset += seg.text.length;
                return seg.style ? (
                    <span key={at} style={seg.style}>
                        {seg.text}
                    </span>
                ) : (
                    seg.text
                );
            })}
        </>
    );
}
