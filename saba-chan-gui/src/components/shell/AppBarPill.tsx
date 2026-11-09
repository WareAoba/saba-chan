import clsx from 'clsx';
import { type HTMLAttributes, type ReactNode, type Ref, useLayoutEffect, useRef, useState } from 'react';
import { motionAllowed } from '../../hooks/useSwapAnimation';

/** 접힌 알약(원)의 지름. 알약 높이와 같다 */
const CIRCLE = 30;
/** 튕기며 생기기 */
const SPRING = 'cubic-bezier(.3,1.5,.5,1)';
/** 늘어나기·너비 바뀌기 */
const GROW = 'cubic-bezier(.3,1.25,.5,1)';
/** 원이 다 생긴 뒤 늘어나기 시작 */
const GROW_DELAY_MS = 260;
/** 늘어나기 시작한 뒤 글자가 보이기 시작 */
const LABEL_DELAY_MS = 120;

type Timer = ReturnType<typeof setTimeout>;

function canAnimate(el: HTMLElement | null) {
    return !!el && typeof el.animate === 'function' && motionAllowed();
}

/** 지금 글자대로라면 차지할 너비 (움직이는 중의 인라인 너비를 잠깐 풀고 잰다) */
function naturalWidth(el: HTMLElement) {
    const w = el.style.width;
    el.style.width = 'auto';
    const n = el.getBoundingClientRect().width;
    el.style.width = w;
    return n;
}

const done = (a: Animation | undefined) => (a?.finished ?? Promise.resolve()).catch(() => undefined);

/**
 * 상단 바 알약 공용 움직임 (업데이트 알림 시안 v8).
 * 나타날 때: 원이 튕기며 생긴 뒤 오른쪽 끝을 붙인 채 왼쪽으로 늘어난다.
 * 사라질 때: 글자가 빠지며 원으로 줄었다가 작아지며 없어진다. 그동안은 그려 둔다.
 * 글자가 바뀌어 너비가 달라지면 너비를 옮겨 간다. 움직임을 끄면(설정·시스템) 바로 바뀐다.
 * 반환: 붙일 ref와 지금 그려야 하는지(사라지는 중에도 true)
 */
export function usePillMotion<T extends HTMLElement>(visible: boolean, changeKey?: unknown) {
    const ref = useRef<T>(null);
    const [rendered, setRendered] = useState(visible);
    const shown = useRef(false); // 나타난 상태(나타나는 중 포함)
    const entering = useRef(false);
    const width = useRef(0); // 마지막으로 맞춘 너비
    const lastKey = useRef(changeKey);
    const timers = useRef<Timer[]>([]);
    if (visible && !rendered) setRendered(true);

    const clearTimers = () => {
        for (const t of timers.current) clearTimeout(t);
        timers.current = [];
    };
    const later = (ms: number, fn: () => void) => timers.current.push(setTimeout(fn, ms));

    // biome-ignore lint/correctness/useExhaustiveDependencies: 보임이 바뀔 때만 나타나기·사라지기를 시작한다
    useLayoutEffect(() => {
        const el = ref.current;
        if (visible) {
            if (!el || shown.current) return;
            shown.current = true;
            clearTimers();
            for (const a of el.getAnimations?.() ?? []) a.cancel();
            if (!canAnimate(el)) {
                delete el.dataset.collapsed;
                el.style.width = '';
                width.current = el.getBoundingClientRect().width;
                return;
            }
            entering.current = true;
            el.dataset.collapsed = '1';
            const to = naturalWidth(el);
            el.style.width = `${CIRCLE}px`;
            // 1) 원이 튕기며 생긴다
            el.animate([{ transform: 'scale(0)' }, { transform: 'scale(1)' }], { duration: 420, easing: SPRING });
            el.querySelector<HTMLElement>('.appbar-pill-icon')?.animate?.(
                [{ transform: 'scale(0) rotate(-90deg)' }, { transform: 'scale(1) rotate(0)' }],
                { duration: 480, easing: SPRING },
            );
            // 2) 오른쪽 끝을 붙인 채 왼쪽으로 늘어난다
            later(GROW_DELAY_MS, () => {
                if (!shown.current) return;
                const grow = el.animate([{ width: `${CIRCLE}px` }, { width: `${to}px` }], { duration: 460, easing: GROW });
                el.style.width = `${to}px`;
                later(LABEL_DELAY_MS, () => {
                    if (shown.current) delete el.dataset.collapsed;
                });
                done(grow).then(() => {
                    if (!shown.current) return;
                    entering.current = false;
                    el.style.width = '';
                    width.current = naturalWidth(el);
                });
            });
            return;
        }
        if (!shown.current) {
            setRendered(false);
            return;
        }
        shown.current = false;
        entering.current = false;
        clearTimers();
        if (!el || !canAnimate(el)) {
            setRendered(false);
            return;
        }
        const from = el.getBoundingClientRect().width;
        el.dataset.collapsed = '1';
        el.style.width = `${CIRCLE}px`;
        // 1) 글자가 빠지고 원으로 줄어든다  2) 원이 작아지며 사라진다
        (async () => {
            await done(el.animate([{ width: `${from}px` }, { width: `${CIRCLE}px` }], { duration: 320, easing: 'cubic-bezier(.5,0,.3,1)' }));
            if (shown.current) return;
            await done(
                el.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.12)', offset: 0.35 }, { transform: 'scale(0)' }], {
                    duration: 260,
                    easing: 'ease-in',
                    fill: 'forwards',
                }),
            );
            if (shown.current) return;
            el.style.width = '';
            setRendered(false);
        })();
    }, [visible, rendered]);

    // 상태가 바뀌어 너비가 달라지면 옛 너비에서 새 너비로 옮겨 간다 (나타나고 사라지는 중에는 그쪽이 맡는다)
    useLayoutEffect(() => {
        const el = ref.current;
        const keyChanged = !Object.is(lastKey.current, changeKey);
        lastKey.current = changeKey;
        if (!el || !shown.current || entering.current) return;
        const to = naturalWidth(el);
        const from = width.current;
        width.current = to;
        if (!canAnimate(el)) return;
        if (from > 0 && Math.abs(from - to) > 0.5) {
            el.animate([{ width: `${from}px` }, { width: `${to}px` }], { duration: 380, easing: GROW });
        }
        if (keyChanged) {
            el.querySelector<HTMLElement>('.appbar-pill-icon')?.animate?.([{ transform: 'scale(.4)' }, { transform: 'scale(1)' }], {
                duration: 420,
                easing: SPRING,
            });
        }
    });

    // biome-ignore lint/correctness/useExhaustiveDependencies: 떼어질 때 남은 타이머만 정리한다
    useLayoutEffect(() => () => clearTimers(), []);

    return { ref, rendered };
}

type PillElement = HTMLButtonElement & HTMLOutputElement;

interface AppBarPillProps extends Omit<HTMLAttributes<PillElement>, 'children'> {
    /** false가 되면 사라지는 움직임 뒤에 떼어진다 */
    visible: boolean;
    /** 원 안의 그림 (진행 고리·체크 등) */
    icon: ReactNode;
    label: ReactNode;
    /** 색 모양: tone-<값> 클래스 */
    tone?: string;
    /** 누를 수 있으면 button, 보이기만 하면 output */
    as?: 'button' | 'output';
    /** 바뀌면 원 안의 그림이 한 번 튄다 (상태가 바뀐 때) */
    changeKey?: unknown;
    /** 알약 옆에 함께 붙였다 떼는 것 (팝오버). 있으면 둘을 popover-anchor로 묶는다 */
    after?: ReactNode;
    anchorRef?: Ref<HTMLSpanElement>;
}

/**
 * 상단 바 알약 한 개 (작업 진행·Python 준비·업데이트가 같이 쓴다). 모양은 shell.css `.appbar-pill`
 */
export function AppBarPill({ visible, icon, label, tone, as = 'output', changeKey, after, anchorRef, className, ...rest }: AppBarPillProps) {
    const { ref, rendered } = usePillMotion<PillElement>(visible, changeKey ?? tone);
    if (!rendered) return null;
    const Tag = as;
    const pill = (
        <Tag
            ref={ref}
            {...(as === 'button' ? { type: 'button' as const } : {})}
            className={clsx('appbar-preflight appbar-pill', tone && `tone-${tone}`, className)}
            {...rest}
        >
            <span className="appbar-pill-icon">{icon}</span>
            <span className="appbar-preflight-text appbar-pill-label">{label}</span>
        </Tag>
    );
    if (after === undefined && !anchorRef) return pill;
    return (
        <span className="popover-anchor" ref={anchorRef}>
            {pill}
            {after}
        </span>
    );
}
