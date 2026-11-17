import { type CSSProperties, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useTranslation } from 'react-i18next';
import { useUIStore } from '../../stores/useUIStore';
import { Icon } from '../Icon';
import { advanceGuide, closeSpotlight, goToStep, guideAction, PICK_SPOTS, useGuideStore } from './guides';

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

/** 보이는 `[data-spot]` 요소. 숨겨 둔 탭(App KeptPage `hidden`) 안의 같은 자리는 건너뛴다 */
function findSpot(spot: string): Element | null {
    return (
        Array.from(document.querySelectorAll(`[data-spot="${CSS.escape(spot)}"]`)).find((e) => !e.closest('[hidden]')) ??
        null
    );
}

/**
 * `[data-spot]` 요소를 찾아 그 자리를 따라간다. 없으면 null (기다리는 동안은 'wait').
 * `a|b`면 앞의 것을 먼저 찾고, 뒤의 것을 짚는 동안(`alt`)에도 앞의 것이 생기면 그리로 옮긴다
 */
function useSpotBox(spot: string | undefined, key: string): { box: Box | null | 'wait'; alt: boolean } {
    const [box, setBox] = useState<Box | null | 'wait'>(spot ? 'wait' : null);
    const [alt, setAlt] = useState(false);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 단계(key)가 바뀌면 다시 찾는다
    useEffect(() => {
        if (!spot) {
            setBox(null);
            setAlt(false);
            return;
        }
        setBox('wait');
        const choices = spot.split('|');
        const started = performance.now();
        let el: Element | null = null;
        let at = -1;
        let frame = 0;
        const tick = () => {
            if (!el?.isConnected || el.closest('[hidden]') || at > 0) {
                const found = choices.map(findSpot);
                const i = found.findIndex(Boolean);
                if (found[i] !== el) {
                    el = found[i] ?? null;
                    at = i;
                    el?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
                    setAlt(i > 0);
                }
            }
            if (el) {
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
    return { box, alt };
}

/** 자리가 화면에 있는가 (없으면 false, spot이 없으면 undefined) */
function useSpotSeen(spot: string | undefined, key: string): boolean | undefined {
    const [seen, setSeen] = useState<boolean | undefined>(undefined);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 단계(key)가 바뀌면 다시 본다
    useEffect(() => {
        setSeen(undefined);
        if (!spot) return;
        let frame = 0;
        const tick = () => {
            setSeen(!!findSpot(spot));
            frame = requestAnimationFrame(tick);
        };
        tick();
        return () => cancelAnimationFrame(frame);
    }, [spot, key]);
    return seen;
}

/**
 * 화면 강조: 가이드 단계의 `spot` 자리만 밝게 두고 나머지를 어둡게 덮어 말풍선으로 설명한다.
 * 자리가 없는 단계는 가운데 말풍선. 말풍선 밖(어두운 곳)을 누르거나 Esc·✕면 바로 닫힌다. 밝은 자리는 그대로 누를 수 있다.
 * 사용자가 해야 넘어가는 단계는 "다음" 대신 안내를 보이고 기다린다:
 *   고르기(active.pick)         디스코드 서버·게임 서버 목록을 짚고, 고르면 그 단계를 다시 연다
 *   `wait` 단계                 다음 단계(같은 화면) 자리가 아직 없으면, 생기는 대로 넘어간다
 *   `a|b`의 b를 짚는 중         a가 생기면 그리로 옮긴다
 */
export function Spotlight() {
    const { t } = useTranslation('gui');
    const active = useGuideStore((s) => s.active);
    const guide = useGuideStore((s) => (s.active ? s.guides?.find((g) => g.id === s.active?.id) : undefined));
    const step = active && guide ? guide.steps[active.step] : undefined;
    const pick = active?.pick;
    const key = active ? `${active.id}:${active.step}:${pick ?? ''}` : '';
    const { box, alt } = useSpotBox(pick ? PICK_SPOTS[pick] : step?.spot, key);
    const after = active && !pick ? guide?.steps[active.step + 1] : undefined;
    const nextSeen = useSpotSeen(step?.wait && after && !after.go && !after.action ? after.spot : undefined, key);
    const hasHole = !!box && box !== 'wait';
    const waitNext = hasHole && nextSeen === false;
    const waiting = !!pick || waitNext || (hasHole && alt);
    const openNode = useUIStore((s) => s.openDiscordNodeId);
    const openServer = useUIStore((s) => s.activeServerId);
    const primary = useRef<HTMLButtonElement>(null);
    const shown = !!active && !!step && box !== 'wait';

    // 말풍선이 뜨거나 단계가 바뀌면 주 버튼으로 포커스 (가려진 화면을 Tab으로 지나 누르지 않게)
    // biome-ignore lint/correctness/useExhaustiveDependencies: 단계가 바뀔 때마다
    useEffect(() => {
        if (shown) primary.current?.focus();
    }, [shown, active?.id, active?.step]);

    useEffect(() => {
        if (active?.pick === 'guild' && openNode) goToStep(active.id, active.step);
        if (active?.pick === 'server' && openServer) {
            useGuideStore.setState({ pickedServer: openServer });
            goToStep(active.id, active.step);
        }
    }, [active, openNode, openServer]);

    // 다음 자리를 기다리던 단계: 생기면 넘어간다
    const waitedNext = useRef(false);
    // biome-ignore lint/correctness/useExhaustiveDependencies: 단계가 바뀌면 처음부터
    useEffect(() => {
        waitedNext.current = false;
    }, [key]);
    useEffect(() => {
        if (waitNext) waitedNext.current = true;
        else if (waitedNext.current && nextSeen && active) {
            waitedNext.current = false;
            advanceGuide(active.id, active.step);
        }
    }, [waitNext, nextSeen, active]);

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
                    <p className="spotlight-text">{pick ? t(`guides.pick_${pick}`) : step.text}</p>
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
                        {waiting ? (
                            <span className="spotlight-wait">{t(pick ? 'guides.pick_wait' : 'guides.wait_action')}</span>
                        ) : (
                            <button
                                ref={primary}
                                type="button"
                                className="btn btn-primary btn-sm"
                                onClick={() => advanceGuide(guide.id, active.step)}
                            >
                                {k < n && !handoff ? t('guides.next') : handoff ? t('guides.go') : t('guides.finish')}
                            </button>
                        )}
                    </div>
                </div>
            )}
        </div>,
        document.body,
    );
}
