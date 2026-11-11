/**
 * 페이지 안 찾기 (서버 상세 설정·사바쨩·디스코드 탭의 Ctrl+F).
 * 화면을 다시 그리지 않고 DOM에서 찾는다. 찾는 곳은 세 가지다:
 * - `data-find` 붙은 요소의 글자 (칸 이름, 고른 값, 부르는 말 칩, 원문 키 꼬리표)
 * - 입력 칸(input·textarea)의 값 (비밀번호 칸은 빼고)
 * 접힌 묶음(`data-find-fold` + hidden) 안의 것도 세고, 그 결과로 가면 펼친다.
 */

export type FindMatch =
    | { kind: 'text'; node: Text; start: number; end: number; el: Element }
    | { kind: 'input'; el: HTMLInputElement | HTMLTextAreaElement; start: number; end: number };

/** 원문 키 비교: 대소문자, `-` `_` 공백을 같게 본다 */
export const normKey = (s: string) => s.toLowerCase().replace(/[-\s]/g, '_');

/** 원문 키가 찾는 글자와 맞는가 (두 글자 이상부터) */
export function keyMatches(query: string, key: string | undefined): boolean {
    const q = normKey(query.trim());
    return !!key && q.length >= 2 && normKey(key).includes(q);
}

/** 글자 안에서 찾는 글자가 나오는 자리들 (대소문자 무시) */
export function positions(text: string, query: string): number[] {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    const lo = text.toLowerCase();
    const out: number[] = [];
    for (let i = lo.indexOf(q); i >= 0; i = lo.indexOf(q, i + q.length)) out.push(i);
    return out;
}

const SEARCHABLE_INPUT = (el: Element): el is HTMLInputElement | HTMLTextAreaElement =>
    el instanceof HTMLTextAreaElement ||
    (el instanceof HTMLInputElement && ['text', 'number', 'search', 'url', ''].includes(el.type));

/** 키 꼬리표의 글자는 `-` `_` 공백을 같게 보고 찾는다 */
function textPositions(el: Element, text: string, query: string): { start: number; len: number }[] {
    const q = query.trim();
    if (el.getAttribute('data-find') === 'key') {
        const nq = normKey(q);
        const nt = normKey(text);
        const out = [];
        for (let i = nt.indexOf(nq); nq && i >= 0; i = nt.indexOf(nq, i + nq.length))
            out.push({ start: i, len: nq.length });
        return out;
    }
    return positions(text, q).map((start) => ({ start, len: q.length }));
}

/** root 안에서 문서 순서대로 결과를 모은다 */
export function findMatches(root: Element, query: string): FindMatch[] {
    if (!query.trim()) return [];
    const out: FindMatch[] = [];
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
        if (n.nodeType === Node.TEXT_NODE) {
            const el = n.parentElement?.closest('[data-find]');
            if (!el || !root.contains(el)) continue;
            const text = n.nodeValue || '';
            for (const p of textPositions(el, text, query)) {
                out.push({ kind: 'text', node: n as Text, start: p.start, end: p.start + p.len, el });
            }
        } else if (SEARCHABLE_INPUT(n as Element)) {
            const input = n as HTMLInputElement | HTMLTextAreaElement;
            const q = query.trim();
            for (const start of positions(input.value, q))
                out.push({ kind: 'input', el: input, start, end: start + q.length });
        }
    }
    return out;
}

/** 결과가 있는 요소 (스크롤·포커스 기준) */
export const matchElement = (m: FindMatch): Element => m.el;

/** 결과가 접힌 묶음 안에 있으면 그 묶음 */
export const foldOf = (m: FindMatch): HTMLElement | null =>
    (matchElement(m).closest('[data-find-fold]') as HTMLElement | null) ?? null;

/** 접힌 묶음이 지금 숨어 있는가 */
export const isFoldClosed = (fold: HTMLElement | null) => !!fold && fold.hidden;
