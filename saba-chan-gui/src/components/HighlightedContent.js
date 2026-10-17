import { useMemo } from 'react';
import { highlightLine } from '../utils/syntaxHighlight';

export function HighlightedContent({ text, compiledRules }) {
    return useMemo(() => {
        let offset = 0;
        return highlightLine(text, compiledRules).map((segment) => {
            offset += segment.text.length;
            return segment.style ? <span key={offset} style={segment.style}>{segment.text}</span> : segment.text;
        });
    }, [text, compiledRules]);
}
