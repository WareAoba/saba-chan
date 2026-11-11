import { type RefObject, useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { usePageFindStore } from '../../stores/usePageFindStore';
import { type FindMatch, findMatches, foldOf, isFoldClosed, keyMatches, positions } from '../../utils/pageFind';
import { Icon } from '../Icon';
import './PageFind.css';

/*
 * 서버 상세 설정·사바쨩·디스코드 탭의 Ctrl+F 찾기 (사용자 11-10 시안 승인).
 * 오른쪽 위 작은 찾기 창에 쓰면 지금 탭의 칸 이름·입력된 값·원문 키에서 찾아 그 자리에서 칠한다.
 * 화면은 다시 그리지 않는다: 글자는 CSS Custom Highlight API로, 입력 칸 값은 위에 얹은 얇은 층으로 칠한다.
 */

const HL = 'saba-find';
const HL_CUR = 'saba-find-current';

/** CSS Custom Highlight API (없는 웹뷰는 글자도 얹은 층으로 칠한다) */
// biome-ignore lint/suspicious/noExplicitAny: lib.dom에 아직 없는 웹뷰가 있어 있는지만 본다
const highlightApi = (): { registry: any; Highlight: any } | null => {
    // biome-ignore lint/suspicious/noExplicitAny: 위와 같다
    const w = globalThis as any;
    return w.CSS?.highlights && typeof w.Highlight === 'function'
        ? { registry: w.CSS.highlights, Highlight: w.Highlight }
        : null;
};

const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || '');

/** 원문 키 꼬리표: 찾는 글자가 키와 맞고 이름에는 없을 때만 그 줄 이름 옆에 뜬다 */
export function FindKeyTag({ findKey, label }: { findKey?: string; label?: string }) {
    const show = usePageFindStore(
        (s) => s.open && keyMatches(s.query, findKey) && positions(label || '', s.query).length === 0,
    );
    return show ? (
        <code className="find-keytag" data-find="key">
            {findKey}
        </code>
    ) : null;
}

interface Rect {
    top: number;
    left: number;
    width: number;
    height: number;
    cur: boolean;
}

/** 입력 칸 값 안의 글자 자리를 화면 좌표로 (한 줄 입력만, 여러 줄은 칸 전체) */
function inputRect(m: Extract<FindMatch, { kind: 'input' }>, measure: HTMLSpanElement): Omit<Rect, 'cur'> | null {
    const el = m.el;
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) return null;
    if (el instanceof HTMLTextAreaElement) return { top: r.top, left: r.left, width: r.width, height: r.height };
    const cs = getComputedStyle(el);
    measure.style.font = cs.font;
    measure.style.letterSpacing = cs.letterSpacing;
    const widthOf = (s: string) => {
        measure.textContent = s;
        return measure.getBoundingClientRect().width;
    };
    const padL = parseFloat(cs.paddingLeft) + parseFloat(cs.borderLeftWidth);
    const padR = parseFloat(cs.paddingRight) + parseFloat(cs.borderRightWidth);
    const x0 = r.left + padL;
    const x1 = r.right - padR;
    const left = x0 + widthOf(el.value.slice(0, m.start)) - el.scrollLeft;
    const right = left + widthOf(el.value.slice(m.start, m.end));
    const l = Math.max(left, x0);
    const w = Math.min(right, x1) - l;
    if (w <= 0) return null;
    const padT = parseFloat(cs.paddingTop) + parseFloat(cs.borderTopWidth);
    const padB = parseFloat(cs.paddingBottom) + parseFloat(cs.borderBottomWidth);
    return { top: r.top + padT, left: l, width: w, height: r.height - padT - padB };
}

const rangeOf = (m: Extract<FindMatch, { kind: 'text' }>) => {
    const range = document.createRange();
    range.setStart(m.node, m.start);
    range.setEnd(m.node, m.end);
    return range;
};

/** 접힌 묶음을 펴는 단추 (`aria-controls`로 묶음을 가리킨다) */
const foldToggle = (root: Element, fold: HTMLElement) =>
    Array.from(root.querySelectorAll<HTMLElement>('[aria-controls]')).find(
        (b) => b.getAttribute('aria-controls') === fold.id,
    );

interface PageFindProps {
    /** 찾을 곳 (탭 본문) */
    rootRef: RefObject<HTMLElement | null>;
    /** 이 탭에서 찾기를 쓰는가 (설정·사바쨩·디스코드) */
    enabled: boolean;
    /** 탭·서버가 바뀌면 다시 찾는다 */
    pageKey: string;
    /** 찾기 창을 붙일 곳 (탭 줄). 없으면 화면 오른쪽 위 */
    anchorRef?: RefObject<HTMLElement | null>;
    /** 창(대화상자) 안의 찾기: 떠 있는 동안 뒤 화면의 찾기는 Ctrl+F를 받지 않고 칠하지 않는다 */
    modal?: boolean;
}

export function PageFind({ rootRef, enabled, pageKey, anchorRef, modal = false }: PageFindProps) {
    const { t } = useTranslation('gui');
    const storeOpen = usePageFindStore((s) => s.open);
    const modalActive = usePageFindStore((s) => s.modal);
    const setModal = usePageFindStore((s) => s.setModal);
    // 창 안의 찾기가 떠 있으면 뒤 화면의 찾기는 쉰다 (같은 칠하기 이름을 함께 쓰므로)
    const live = enabled && (modal || !modalActive);
    const open = storeOpen && live;
    const query = usePageFindStore((s) => s.query);
    const setOpen = usePageFindStore((s) => s.setOpen);
    const setQuery = usePageFindStore((s) => s.setQuery);
    const inputRef = useRef<HTMLInputElement>(null);
    const measureRef = useRef<HTMLSpanElement>(null);
    const matchesRef = useRef<FindMatch[]>([]);
    const curRef = useRef(0);
    const [cur, setCur] = useState(0);
    const [total, setTotal] = useState(0);
    const [rects, setRects] = useState<Rect[]>([]);
    const [pos, setPos] = useState<{ top: number; right: number } | null>(null);
    const [shake, setShake] = useState(false);
    const frame = useRef(0);

    /** 칠하기: 글자는 하이라이트 API, 입력 칸(과 API가 없을 때의 글자)은 얹은 층 */
    const paint = useCallback(() => {
        const api = highlightApi();
        const matches = matchesRef.current;
        const c = curRef.current;
        const visible = (m: FindMatch) => !isFoldClosed(foldOf(m));
        if (api) {
            const all = new api.Highlight();
            const now = new api.Highlight();
            matches.forEach((m, i) => {
                if (m.kind === 'text' && visible(m)) (i === c ? now : all).add(rangeOf(m));
            });
            api.registry.set(HL, all);
            api.registry.set(HL_CUR, now);
        }
        const out: Rect[] = [];
        const measure = measureRef.current;
        matches.forEach((m, i) => {
            if (!visible(m)) return;
            if (m.kind === 'input') {
                const r = measure && inputRect(m, measure);
                if (r) out.push({ ...r, cur: i === c });
            } else if (!api) {
                for (const r of Array.from(rangeOf(m).getClientRects?.() ?? [])) {
                    out.push({ top: r.top, left: r.left, width: r.width, height: r.height, cur: i === c });
                }
            }
        });
        // 앱 위 막대(--appbar-h) 밑으로 스크롤되어 들어간 칸은 막대 위에 칠하지 않는다
        const bar = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--appbar-h')) || 0;
        setRects(out.filter((r) => r.top + r.height > bar && r.top < window.innerHeight));
        // 찾기 창 자리: 탭 줄 오른쪽 끝 아래, 탭 줄이 위로 밀려 올라가면 앱 위 막대 밑에 붙는다
        const a = anchorRef?.current?.getBoundingClientRect();
        setPos(a ? { top: Math.max(a.bottom + 8, bar + 8), right: Math.max(window.innerWidth - a.right, 12) } : null);
    }, [anchorRef]);

    const schedulePaint = useCallback(() => {
        cancelAnimationFrame(frame.current);
        frame.current = requestAnimationFrame(paint);
    }, [paint]);

    /** 접힌 묶음마다 안에 든 결과 수를 머리에 띄운다 */
    const badges = useCallback(() => {
        const root = rootRef.current;
        if (!root) return;
        for (const b of Array.from(root.querySelectorAll('[data-find-badge]'))) b.removeAttribute('data-find-badge');
        const counts = new Map<HTMLElement, number>();
        for (const m of matchesRef.current) {
            const fold = foldOf(m);
            if (isFoldClosed(fold)) counts.set(fold as HTMLElement, (counts.get(fold as HTMLElement) || 0) + 1);
        }
        for (const [fold, n] of counts)
            foldToggle(root, fold)?.setAttribute('data-find-badge', t('find.fold_hits', { count: n }));
    }, [rootRef, t]);

    const scrollToCur = useCallback(() => {
        const m = matchesRef.current[curRef.current];
        if (!m) return;
        const el = m.el as HTMLElement;
        const r = el.getBoundingClientRect();
        if (r.top < 80 || r.bottom > window.innerHeight - 40) {
            const reduce = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
            el.scrollIntoView?.({ block: 'center', behavior: reduce ? 'auto' : 'smooth' });
        }
    }, []);

    /** 결과로 간다: 접힌 묶음 안이면 펴고, 보이게 스크롤한다 */
    const goTo = useCallback(
        (i: number) => {
            curRef.current = i;
            setCur(i);
            const m = matchesRef.current[i];
            const root = rootRef.current;
            const fold = m && foldOf(m);
            if (root && fold && isFoldClosed(fold)) {
                foldToggle(root, fold)?.click();
                // 펼쳐진 뒤(다음 그리기) 칠하고 스크롤한다
                setTimeout(() => {
                    paint();
                    scrollToCur();
                }, 0);
                return;
            }
            paint();
            scrollToCur();
        },
        [paint, rootRef, scrollToCur],
    );

    /** 다시 찾기. reset이면 화면에 보이는 첫 결과부터 */
    const research = useCallback(
        (reset: boolean) => {
            const root = rootRef.current;
            const q = usePageFindStore.getState().query;
            matchesRef.current = root && usePageFindStore.getState().open ? findMatches(root, q) : [];
            const n = matchesRef.current.length;
            setTotal(n);
            badges();
            if (reset) {
                const first = matchesRef.current.findIndex(
                    (m) => !isFoldClosed(foldOf(m)) && m.el.getBoundingClientRect().top >= 80,
                );
                curRef.current = Math.max(first, 0);
                setCur(curRef.current);
                if (n) goTo(curRef.current);
                else paint();
                return;
            }
            curRef.current = Math.min(curRef.current, Math.max(0, n - 1));
            setCur(curRef.current);
            paint();
        },
        [badges, goTo, paint, rootRef],
    );

    // Ctrl+F (macOS ⌘F): 이 탭이면 웹뷰 찾기 대신 연다
    useEffect(() => {
        const onKey = (e: KeyboardEvent) => {
            if (!live || !(e.ctrlKey || e.metaKey) || e.altKey || e.shiftKey || e.key.toLowerCase() !== 'f') return;
            e.preventDefault();
            setOpen(true);
            requestAnimationFrame(() => {
                inputRef.current?.focus();
                inputRef.current?.select();
            });
        };
        window.addEventListener('keydown', onKey);
        return () => window.removeEventListener('keydown', onKey);
    }, [live, setOpen]);

    // 찾기를 쓰지 않는 탭으로 가면 닫는다 (창 안의 찾기가 떠 있으면 그 찾기의 것이라 두다)
    useEffect(() => {
        if (!enabled && storeOpen && !modalActive) setOpen(false);
    }, [enabled, storeOpen, modalActive, setOpen]);

    // 창 안의 찾기: 떠 있는 동안 뒤 화면의 찾기를 쉬게 한다
    useEffect(() => {
        if (!modal) return;
        setModal(true);
        return () => setModal(false);
    }, [modal, setModal]);

    // 글자·탭이 바뀌면 다시 찾는다 (꼬리표가 그려진 뒤)
    // biome-ignore lint/correctness/useExhaustiveDependencies: pageKey가 바뀌면 새 탭 본문에서 찾는다
    useLayoutEffect(() => {
        if (!open) return;
        const id = requestAnimationFrame(() => research(true));
        return () => cancelAnimationFrame(id);
    }, [open, query, pageKey, research]);

    // 탭 안이 바뀌면(불러오기 끝, 값 고침, 묶음 접기) 자리를 지키며 다시 찾는다
    // biome-ignore lint/correctness/useExhaustiveDependencies: pageKey가 바뀌면 새 탭 본문을 지켜본다
    useEffect(() => {
        const root = rootRef.current;
        if (!open || !root) return;
        let timer = 0;
        const again = () => {
            clearTimeout(timer);
            timer = window.setTimeout(() => research(false), 60);
        };
        const mo = new MutationObserver(again);
        mo.observe(root, {
            subtree: true,
            childList: true,
            characterData: true,
            attributes: true,
            attributeFilter: ['hidden'],
        });
        root.addEventListener('input', again);
        window.addEventListener('scroll', schedulePaint, true);
        window.addEventListener('resize', schedulePaint);
        return () => {
            clearTimeout(timer);
            mo.disconnect();
            root.removeEventListener('input', again);
            window.removeEventListener('scroll', schedulePaint, true);
            window.removeEventListener('resize', schedulePaint);
        };
    }, [open, pageKey, research, schedulePaint, rootRef]);

    // 닫으면 칠한 것을 지운다 (쉬는 찾기는 칠한 게 있을 때만: 창 안 찾기가 칠한 것을 지우지 않게)
    useEffect(() => {
        if (open) return;
        if (!live && !matchesRef.current.length) return;
        const api = highlightApi();
        api?.registry.delete(HL);
        api?.registry.delete(HL_CUR);
        for (const b of Array.from(rootRef.current?.querySelectorAll('[data-find-badge]') ?? []))
            b.removeAttribute('data-find-badge');
        matchesRef.current = [];
        setRects([]);
        setTotal(0);
    }, [open, live, rootRef]);
    useEffect(
        () => () => {
            const api = highlightApi();
            api?.registry.delete(HL);
            api?.registry.delete(HL_CUR);
            usePageFindStore.getState().setOpen(false);
        },
        [],
    );

    const step = (d: number) => {
        const n = matchesRef.current.length;
        if (!n) {
            setShake(true);
            return;
        }
        goTo((curRef.current + d + n) % n);
    };

    /** 닫고, 마지막 결과가 있던 줄의 입력에 커서를 둔다 */
    const close = (focusField: boolean) => {
        const m = matchesRef.current[curRef.current];
        setOpen(false);
        if (!focusField || !m) return;
        const target =
            m.kind === 'input'
                ? m.el
                : m.el
                      .closest('.ss-row, [data-find-row]')
                      ?.querySelector<HTMLElement>(
                          'input, textarea, select, .custom-dropdown-trigger, [role="switch"]',
                      );
        target?.focus({ preventScroll: true });
    };

    if (!open) return <span ref={measureRef} className="page-find-measure" aria-hidden="true" />;
    const q = query.trim();
    return (
        <>
            <span ref={measureRef} className="page-find-measure" aria-hidden="true" />
            {createPortal(
                <div className={`page-find-layer${modal ? ' page-find-layer--modal' : ''}`} aria-hidden="true">
                    {rects.map((r, i) => (
                        <i
                            // biome-ignore lint/suspicious/noArrayIndexKey: 자리만 그리는 칸이라 순서가 곧 정체다
                            key={i}
                            className={r.cur ? 'cur' : undefined}
                            style={{ top: r.top, left: r.left, width: r.width, height: r.height }}
                        />
                    ))}
                </div>,
                document.body,
            )}
            <div
                className={`page-find ${q && !total ? 'none' : ''} ${shake ? 'shake' : ''}`}
                onAnimationEnd={() => setShake(false)}
                role="search"
                aria-label={t('find.label')}
                style={pos ? { top: pos.top, right: pos.right } : undefined}
            >
                <Icon name="search" size="sm" />
                <input
                    ref={inputRef}
                    type="search"
                    value={query}
                    placeholder={t('find.placeholder')}
                    aria-label={t('find.label')}
                    spellCheck={false}
                    autoComplete="off"
                    // biome-ignore lint/a11y/noAutofocus: Ctrl+F로 연 찾기 창은 바로 글자를 받는다
                    autoFocus
                    onChange={(e) => setQuery(e.target.value)}
                    onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                            e.preventDefault();
                            step(e.shiftKey ? -1 : 1);
                        } else if (e.key === 'Escape') {
                            e.preventDefault();
                            close(true);
                        }
                    }}
                />
                <span className="page-find-count" aria-live="polite">
                    {!q ? '' : total ? t('find.count', { cur: cur + 1, total }) : t('find.none')}
                </span>
                <span className="page-find-sep" />
                <button
                    type="button"
                    onClick={() => step(-1)}
                    disabled={total < 2}
                    aria-label={t('find.prev')}
                    data-tooltip={t('find.prev')}
                >
                    <Icon name="chevronUp" size="sm" />
                </button>
                <button
                    type="button"
                    onClick={() => step(1)}
                    disabled={total < 2}
                    aria-label={t('find.next')}
                    data-tooltip={t('find.next')}
                >
                    <Icon name="chevronDown" size="sm" />
                </button>
                <button
                    type="button"
                    onClick={() => close(false)}
                    aria-label={t('find.close')}
                    data-tooltip={t('find.close')}
                >
                    <Icon name="close" size="sm" />
                </button>
            </div>
        </>
    );
}

/** 탭 줄의 "찾기" 단추 (마우스·터치로 여는 길) */
export function PageFindButton() {
    const { t } = useTranslation('gui');
    const setOpen = usePageFindStore((s) => s.setOpen);
    return (
        <button type="button" className="page-find-btn" onClick={() => setOpen(true)} aria-label={t('find.label')}>
            <Icon name="search" size="sm" />
            <span>{t('find.button')}</span>
            <kbd>{isMac ? '⌘F' : 'Ctrl F'}</kbd>
        </button>
    );
}
