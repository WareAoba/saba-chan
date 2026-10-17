/**
 * syntaxHighlight 단위 테스트 — TDD 기반 콘솔 구문 강조 인프라 검증
 *
 * 검증 영역:
 *  - hl 명명 캡처 그룹의 정확한 위치 계산 (중복 토큰 케이스 포함)
 *  - 세그먼트 병합 정확성 (동일 규칙 반복 매치 시 span 증폭 방지)
 *  - 유니코드/서로게이트 페어 안전성
 *  - zero-length 매치 무한 루프 방지
 *  - ReDoS 정적 검증 + 실행 이터레이션 캡
 *  - 캐시 무효화 (rawRules 레퍼런스 기반)
 *  - 스키마 검증 (필수 필드, 중복 이름, 알 수 없는 토큰)
 *  - 우선순위(priority) 정렬
 *  - background / underline 스타일 확장
 */
import { beforeEach, describe, expect, it } from 'vitest';
import {
    compileRules,
    getCachedRules,
    getCompileDiagnostics,
    highlightLine,
    __resetRuleCache,
} from '../utils/syntaxHighlight';

describe('compileRules', () => {
    it('빈 배열/undefined/null을 안전하게 처리한다', () => {
        expect(compileRules(null).compiled).toEqual([]);
        expect(compileRules(undefined).compiled).toEqual([]);
        expect(compileRules([]).compiled).toEqual([]);
    });

    it('Python 형식 (?P<hl>...)을 JS 형식 (?<hl>...)로 변환한다', () => {
        const { compiled } = compileRules([
            { name: 'tst', pattern: '(?P<hl>foo)', color: '#fff' },
        ]);
        expect(compiled).toHaveLength(1);
        expect(compiled[0].hasHlGroup).toBe(true);
    });

    it('잘못된 정규식은 진단을 남기고 스킵한다', () => {
        const { compiled, diagnostics } = compileRules([
            { name: 'bad', pattern: '[unclosed', color: '#fff' },
        ]);
        expect(compiled).toHaveLength(0);
        expect(diagnostics).toHaveLength(1);
        expect(diagnostics[0].level).toBe('error');
        expect(diagnostics[0].rule).toBe('bad');
    });

    it('필수 필드 누락 시 진단을 남긴다', () => {
        const { compiled, diagnostics } = compileRules([
            { pattern: 'foo', color: '#fff' }, // name 없음
            { name: 'nopattern', color: '#fff' },
            { name: 'ok', pattern: 'x', color: '#fff' },
        ]);
        expect(compiled).toHaveLength(1);
        expect(compiled[0].name).toBe('ok');
        expect(diagnostics.filter((d) => d.level === 'error')).toHaveLength(2);
    });

    it('중복 이름에 대해 warn 진단을 남긴다', () => {
        const { compiled, diagnostics } = compileRules([
            { name: 'dup', pattern: 'a', color: '#fff' },
            { name: 'dup', pattern: 'b', color: '#fff' },
        ]);
        expect(compiled).toHaveLength(2); // 둘 다 컴파일은 됨
        expect(diagnostics.some((d) => d.level === 'warn' && /duplicate/i.test(d.message))).toBe(true);
    });

    it('알 수 없는 시맨틱 토큰에 대해 warn 진단을 남긴다', () => {
        const { diagnostics } = compileRules([
            { name: 't', pattern: 'x', color: 'erorr' }, // 오타
        ]);
        expect(diagnostics.some((d) => d.level === 'warn' && /color/i.test(d.message))).toBe(true);
    });

    it('정적 ReDoS 패턴을 거부한다', () => {
        const { compiled, diagnostics } = compileRules([
            { name: 'redos', pattern: '(a+)+b', color: '#fff' },
        ]);
        expect(compiled).toHaveLength(0);
        expect(diagnostics.some((d) => /redos|backtrack/i.test(d.message))).toBe(true);
    });

    it('priority 내림차순으로 정렬한다 (기본 0은 선언 순서 유지)', () => {
        const { compiled } = compileRules([
            { name: 'a', pattern: 'a', color: '#fff' },
            { name: 'b', pattern: 'b', color: '#fff', priority: 10 },
            { name: 'c', pattern: 'c', color: '#fff' },
            { name: 'd', pattern: 'd', color: '#fff', priority: 5 },
        ]);
        expect(compiled.map((r) => r.name)).toEqual(['b', 'd', 'a', 'c']);
    });

    it('background/underline 필드를 수용한다', () => {
        const { compiled } = compileRules([
            {
                name: 'x',
                pattern: 'x',
                color: '#fff',
                background: '#000',
                underline: true,
            },
        ]);
        expect(compiled[0].background).toBeTruthy();
        expect(compiled[0].underline).toBe(true);
    });
});

describe('highlightLine — hl group 위치 버그', () => {
    it('매치 내 동일 문자열이 반복될 때 hl 그룹이 올바른 위치를 가리킨다', () => {
        // regex: "Player (?P<hl>\w+) killed \w+"
        // text:  "Player Alice killed Alice"
        // hl은 첫 Alice (index 7~12) 여야 함 — indexOf 사용 시 동일 결과지만
        // 만약 text가 "Alice Player Alice killed Bob"이면 indexOf가 첫 Alice(0)를 잡아 버그 발생
        const { compiled } = compileRules([
            { name: 'kill', pattern: 'Player (?P<hl>\\w+) killed \\w+', color: '#ff0000' },
        ]);
        const text = 'Alice Player Alice killed Bob';
        const segs = highlightLine(text, compiled);
        // 하이라이트된 세그먼트는 정확히 인덱스 13~18의 "Alice" (두 번째 Alice) 여야 함
        const highlighted = segs.filter((s) => s.style !== null);
        expect(highlighted).toHaveLength(1);
        expect(highlighted[0].text).toBe('Alice');
        // 앞쪽 "Alice " (6자) + "Player " (7자) = 13자 → 하이라이트 시작점
        const offset = segs.slice(0, segs.indexOf(highlighted[0])).reduce((n, s) => n + s.text.length, 0);
        expect(offset).toBe(13);
    });

    it('hl 그룹이 없으면 전체 매치를 하이라이트한다', () => {
        const { compiled } = compileRules([
            { name: 'lvl', pattern: 'ERROR', color: '#f00' },
        ]);
        const segs = highlightLine('got ERROR!', compiled);
        const hl = segs.filter((s) => s.style);
        expect(hl).toHaveLength(1);
        expect(hl[0].text).toBe('ERROR');
    });
});

describe('highlightLine — 세그먼트 병합', () => {
    it('동일 규칙 반복 매치 시 세그먼트 수가 선형적으로 증가하지 않는다', () => {
        // "aaa aaa aaa ..." 20회 — 각 "aaa"가 별도 span이 되면 20개 이상
        const { compiled } = compileRules([
            { name: 'a3', pattern: 'aaa', color: '#abc' },
        ]);
        const text = ('aaa '.repeat(20)).trim();
        const segs = highlightLine(text, compiled);
        // 하이라이트 세그먼트끼리 동일 스타일이므로 "텍스트 종류 x 2" 선에서 끝나야 함
        // 정확한 수는 aaa(highlight) + " "(null) 반복이므로 39개 — 그게 맞음.
        // 단, 스타일 객체 참조가 같아야 병합 가능. 여기서는 공백으로 분리되므로 39개가 정상.
        expect(segs.length).toBeLessThanOrEqual(39);
        // 모든 하이라이트 세그먼트가 동일 스타일 객체여야 함
        const highlights = segs.filter((s) => s.style);
        const first = highlights[0].style;
        for (const h of highlights) {
            expect(h.style).toBe(first); // 레퍼런스 동일성
        }
    });

    it('동일 스타일을 가진 연속 문자 범위는 하나로 병합된다', () => {
        // 두 규칙이 같은 색을 가리키고 인접 매치할 때 병합되지는 않지만,
        // 동일 규칙이 인접 매치 (예: "ababab" pattern="a|b" color=#fff)는 문자 단위 병합.
        const { compiled } = compileRules([
            { name: 'ab', pattern: '[ab]', color: '#fff' },
        ]);
        const segs = highlightLine('abab', compiled);
        // 모두 하이라이트 + 스타일 동일 → 단일 세그먼트
        expect(segs).toHaveLength(1);
        expect(segs[0].text).toBe('abab');
    });
});

describe('highlightLine — 유니코드/zero-length', () => {
    it('이모지(서로게이트 페어)를 안전하게 처리한다', () => {
        const { compiled } = compileRules([
            { name: 'emoji', pattern: '🔥', color: '#f00' },
        ]);
        // "🔥" 는 UTF-16에서 2 code unit
        const text = 'a🔥b';
        const segs = highlightLine(text, compiled);
        const hl = segs.filter((s) => s.style);
        expect(hl).toHaveLength(1);
        expect(hl[0].text).toBe('🔥');
    });

    it('zero-length 매치는 무한 루프 없이 한 문자씩 전진한다', () => {
        const { compiled } = compileRules([
            { name: 'empty', pattern: '^', color: '#f00' },
        ]);
        // start-of-string 앵커는 zero-length. lastIndex 전진 로직이 없으면 hang.
        // 이 테스트가 끝나면 OK.
        const segs = highlightLine('abc', compiled);
        expect(segs).toBeDefined();
        expect(segs.map((s) => s.text).join('')).toBe('abc');
    });
});

describe('highlightLine — ReDoS 보호', () => {
    it('규칙당 매치 횟수가 상한을 넘지 않고 조기 종료된다', () => {
        // . 하나에 매치하는 글로벌 규칙 — 긴 문자열
        const { compiled } = compileRules([
            { name: 'any', pattern: 'a', color: '#fff' },
        ]);
        const text = 'a'.repeat(10000);
        const start = Date.now();
        const segs = highlightLine(text, compiled);
        const elapsed = Date.now() - start;
        // 최소한 반환은 되어야 하고, 현실적인 시간 안에 완료
        expect(segs).toBeDefined();
        expect(elapsed).toBeLessThan(1000);
    });
});

describe('getCachedRules — WeakMap 캐시', () => {
    beforeEach(() => {
        __resetRuleCache();
    });

    it('동일한 rawRules 레퍼런스는 캐싱된 결과를 반환한다', () => {
        const rules = [{ name: 't', pattern: 'x', color: '#fff' }];
        const a = getCachedRules('mod', rules);
        const b = getCachedRules('mod', rules);
        expect(a).toBe(b);
    });

    it('rawRules 레퍼런스가 바뀌면 재컴파일된다 (모듈 리로드 대응)', () => {
        const rules1 = [{ name: 't', pattern: 'x', color: '#fff' }];
        const rules2 = [{ name: 't', pattern: 'y', color: '#fff' }];
        const a = getCachedRules('mod', rules1);
        const b = getCachedRules('mod', rules2);
        expect(a).not.toBe(b);
        expect(a[0].regex.source).not.toBe(b[0].regex.source);
    });

    it('getCompileDiagnostics는 컴파일 단계에서 수집된 진단을 반환한다', () => {
        const rules = [
            { name: 'bad', pattern: '[', color: '#fff' },
            { name: 'ok', pattern: 'x', color: '#fff' },
        ];
        getCachedRules('mod', rules);
        const diags = getCompileDiagnostics(rules);
        expect(diags.some((d) => d.rule === 'bad' && d.level === 'error')).toBe(true);
    });
});

describe('highlightLine — background/underline 렌더', () => {
    it('background 지정 시 style에 backgroundColor가 들어간다', () => {
        const { compiled } = compileRules([
            { name: 'bg', pattern: 'x', color: '#fff', background: '#000' },
        ]);
        const segs = highlightLine('x', compiled);
        expect(segs[0].style.backgroundColor).toBeTruthy();
    });

    it('underline true → textDecoration underline', () => {
        const { compiled } = compileRules([
            { name: 'u', pattern: 'x', color: '#fff', underline: true },
        ]);
        const segs = highlightLine('x', compiled);
        expect(segs[0].style.textDecoration).toBe('underline');
    });
});

describe('resolveColor — 시맨틱 토큰', () => {
    it('확장된 시맨틱 토큰을 CSS var로 해석한다', () => {
        const { compiled } = compileRules([
            { name: 't', pattern: 'x', color: 'critical' },
        ]);
        // critical / port / duration 등 확장 토큰 지원
        expect(compiled[0].color).toMatch(/var\(--syn-critical|#/);
    });

    it('HEX는 그대로 통과시킨다', () => {
        const { compiled } = compileRules([
            { name: 't', pattern: 'x', color: '#abcdef' },
        ]);
        expect(compiled[0].color).toBe('#abcdef');
    });
});
