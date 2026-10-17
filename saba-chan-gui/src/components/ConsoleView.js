import { useMemo } from 'react';
import { highlightLine, spanSegments } from '../utils/syntaxHighlight';

/**
 * 단일 콘솔 라인의 content를 하이라이팅하여 렌더링.
 * 데몬이 준 spans가 있으면 그것을, 없으면 모듈 규칙(compiledRules)을 쓴다. 색은 tok-<토큰> 클래스로 테마가 정한다.
 */
export function HighlightedContent({ text, spans, compiledRules }) {
    const segments = useMemo(
        () => (Array.isArray(spans) ? spanSegments(text, spans) : highlightLine(text, compiledRules)),
        [text, spans, compiledRules],
    );

    // 칠할 조각이 없으면 글자 그대로 (한 줄 전체가 한 조각이어도 토큰·스타일이 있으면 칠한다)
    if (!segments.some((seg) => seg.token || seg.style)) {
        return <>{text}</>;
    }

    // 조각의 줄 안 시작 위치를 key로 쓴다 (같은 줄 안에서 겹치지 않는다)
    let offset = 0;
    return (
        <>
            {segments.map((seg) => {
                const at = offset;
                offset += seg.text.length;
                return seg.token || seg.style ? (
                    <span
                        key={at}
                        className={seg.token ? `tok-${seg.token}` : undefined}
                        style={seg.style || undefined}
                    >
                        {seg.text}
                    </span>
                ) : (
                    seg.text
                );
            })}
        </>
    );
}
