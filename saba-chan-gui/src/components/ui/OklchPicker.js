import clsx from 'clsx';
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import ReactDOM from 'react-dom';
import { useTranslation } from 'react-i18next';
import { hexToOklch, inGamut, MAX_CHROMA, maxChroma, oklchToHex, oklchToRgb } from '../../utils/oklch';
import { Icon } from '../Icon';
import './OklchPicker.css';

const AREA_W = 240;
const AREA_H = 152;
const GAP = 8;
const POP_W = 272;
const CLOSE_MS = 140;

const fromHex = (hex, prevHue = 270) => {
    const v = hexToOklch(hex) || { l: 0.6, c: 0.1, h: prevHue };
    return { l: v.l, c: v.c, h: v.h ?? prevHue };
};

/** 밝기(세로) × 채도(가로) 평면. sRGB 밖은 비워 둔다 */
function drawArea(canvas, h) {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const img = ctx.createImageData(AREA_W, AREA_H);
    for (let y = 0; y < AREA_H; y++) {
        const L = 1 - y / (AREA_H - 1);
        for (let x = 0; x < AREA_W; x++) {
            const rgb = oklchToRgb(L, (x / (AREA_W - 1)) * MAX_CHROMA, h);
            const i = (y * AREA_W + x) * 4;
            if (inGamut(rgb)) {
                img.data[i] = rgb[0] * 255;
                img.data[i + 1] = rgb[1] * 255;
                img.data[i + 2] = rgb[2] * 255;
                img.data[i + 3] = 255;
            }
        }
    }
    ctx.putImageData(img, 0, 0);
}

/** 색상 띠 (밝기·채도는 고정, 범위 안으로 맞춘다) */
function drawHue(canvas) {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const img = ctx.createImageData(AREA_W, 1);
    for (let x = 0; x < AREA_W; x++) {
        const h = (x / (AREA_W - 1)) * 360;
        const c = Math.min(0.16, maxChroma(0.72, h));
        const rgb = oklchToRgb(0.72, c, h);
        img.data.set([rgb[0] * 255, rgb[1] * 255, rgb[2] * 255, 255], x * 4);
    }
    ctx.putImageData(img, 0, 0);
}

/** 포인터를 누른 채 움직이는 동안 계속 값을 준다 */
function useDrag(onPoint) {
    return (e) => {
        const el = e.currentTarget;
        el.setPointerCapture?.(e.pointerId);
        const at = (ev) => {
            const r = el.getBoundingClientRect();
            onPoint(
                Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width)),
                Math.min(1, Math.max(0, (ev.clientY - r.top) / r.height)),
            );
        };
        at(e);
        const move = (ev) => at(ev);
        const up = () => {
            el.removeEventListener('pointermove', move);
            el.removeEventListener('pointerup', up);
            el.removeEventListener('pointercancel', up);
        };
        el.addEventListener('pointermove', move);
        el.addEventListener('pointerup', up);
        el.addEventListener('pointercancel', up);
    };
}

function PickerPanel({ value, onChange }) {
    const { t } = useTranslation('gui');
    const [lch, setLch] = useState(() => fromHex(value));
    const [hexText, setHexText] = useState(value);
    const areaRef = useRef(null);
    const hueRef = useRef(null);
    const lastOut = useRef(value);

    // 바깥에서 값이 바뀌면 (초기화 등) 따라간다
    useEffect(() => {
        if (value?.toLowerCase() === lastOut.current?.toLowerCase()) return;
        setLch((prev) => fromHex(value, prev.h));
        setHexText(value);
        lastOut.current = value;
    }, [value]);

    useEffect(() => {
        if (hueRef.current) drawHue(hueRef.current);
    }, []);
    useEffect(() => {
        if (areaRef.current) drawArea(areaRef.current, lch.h);
    }, [lch.h]);

    const emit = useCallback(
        (next) => {
            const c = Math.min(next.c, maxChroma(next.l, next.h));
            const v = { ...next, c };
            setLch(v);
            const hex = oklchToHex(v.l, v.c, v.h);
            setHexText(hex);
            lastOut.current = hex;
            onChange(hex);
        },
        [onChange],
    );

    const onArea = useDrag((x, y) => emit({ h: lch.h, l: 1 - y, c: x * MAX_CHROMA }));
    const onHue = useDrag((x) => emit({ ...lch, h: x * 360 }));

    const nudge = (e, key) => {
        const step = e.shiftKey ? 10 : 1;
        const map = {
            l: { ArrowUp: 0.01, ArrowDown: -0.01 },
            c: { ArrowRight: 0.005, ArrowLeft: -0.005 },
            h: { ArrowRight: 1, ArrowLeft: -1, ArrowUp: 1, ArrowDown: -1 },
        };
        if (key === 'area') {
            const dl = map.l[e.key];
            const dc = map.c[e.key];
            if (dl === undefined && dc === undefined) return;
            e.preventDefault();
            emit({
                ...lch,
                l: Math.min(1, Math.max(0, lch.l + (dl || 0) * step)),
                c: Math.min(MAX_CHROMA, Math.max(0, lch.c + (dc || 0) * step)),
            });
        } else {
            const dh = map.h[e.key];
            if (dh === undefined) return;
            e.preventDefault();
            emit({ ...lch, h: (lch.h + dh * step + 360) % 360 });
        }
    };

    const commitHex = (text) => {
        const v = hexToOklch(text);
        if (!v) return setHexText(lastOut.current);
        const hex = `#${text.replace('#', '').toLowerCase()}`;
        const full = hex.length === 4 ? `#${[...hex.slice(1)].map((c) => c + c).join('')}` : hex;
        setLch({ l: v.l, c: v.c, h: v.h ?? lch.h });
        setHexText(full);
        lastOut.current = full;
        onChange(full);
    };

    return (
        <>
            <div className="oklch-area-wrap">
                <canvas
                    ref={areaRef}
                    className="oklch-area"
                    width={AREA_W}
                    height={AREA_H}
                    onPointerDown={onArea}
                    onKeyDown={(e) => nudge(e, 'area')}
                    tabIndex={0}
                    role="slider"
                    aria-label={t('settings_modal.color_picker_area', 'Lightness and chroma')}
                    aria-valuemin={0}
                    aria-valuemax={100}
                    aria-valuenow={Math.round(lch.l * 100)}
                    aria-valuetext={`L ${Math.round(lch.l * 100)}%, C ${lch.c.toFixed(3)}`}
                />
                <span
                    className="oklch-thumb"
                    style={{
                        left: `${(lch.c / MAX_CHROMA) * 100}%`,
                        top: `${(1 - lch.l) * 100}%`,
                        background: oklchToHex(lch.l, lch.c, lch.h),
                    }}
                />
            </div>
            <div className="oklch-hue-wrap">
                <canvas
                    ref={hueRef}
                    className="oklch-hue"
                    width={AREA_W}
                    height={1}
                    onPointerDown={onHue}
                    onKeyDown={(e) => nudge(e, 'hue')}
                    tabIndex={0}
                    role="slider"
                    aria-label={t('settings_modal.color_picker_hue', 'Hue')}
                    aria-valuemin={0}
                    aria-valuemax={360}
                    aria-valuenow={Math.round(lch.h)}
                />
                <span className="oklch-hue-thumb" style={{ left: `${(lch.h / 360) * 100}%` }} />
            </div>
            <div className="oklch-row">
                <span className="oklch-preview" style={{ background: hexText }} />
                <input
                    className="input oklch-hex"
                    value={hexText}
                    spellCheck={false}
                    aria-label="HEX"
                    onChange={(e) => setHexText(e.target.value)}
                    onBlur={(e) => commitHex(e.target.value)}
                    onKeyDown={(e) => e.key === 'Enter' && commitHex(e.currentTarget.value)}
                />
                <span className="oklch-readout">
                    oklch({Math.round(lch.l * 100)}% {lch.c.toFixed(3)} {Math.round(lch.h)})
                </span>
            </div>
        </>
    );
}

/**
 * OKLCH 컬러 피커. 값은 hex로 주고받는다.
 * variant: 'swatch' (강조색 무지개 견본) | 'field' (견본 + hex 글자)
 */
export function OklchPicker({ value, onChange, variant = 'field', label, className }) {
    const [open, setOpen] = useState(false);
    const [closing, setClosing] = useState(false);
    const [pos, setPos] = useState({});
    const triggerRef = useRef(null);
    const popRef = useRef(null);
    const timer = useRef(null);

    const place = useCallback(() => {
        const r = triggerRef.current?.getBoundingClientRect();
        if (!r) return;
        const popH = popRef.current?.offsetHeight || 260;
        const below = window.innerHeight - r.bottom;
        const up = below < popH + GAP * 2 && r.top > below;
        const left = Math.min(Math.max(GAP, r.right - POP_W), window.innerWidth - POP_W - GAP);
        setPos({ left, ...(up ? { bottom: window.innerHeight - r.top + GAP } : { top: r.bottom + GAP }), up });
    }, []);

    const close = useCallback(() => {
        setClosing(true);
        clearTimeout(timer.current);
        timer.current = setTimeout(() => {
            setOpen(false);
            setClosing(false);
        }, CLOSE_MS);
    }, []);

    useEffect(() => () => clearTimeout(timer.current), []);

    useLayoutEffect(() => {
        if (!open) return;
        place();
        window.addEventListener('resize', place);
        window.addEventListener('scroll', place, true);
        return () => {
            window.removeEventListener('resize', place);
            window.removeEventListener('scroll', place, true);
        };
    }, [open, place]);

    useEffect(() => {
        if (!open || closing) return;
        const down = (e) => {
            if (triggerRef.current?.contains(e.target) || popRef.current?.contains(e.target)) return;
            close();
        };
        const key = (e) => {
            if (e.key === 'Escape') {
                e.stopPropagation();
                close();
                triggerRef.current?.focus();
            }
        };
        document.addEventListener('mousedown', down);
        document.addEventListener('keydown', key, true);
        return () => {
            document.removeEventListener('mousedown', down);
            document.removeEventListener('keydown', key, true);
        };
    }, [open, closing, close]);

    const shown = open && !closing;
    const { up, ...style } = pos;

    return (
        <>
            <button
                type="button"
                ref={triggerRef}
                className={clsx('oklch-trigger', `oklch-trigger-${variant}`, className, { open: shown })}
                aria-label={label}
                title={label}
                aria-haspopup="dialog"
                aria-expanded={shown}
                onClick={() => {
                    if (shown) return close();
                    clearTimeout(timer.current);
                    setClosing(false);
                    setOpen(true);
                }}
            >
                {variant === 'swatch' ? (
                    <Icon name="edit" size="sm" />
                ) : (
                    <>
                        <span className="color-input-preview" style={{ background: value }} />
                        <span className="color-input-hex">{value}</span>
                    </>
                )}
            </button>
            {open &&
                ReactDOM.createPortal(
                    <div
                        ref={popRef}
                        className={clsx('oklch-pop', { closing, up })}
                        style={{ position: 'fixed', zIndex: 10000, width: POP_W, ...style }}
                        role="dialog"
                        aria-label={label}
                    >
                        <PickerPanel value={value} onChange={onChange} />
                    </div>,
                    document.body,
                )}
        </>
    );
}

export default OklchPicker;
