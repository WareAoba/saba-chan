/**
 * Console Syntax Highlighter
 *
 * 모듈의 syntax_highlight 규칙을 받아 콘솔 로그 라인을 하이라이팅합니다.
 *
 * ## 규칙 형식 (module.toml → API → 여기)
 * ```json
 * { "name": "timestamp", "pattern": "^\\[\\d{2}:\\d{2}:\\d{2}\\]", "color": "#6c7086", "bold": false, "italic": false,
 *   "background": "warn", "underline": false, "priority": 0 }
 * ```
 *
 * - pattern에 `(?P<hl>...)` 또는 `(?<hl>...)` 캡처가 있으면 해당 부분만 하이라이팅
 * - 없으면 전체 매치를 하이라이팅
 * - 겹치면 priority가 큰 규칙이 이기고, 같으면 먼저 선언된 규칙이 이긴다 (이미 칠한 범위는 덮지 않음)
 * - 맨 앞의 `(?i)` 같은 단독 플래그는 JS 플래그로 바꾼다 (JS RegExp는 패턴 안 단독 플래그를 모른다)
 * - background가 시맨틱 토큰이면 글자가 읽히도록 옅게 깐다
 */

// ── Semantic tokens → CSS 색상 매핑 (Catppuccin Mocha 기반) ──
export const TOKEN_COLORS = {
    error: '#f38ba8',
    warn: '#f9e2af',
    warning: '#f9e2af',
    info: '#89b4fa',
    debug: '#6c7086',
    trace: '#6c7086',
    success: '#a6e3a1',
    string: '#a6e3a1',
    number: '#fab387',
    keyword: '#cba6f7',
    comment: '#6c7086',
    timestamp: '#7f849c',
    player: '#89dceb',
    command: '#f5c2e7',
    selector: '#f9e2af',
    ip: '#74c7ec',
    uuid: '#74c7ec',
    path: '#94e2d5',
    dim: '#6c7086',
};

/**
 * 색상 문자열 해석: "#hex" → 그대로, "token" → TOKEN_COLORS 참조
 */
function resolveColor(color) {
    if (!color) return null;
    if (color.startsWith('#') || color.startsWith('rgb')) return color;
    return TOKEN_COLORS[color.toLowerCase()] || null;
}

/** 배경: 직접 준 색은 그대로, 토큰은 20% 정도로 옅게 (#rrggbb + alpha) */
function resolveBackground(bg) {
    if (!bg) return undefined;
    const token = TOKEN_COLORS[bg.toLowerCase()];
    if (token) return `${token}33`;
    return resolveColor(bg) || bg;
}

/** 맨 앞의 단독 플래그 `(?i)`, `(?is)` → JS 플래그 (지원하는 i·s·m만) */
function splitInlineFlags(src) {
    const m = src.match(/^\(\?([a-zA-Z]+)\)/);
    if (!m || /[^ism]/.test(m[1])) return { src, flags: '' };
    return { src: src.slice(m[0].length), flags: m[1] };
}

/**
 * 모듈 규칙 배열을 사전 컴파일합니다.
 * 반환값을 캐싱하여 매 렌더마다 re-compile하지 않도록 합니다.
 *
 * @param {Array} rules - [{ name, pattern, color, bold, italic }]
 * @returns {Array} compiled - [{ regex, color, bold, italic, hasHlGroup }]
 */
function compileRules(rules) {
    if (!rules || rules.length === 0) return [];
    return rules
        .map((rule) => {
            try {
                // Named group (?P<hl>...) → JS 형식 (?<hl>...) 변환
                const { src, flags } = splitInlineFlags(rule.pattern.replace(/\(\?P</g, '(?<'));
                const regex = new RegExp(src, `g${flags}`);
                const hasHlGroup = /\(\?<hl>/.test(src);
                return {
                    name: rule.name,
                    regex,
                    color: resolveColor(rule.color) || rule.color,
                    background: resolveBackground(rule.background),
                    bold: !!rule.bold,
                    italic: !!rule.italic,
                    underline: !!rule.underline,
                    priority: Number.isFinite(rule.priority) ? rule.priority : 0,
                    hasHlGroup,
                };
            } catch (e) {
                console.warn(`[SyntaxHighlight] Invalid pattern "${rule.pattern}" in rule "${rule.name}":`, e);
                return null;
            }
        })
        .filter(Boolean)
        .sort((a, b) => b.priority - a.priority); // 안정 정렬: 같은 priority는 선언 순서 유지
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
                textDecoration: rule.underline ? 'underline' : undefined,
                backgroundColor: rule.background,
                borderRadius: rule.background ? 2 : undefined,
            };

            // 이미 칠해진 범위는 건너뜀 (priority 큰 규칙, 같으면 먼저 선언된 규칙이 먼저 칠한다)
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
