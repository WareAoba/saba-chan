/**
 * Console Syntax Highlighter
 *
 * 모듈의 syntax_highlight 규칙을 받아 콘솔 로그 라인을 하이라이팅합니다.
 *
 * ## 규칙 형식 (module.toml → API → 여기)
 * ```json
 * { "name": "timestamp", "pattern": "^\\[\\d{2}:\\d{2}:\\d{2}\\]", "color": "#6c7086", "bold": false, "italic": false }
 * ```
 *
 * - pattern에 `(?P<hl>...)` 또는 `(?<hl>...)` 캡처가 있으면 해당 부분만 하이라이팅
 * - 없으면 전체 매치를 하이라이팅
 * - 규칙은 선언 순서대로 적용되며, 이미 하이라이팅된 범위는 덮어쓰지 않음
 */

// ── Semantic tokens → CSS 색상 매핑 (Catppuccin Mocha 기반) ──
const TOKEN_COLORS = {
    error:     '#f38ba8',
    warn:      '#f9e2af',
    warning:   '#f9e2af',
    info:      '#89b4fa',
    debug:     '#6c7086',
    trace:     '#6c7086',
    success:   '#a6e3a1',
    string:    '#a6e3a1',
    number:    '#fab387',
    keyword:   '#cba6f7',
    comment:   '#6c7086',
    timestamp: '#7f849c',
    player:    '#89dceb',
    command:   '#f5c2e7',
    selector:  '#f9e2af',
    ip:        '#74c7ec',
    uuid:      '#74c7ec',
    path:      '#94e2d5',
    dim:       '#6c7086',
};

/**
 * 색상 문자열 해석: "#hex" → 그대로, "token" → TOKEN_COLORS 참조
 */
function resolveColor(color) {
    if (!color) return null;
    if (color.startsWith('#') || color.startsWith('rgb')) return color;
    return TOKEN_COLORS[color.toLowerCase()] || null;
}

/**
 * 모듈 규칙 배열을 사전 컴파일합니다.
 * 반환값을 캐싱하여 매 렌더마다 re-compile하지 않도록 합니다.
 *
 * @param {Array} rules - [{ name, pattern, color, bold, italic }]
 * @returns {Array} compiled - [{ regex, color, bold, italic, hasHlGroup }]
 */
export function compileRules(rules) {
    if (!rules || rules.length === 0) return [];
    return rules.map(rule => {
        try {
            // Named group (?P<hl>...) → JS 형식 (?<hl>...) 변환
            let src = rule.pattern.replace(/\(\?P</g, '(?<');
            const regex = new RegExp(src, 'g');
            const hasHlGroup = /\(\?<hl>/.test(src);
            return {
                name: rule.name,
                regex,
                color: resolveColor(rule.color) || rule.color,
                bold: !!rule.bold,
                italic: !!rule.italic,
                hasHlGroup,
            };
        } catch (e) {
            console.warn(`[SyntaxHighlight] Invalid pattern "${rule.pattern}" in rule "${rule.name}":`, e);
            return null;
        }
    }).filter(Boolean);
}

/**
 * 단일 로그 라인을 하이라이팅하여 React 엘리먼트 배열로 반환합니다.
 *
 * @param {string} text - 원본 라인 텍스트
 * @param {Array} compiledRules - compileRules()의 반환값
 * @returns {Array} segments - [{ text, style }] (style이 null이면 기본 텍스트)
 */
export function highlightLine(text, compiledRules) {
    if (!compiledRules || compiledRules.length === 0 || !text) {
        return [{ text, style: null }];
    }

    const len = text.length;
    // 각 문자 위치에 대한 스타일 (null = 미지정)
    const styleMap = new Array(len).fill(null);

    for (const rule of compiledRules) {
        rule.regex.lastIndex = 0;
        let match;
        while ((match = rule.regex.exec(text)) !== null) {
            let start, end;
            if (rule.hasHlGroup && match.groups?.hl !== undefined) {
                // hl 그룹의 시작/끝 계산
                const hlValue = match.groups.hl;
                const hlIdx = match[0].indexOf(hlValue);
                start = match.index + hlIdx;
                end = start + hlValue.length;
            } else {
                start = match.index;
                end = start + match[0].length;
            }

            const style = {
                color: rule.color,
                fontWeight: rule.bold ? 600 : undefined,
                fontStyle: rule.italic ? 'italic' : undefined,
            };

            // 이미 칠해진 범위는 건너뜀 (선언 순서 우선)
            for (let i = start; i < end && i < len; i++) {
                if (styleMap[i] === null) {
                    styleMap[i] = style;
                }
            }

            // zero-length match 방지
            if (match[0].length === 0) rule.regex.lastIndex++;
        }
    }

    // 연속된 동일 스타일을 병합하여 segment 배열 생성
    const segments = [];
    let segStart = 0;
    let segStyle = styleMap[0];

    for (let i = 1; i <= len; i++) {
        const cur = i < len ? styleMap[i] : undefined;
        if (cur !== segStyle) {
            segments.push({ text: text.slice(segStart, i), style: segStyle });
            segStart = i;
            segStyle = cur;
        }
    }

    return segments;
}

/**
 * 모듈 이름으로 캐싱된 컴파일 규칙을 관리합니다.
 */
const ruleCache = new Map();

export function getCachedRules(moduleName, rawRules) {
    if (!rawRules || rawRules.length === 0) return [];
    const key = moduleName;
    if (ruleCache.has(key)) return ruleCache.get(key);
    const compiled = compileRules(rawRules);
    ruleCache.set(key, compiled);
    return compiled;
}

export function invalidateRuleCache(moduleName) {
    if (moduleName) {
        ruleCache.delete(moduleName);
    } else {
        ruleCache.clear();
    }
}
