/**
 * Console Syntax Highlighter
 * ===========================
 *
 * 모듈 `module.toml` → 데몬 IPC → GUI 로 전달되는 `syntax_highlight.rules` 배열을
 * 기반으로 콘솔 라인을 하이라이팅한다.
 *
 * ## 규칙 스키마 (입력)
 * ```ts
 * interface Rule {
 *   name: string;            // 고유 식별자
 *   pattern: string;         // 정규식 (Python (?P<hl>...) 허용)
 *   color: string;           // 시맨틱 토큰 | "#rrggbb" | "rgb(...)"
 *   background?: string;     // 배경색 (optional)
 *   bold?: boolean;
 *   italic?: boolean;
 *   underline?: boolean;
 *   priority?: number;       // 기본 0. 큰 값이 먼저 적용됨 (= 우선)
 * }
 * ```
 *
 * ## 설계 원칙
 * - **정확성**: `d` 플래그 + `match.indices` 로 명명 캡처의 절대 위치를 구한다.
 *   `indexOf` 기반의 옛 구현은 hl 값이 매치 내 여러 번 나타날 때 오판하는 버그가 있었다.
 * - **유니코드 안전**: `u` 플래그를 활성화하고 zero-length 매치에서는
 *   코드 포인트 단위로 `lastIndex` 를 전진시킨다.
 * - **세그먼트 병합**: 스타일을 규칙별로 참조 공유하여 동일 규칙의 반복 매치가
 *   동일 스타일 객체를 갖도록 한다 (DOM span 팽창 방지).
 * - **ReDoS 방어**: 컴파일 시 정적 휴리스틱으로 재앙적 백트래킹 패턴 거부,
 *   실행 시 규칙당 매치 횟수 상한을 두고 초장문 라인은 매칭 범위 제한.
 * - **스키마 검증**: 필수 필드 누락·중복 이름·알 수 없는 토큰·잘못된 정규식은
 *   `diagnostics` 배열에 기록되어 UI 노출이 가능하도록 한다.
 * - **캐시**: 원본 `rawRules` 배열 레퍼런스를 키로 `WeakMap` 캐시.
 *   모듈 리로드로 새 배열이 전달되면 자동으로 재컴파일된다.
 */

// ───────────────────────────────────────────────────────────
// 1. 시맨틱 컬러 팔레트 (CSS 변수 fallback 포함)
// ───────────────────────────────────────────────────────────
// 모든 토큰은 `var(--syn-<name>, <fallback>)` 형태로 반환하여
// themeManager 가 라이트/다크 테마별로 값을 재정의할 수 있다.
// Fallback 은 Catppuccin Mocha(다크) 기준.
const TOKEN_DEFAULTS = {
    error:     '#f38ba8',
    critical:  '#eba0ac',
    fatal:     '#eba0ac',
    warn:      '#f9e2af',
    warning:   '#f9e2af',
    notice:    '#fab387',
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
    hostname:  '#74c7ec',
    port:      '#fab387',
    duration:  '#94e2d5',
    bytes:     '#94e2d5',
    path:      '#94e2d5',
    url:       '#89dceb',
    dim:       '#6c7086',
};

const KNOWN_TOKENS = new Set(Object.keys(TOKEN_DEFAULTS));

export const SEMANTIC_TOKENS = Object.freeze({ ...TOKEN_DEFAULTS });

/**
 * 색상 값을 해석한다.
 * @returns {{ value: string | null, known: boolean }}
 *   - value: CSS 로 바로 쓸 수 있는 문자열 (null = 유효하지 않음)
 *   - known: 알려진 토큰 또는 HEX/rgb/hsl 여부 (false = 진단 대상)
 */
function resolveColor(color) {
    if (!color || typeof color !== 'string') return { value: null, known: false };
    const trimmed = color.trim();
    if (!trimmed) return { value: null, known: false };
    if (trimmed.startsWith('#') || /^rgba?\(/i.test(trimmed) || /^hsla?\(/i.test(trimmed)) {
        return { value: trimmed, known: true };
    }
    const key = trimmed.toLowerCase();
    if (KNOWN_TOKENS.has(key)) {
        return { value: `var(--syn-${key}, ${TOKEN_DEFAULTS[key]})`, known: true };
    }
    // 알 수 없는 토큰 — CSS 이름 색상(red 등)이면 브라우저가 해석하지만 진단 필요.
    return { value: trimmed, known: false };
}

// ───────────────────────────────────────────────────────────
// 2. ReDoS 정적 휴리스틱
// ───────────────────────────────────────────────────────────
const MAX_PATTERN_LENGTH = 4000;

/**
 * 명백히 위험한 패턴만 거부한다. 거짓 양성을 줄이기 위해 좁은 검출 기준을 사용.
 * - 길이 상한 초과
 * - 중첩 수량자: `(...)+` 직후 바로 `+*` — 예: `(a+)+`, `(a*)*`
 */
function detectReDoS(pattern) {
    if (pattern.length > MAX_PATTERN_LENGTH) {
        return `pattern too long (${pattern.length} > ${MAX_PATTERN_LENGTH})`;
    }
    if (/\([^()]*[+*][^()]*\)[+*]/.test(pattern)) {
        return 'potential catastrophic backtracking (nested quantifier)';
    }
    return null;
}

// ───────────────────────────────────────────────────────────
// 3. 규칙 컴파일
// ───────────────────────────────────────────────────────────

/**
 * RegExp 컴파일. `u` 플래그 먼저 시도, 실패 시 `u` 없이 재시도.
 */
function tryCompileRegex(pattern) {
    const normalized = pattern.replace(/\(\?P</g, '(?<');
    try {
        return { regex: new RegExp(normalized, 'gdu'), normalized };
    } catch (_e) {
        try {
            return { regex: new RegExp(normalized, 'gd'), normalized };
        } catch (e2) {
            return { error: e2.message };
        }
    }
}

/**
 * 모듈 규칙 배열을 컴파일하여 내부 표현과 진단을 반환한다.
 *
 * @param {Array} rules
 * @returns {{ compiled: Array, diagnostics: Array<{level:'error'|'warn', rule:string|null, message:string}> }}
 */
export function compileRules(rules) {
    const diagnostics = [];
    if (!rules || !Array.isArray(rules) || rules.length === 0) {
        return { compiled: [], diagnostics };
    }

    const seenNames = new Set();
    const compiled = [];

    for (let idx = 0; idx < rules.length; idx++) {
        const rule = rules[idx];

        if (!rule || typeof rule !== 'object') {
            diagnostics.push({ level: 'error', rule: null, message: `Rule at index ${idx} is not an object` });
            continue;
        }

        const name = typeof rule.name === 'string' ? rule.name : '';
        if (!name) {
            diagnostics.push({ level: 'error', rule: null, message: `Rule at index ${idx} missing "name"` });
            continue;
        }

        if (seenNames.has(name)) {
            diagnostics.push({ level: 'warn', rule: name, message: `Duplicate rule name "${name}"` });
        }
        seenNames.add(name);

        if (typeof rule.pattern !== 'string' || rule.pattern.length === 0) {
            diagnostics.push({ level: 'error', rule: name, message: 'Missing or empty pattern' });
            continue;
        }

        const redosReason = detectReDoS(rule.pattern);
        if (redosReason) {
            diagnostics.push({ level: 'error', rule: name, message: `Rejected (ReDoS): ${redosReason}` });
            continue;
        }

        const compiledRegex = tryCompileRegex(rule.pattern);
        if (compiledRegex.error) {
            diagnostics.push({ level: 'error', rule: name, message: `Invalid regex: ${compiledRegex.error}` });
            continue;
        }

        const colorRes = resolveColor(rule.color);
        if (!colorRes.value) {
            diagnostics.push({ level: 'error', rule: name, message: `Missing or invalid color "${rule.color}"` });
            continue;
        }
        if (!colorRes.known) {
            diagnostics.push({ level: 'warn', rule: name, message: `Unknown color token "${rule.color}" (passing through to CSS)` });
        }

        let background = null;
        if (rule.background !== undefined && rule.background !== null && rule.background !== '') {
            const bgRes = resolveColor(rule.background);
            if (bgRes.value) {
                background = bgRes.value;
                if (!bgRes.known) {
                    diagnostics.push({ level: 'warn', rule: name, message: `Unknown background token "${rule.background}"` });
                }
            } else {
                diagnostics.push({ level: 'warn', rule: name, message: `Invalid background "${rule.background}"` });
            }
        }

        const hasHlGroup = /\(\?<hl>/.test(compiledRegex.normalized);
        const priority = Number.isFinite(rule.priority) ? Number(rule.priority) : 0;

        const entry = {
            name,
            regex: compiledRegex.regex,
            color: colorRes.value,
            background,
            bold: !!rule.bold,
            italic: !!rule.italic,
            underline: !!rule.underline,
            priority,
            hasHlGroup,
        };
        // 스타일 객체를 컴파일 시점에 한 번 생성하여 규칙 수명 동안 레퍼런스 공유.
        entry.style = buildStyle(entry);
        compiled.push(entry);
    }

    // 우선순위 내림차순 (Array.prototype.sort 는 stable → 동일 priority 는 선언 순서 유지)
    compiled.sort((a, b) => b.priority - a.priority);

    return { compiled, diagnostics };
}

// ───────────────────────────────────────────────────────────
// 4. 스타일 객체 빌더
// ───────────────────────────────────────────────────────────
function buildStyle(rule) {
    const style = { color: rule.color };
    if (rule.bold) style.fontWeight = 600;
    if (rule.italic) style.fontStyle = 'italic';
    if (rule.underline) style.textDecoration = 'underline';
    if (rule.background) style.backgroundColor = rule.background;
    return Object.freeze(style);
}

// ───────────────────────────────────────────────────────────
// 5. 라인 하이라이팅
// ───────────────────────────────────────────────────────────
const MAX_LINE_MATCH_LENGTH = 16384;
const MAX_MATCHES_PER_RULE  = 1024;

/**
 * 단일 로그 라인을 세그먼트 배열로 변환.
 *
 * @param {string} text
 * @param {Array} compiled - compileRules().compiled
 * @returns {Array<{ text: string, style: object | null }>}
 */
export function highlightLine(text, compiled) {
    if (text === null || text === undefined) {
        return [{ text: '', style: null }];
    }
    if (typeof text !== 'string') {
        text = String(text);
    }
    if (!compiled || compiled.length === 0 || text.length === 0) {
        return [{ text, style: null }];
    }

    const matchText = text.length > MAX_LINE_MATCH_LENGTH ? text.slice(0, MAX_LINE_MATCH_LENGTH) : text;
    const len = text.length;

    // styleIndex[i] = 0 → null, 그 외엔 styles 배열 인덱스
    const styleIndex = new Int32Array(len);
    const styles = [null];
    // 규칙별 styles 배열 내 인덱스 (이번 호출 범위 내에서만 유효)
    const ruleStyleIdx = new Map();

    for (const rule of compiled) {
        rule.regex.lastIndex = 0;
        let iterations = 0;
        let match;
        while ((match = rule.regex.exec(matchText)) !== null) {
            if (++iterations > MAX_MATCHES_PER_RULE) break;

            let start;
            let end;
            if (rule.hasHlGroup && match.indices && match.indices.groups && match.indices.groups.hl) {
                [start, end] = match.indices.groups.hl;
            } else {
                start = match.index;
                end = start + match[0].length;
            }

            if (start < 0) start = 0;
            if (end > len) end = len;

            if (start < end) {
                let idx = ruleStyleIdx.get(rule);
                if (idx === undefined) {
                    idx = styles.length;
                    styles.push(rule.style);
                    ruleStyleIdx.set(rule, idx);
                }
                for (let i = start; i < end; i++) {
                    if (styleIndex[i] === 0) styleIndex[i] = idx;
                }
            }

            if (match[0].length === 0) {
                const pos = rule.regex.lastIndex;
                if (pos >= matchText.length) break;
                const cp = matchText.codePointAt(pos);
                rule.regex.lastIndex += (cp !== undefined && cp > 0xFFFF) ? 2 : 1;
            }
        }
    }

    const segments = [];
    let segStart = 0;
    let segIdx = styleIndex[0];
    for (let i = 1; i <= len; i++) {
        const cur = i < len ? styleIndex[i] : -1;
        if (cur !== segIdx) {
            segments.push({ text: text.slice(segStart, i), style: styles[segIdx] });
            segStart = i;
            segIdx = cur;
        }
    }

    return segments;
}

// ───────────────────────────────────────────────────────────
// 6. 캐시 (WeakMap: rawRules 레퍼런스 기반)
// ───────────────────────────────────────────────────────────
// WeakMap 은 clear() 가 없으므로 mutable 래퍼를 사용하여 테스트에서 리셋 가능.
const cacheHolder = { map: new WeakMap() };

function getEntry(rawRules) {
    if (!rawRules || !Array.isArray(rawRules) || rawRules.length === 0) {
        return { compiled: [], diagnostics: [] };
    }
    let entry = cacheHolder.map.get(rawRules);
    if (!entry) {
        entry = compileRules(rawRules);
        cacheHolder.map.set(rawRules, entry);
        if (entry.diagnostics.length > 0 && typeof console !== 'undefined') {
            const errors = entry.diagnostics.filter((d) => d.level === 'error');
            const warns = entry.diagnostics.filter((d) => d.level === 'warn');
            if (errors.length > 0) {
                // eslint-disable-next-line no-console
                console.warn(
                    `[SyntaxHighlight] ${errors.length} rule(s) rejected, ${warns.length} warning(s). ` +
                    `Use getCompileDiagnostics() to inspect.`,
                    entry.diagnostics,
                );
            }
        }
    }
    return entry;
}

/**
 * @param {string} _moduleName - (사용하지 않음, 외부 호출자와의 하위 호환 유지)
 * @param {Array} rawRules
 * @returns {Array} compiled rules
 */
export function getCachedRules(_moduleName, rawRules) {
    return getEntry(rawRules).compiled;
}

/**
 * 해당 rawRules 의 컴파일 진단 배열을 반환한다.
 * UI 에서 모듈 카드, 설정창 등에 노출 가능.
 */
export function getCompileDiagnostics(_moduleName, rawRules) {
    // moduleName 은 옵션: 단일 인자(rawRules)로 호출해도 동작
    if (Array.isArray(_moduleName) && rawRules === undefined) {
        return getEntry(_moduleName).diagnostics;
    }
    return getEntry(rawRules).diagnostics;
}

/** 테스트 전용 — 캐시 초기화 */
export function __resetRuleCache() {
    cacheHolder.map = new WeakMap();
}
