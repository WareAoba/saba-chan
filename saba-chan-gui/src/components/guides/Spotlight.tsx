import { type CSSProperties, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { advanceGuide, closeSpotlight, goToStep, guideAction, useGuideStore } from './guides';

/** 짚을 자리가 화면에 나타나길 기다리는 시간 (화면을 연 직후 그려지는 동안) */
const WAIT_MS = 2000;
const PAD = 6;
const BUBBLE_W = 340;
const GAP = 14;

interface Box {
    left: number;
    top: number;
    width: number;
    height: number;
}

const same = (a: Box | null, b: Box) =>
    !!a && a.left === b.left && a.top === b.top && a.width === b.width && a.height === b.height;

/** `[data-spot]` 요소를 찾아 그 자리를 따라간다. 없으면 null (기다리는 동안은 'wait') */
function useSpotBox(spot: string | undefined, key: string): Box | null | 'wait' {
    const [box, setBox] = useState<Box | null | 'wait'>(spot ? 'wait' : null);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 단계(key)가 바뀌면 다시 찾는다
    useEffect(() => {
        if (!spot) {
            setBox(null);
            return;
        }
        setBox('wait');
        const started = performance.now();
        let el: Element | null = null;
        let scrolled = false;
        let frame = 0;
        const tick = () => {
            // 숨겨 둔 탭(App KeptPage `hidden`) 안의 같은 자리는 건너뛴다
            if (!el?.isConnected || el.closest('[hidden]')) {
                el =
                    Array.from(document.querySelectorAll(`[data-spot="${CSS.escape(spot)}"]`)).find(
                        (e) => !e.closest('[hidden]'),
                    ) ?? null;
            }
            if (el) {
                if (!scrolled) {
                    el.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
                    scrolled = true;
                }
                const r = el.getBoundingClientRect();
                const next = { left: r.left - PAD, top: r.top - PAD, width: r.width + PAD * 2, height: r.height + PAD * 2 };
                setBox((prev) => (prev !== 'wait' && same(prev, next) ? prev : next));
            } else if (performance.now() - started > WAIT_MS) {
                setBox((prev) => (prev === null ? prev : null));
            }
            frame = requestAnimationFrame(tick);
        };
        tick();
        return () => cancelAnimationFrame(frame);
    }, [spot, key]);
    return box;
}

/**
 * 화면 강조 (사용자 11-11): 가이드 단계의 `spot` 자리만 밝게 두고 나머지를 어둡게 덮어 말풍선으로 설명한다.
 * 자리가 없는 단계는 가운데 말풍선. 말풍선 밖(어두운 곳)을 누르거나 Esc·✕면 바로 닫힌다. 밝은 자리는 그대로 누를 수 있다.
 */
export function Spotlight() {
    const { t } = useTranslation('gui');
    const active = useGuideStore((s) => s.active);
    const guide = useGuideStore((s) => (s.active ? s.guides?.find((g) => g.id === s.active?.id) : undefined));
    const step = active && guide ? guide.steps[active.step] : undefined;
    const box = useSpotBox(step?.spot, active ? `${active.id}:${active.step}` : '');
    const primary = useRef<HTMLButtonElement>(null);
    const shown = !!active && !!step && box !== 'wait';

    // 말풍선이 뜨거나 단계가 바뀌면 주 버튼으로 포커스 (가려진 화면을 Tab으로 지나 누르지 않게)
    // biome-ignore lint/correctness/useExhaustiveDependencies: 단계가 바뀔 때마다
    useEffect(() => {
        if (shown) primary.current?.focus();
    }, [shown, active?.id, active?.step]);

    useEffect(() => {
        if (!active) return;
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') {
                e.stopPropagation();
                closeSpotlight();
            }
        };
        window.addEventListener('keydown', onKey, true);
        return () => window.removeEventListener('keydown', onKey, true);
    }, [active]);

    if (!active || !guide || !step) return null;
    const k = active.step + 1;
    // 화면을 넘겨받는 단계: 버튼이 그 화면을 연다
    const handoff = !!guideAction(step.action)?.handoff;
    const n = guide.steps.length;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const hole = box && box !== 'wait' ? box : null;

    // 말풍선 자리: 자리 아래(모자라면 위), 없으면 가운데
    const width = Math.min(BUBBLE_W, vw - 24);
    let bubbleStyle: CSSProperties;
    if (hole) {
        const left = Math.min(Math.max(12, hole.left + hole.width / 2 - width / 2), vw - width - 12);
        const below = hole.top + hole.height + GAP + 180 < vh;
        bubbleStyle = below
            ? { left, top: hole.top + hole.height + GAP, width }
            : { left, bottom: vh - hole.top + GAP, width };
    } else {
        bubbleStyle = { left: '50%', top: '50%', width, transform: 'translate(-50%, -50%)' };
    }

    // 구멍 둘레 네 조각이 바깥 누름을 받는다 (구멍 안은 아래 화면으로 지나간다)
    const catchers: CSSProperties[] = hole
        ? [
              { left: 0, top: 0, width: vw, height: Math.max(0, hole.top) },
              { left: 0, top: hole.top + hole.height, width: vw, bottom: 0 },
              { left: 0, top: hole.top, width: Math.max(0, hole.left), height: hole.height },
              { left: hole.left + hole.width, top: hole.top, right: 0, height: hole.height },
          ]
        : [{ inset: 0 }];

    return createPortal(
        <div className="spotlight" data-testid="spotlight">
            {catchers.map((style, i) => (
                // biome-ignore lint/a11y/noStaticElementInteractions: 바깥 누름으로 닫기 (키보드는 Esc)
                // biome-ignore lint/a11y/useKeyWithClickEvents: Esc가 같은 일을 한다
                <div
                    // biome-ignore lint/suspicious/noArrayIndexKey: 늘 같은 네 조각
                    key={i}
                    className="spotlight-catch"
                    data-testid="spotlight-outside"
                    style={style}
                    onClick={closeSpotlight}
                />
            ))}
            {hole ? (
                <div className="spotlight-hole" style={hole} />
            ) : (
                box === null && <div className="spotlight-dim" />
            )}
            {box !== 'wait' && (
                <div className="spotlight-bubble" role="dialog" aria-label={guide.title} style={bubbleStyle}>
                    <div className="spotlight-head">
                        <Icon name={guide.icon} size="xs" />
                        <span className="spotlight-title">{guide.title}</span>
                        <span className="spotlight-count">{`${k} / ${n}`}</span>
                        <button type="button" className="icon-btn" onClick={closeSpotlight} aria-label={t('guides.close')}>
                            <Icon name="close" size="xs" />
                        </button>
                    </div>
                    <p className="spotlight-text">{step.text}</p>
                    <div className="spotlight-foot">
                        <span className="spotlight-dots" aria-hidden="true">
                            {guide.steps.map((s, i) => (
                                <span key={s.id ?? i} className={i < k ? 'on' : undefined} />
                            ))}
                        </span>
                        {k > 1 && (
                            <button type="button" className="btn btn-sm" onClick={() => goToStep(guide.id, active.step - 1)}>
                                {t('guides.prev')}
                            </button>
                        )}
                        <button
                            ref={primary}
                            type="button"
                            className="btn btn-primary btn-sm"
                            onClick={() => advanceGuide(guide.id, active.step)}
                        >
                            {k < n && !handoff ? t('guides.next') : handoff ? t('guides.go') : t('guides.finish')}
                        </button>
                    </div>
                </div>
            )}
        </div>,
        document.body,
    );
}
