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
 * - color가 토큰 이름이면 색은 테마가 정한다(tok-<토큰> 클래스, tokens.css --tok-*). #hex는 옛 모듈 호환으로 그대로 칠한다
 *
 * 데몬이 줄마다 spans([[start, end, token]])를 주면 정규식 대신 그것을 쓴다(spanSegments).
 * 정규식은 spans가 없는 동안(데몬이 아직 안 줄 때)의 폴백이다.
 */

// ── 시맨틱 토큰 (데몬 콘솔 토큰 계약과 같은 16개). 색·굵기는 테마(tokens.css --tok-*, shell.css .tok-*)가 정한다 ──
export const TOKENS = [
    'error',
    'warn',
    'info',
    'debug',
    'success',
    'string',
    'number',
    'keyword',
    'timestamp',
    'player',
    'command',
    'selector',
    'ip',
    'uuid',
    'path',
    'dim',
];
// 옛 모듈 값과, 설계 초안(design.md §3.2)의 이름도 확정 어휘로 받는다 (데몬이 어느 쪽을 보내도 칠해지게)
const TOKEN_ALIASES = {
    warning: 'warn',
    trace: 'debug',
    comment: 'dim',
    'level.error': 'error',
    'level.warn': 'warn',
    'level.info': 'info',
    'level.debug': 'debug',
    system: 'info',
    join: 'success',
    leave: 'warn',
    chat: 'string',
    id: 'uuid',
    address: 'ip',
    resource: 'keyword',
    stack: 'dim',
    muted: 'dim',
};

/** 토큰 이름(또는 옛 별칭) → 토큰, 아니면 null */
export function tokenOf(name) {
    if (!name) return null;
    const key = String(name).toLowerCase();
    const token = TOKEN_ALIASES[key] || key;
    return TOKENS.includes(token) ? token : null;
}

/** 배경: 직접 준 색은 그대로, 토큰은 20% 정도로 옅게 */
function resolveBackground(bg) {
    if (!bg) return undefined;
    const token = tokenOf(bg);
    return token ? `color-mix(in oklch, var(--tok-${token}) 20%, transparent)` : bg;
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
                const hasHlGroup = /\(\?<hl>/.test(src);
                // hl 그룹은 d 플래그로 실제 위치를 받는다 (같은 글자가 매치 앞부분에 있어도 맞게)
                const regex = new RegExp(src, `g${hasHlGroup ? 'd' : ''}${flags}`);
                return {
                    name: rule.name,
                    regex,
                    token: tokenOf(rule.color),
                    color: tokenOf(rule.color) ? undefined : rule.color,
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
 * @returns {Array} segments - [{ text, token, style }] (둘 다 없으면 기본 텍스트)
 */
export function highlightLine(text, compiledRules) {
    if (!compiledRules || compiledRules.length === 0 || !text) {
        return [{ text, token: null, style: null }];
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
                [start, end] = match.indices.groups.hl;
            } else {
                start = match.index;
                end = start + match[0].length;
            }

            const extra = {
                color: rule.color,
                fontWeight: rule.bold ? 600 : undefined,
                fontStyle: rule.italic ? 'italic' : undefined,
                textDecoration: rule.underline ? 'underline' : undefined,
                backgroundColor: rule.background,
                borderRadius: rule.background ? 2 : undefined,
            };
            const hasExtra = Object.values(extra).some((v) => v !== undefined);
            const style = { token: rule.token, style: hasExtra ? extra : null };

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
            segments.push({
                text: text.slice(segStart, i),
                token: segStyle?.token || null,
                style: segStyle?.style || null,
            });
            segStart = i;
            segStyle = cur;
        }
    }

    return segments;
}

/**
 * 데몬이 준 spans([[start, end, token]], UTF-16 오프셋, 오름차순·겹침 없음) → 조각.
 * 모르는 토큰이나 범위를 벗어난 구간은 칠하지 않는다.
 */
export function spanSegments(text, spans) {
    if (!text) return [{ text: text || '', token: null, style: null }];
    const out = [];
    let at = 0;
    for (const span of spans || []) {
        if (!Array.isArray(span)) continue;
        const start = Math.max(at, Number(span[0]) || 0);
        const end = Math.min(text.length, Number(span[1]) || 0);
        const token = tokenOf(span[2]);
        if (!token || end <= start) continue;
        if (start > at) out.push({ text: text.slice(at, start), token: null, style: null });
        out.push({ text: text.slice(start, end), token, style: null });
        at = end;
    }
    if (at < text.length) out.push({ text: text.slice(at), token: null, style: null });
    return out;
}

/**
 * 컴파일한 규칙 캐시. 모듈 목록을 다시 받으면 규칙 배열도 새것이라 다시 컴파일한다
 * (모듈을 업데이트해 규칙이 바뀌어도 옛 규칙을 쓰지 않게, 이름이 아니라 배열로 기억한다).
 */
const ruleCache = new WeakMap();

export function getCachedRules(_moduleName, rawRules) {
    if (!rawRules || rawRules.length === 0) return [];
    if (ruleCache.has(rawRules)) return ruleCache.get(rawRules);
    const compiled = compileRules(rawRules);
    ruleCache.set(rawRules, compiled);
    return compiled;
}
