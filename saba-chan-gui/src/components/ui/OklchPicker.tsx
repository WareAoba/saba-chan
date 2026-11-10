import clsx from 'clsx';
import {
    type KeyboardEvent,
    type PointerEvent as ReactPointerEvent,
    useCallback,
    useEffect,
    useRef,
    useState,
} from 'react';
import { useTranslation } from 'react-i18next';
import { hexToOklch, maxChroma, oklchToHex, oklchToRgb } from '../../utils/oklch';
import { Icon } from '../Icon';
import { Popover } from './Popover';
import './OklchPicker.css';

const AREA_W = 240;
const AREA_H = 152;
const GAP = 8;

interface Lch {
    l: number;
    c: number;
    h: number;
}

const fromHex = (hex: string, prevHue = 270): Lch => {
    const v = hexToOklch(hex) || { l: 0.6, c: 0.1, h: prevHue };
    return { l: v.l, c: v.c, h: v.h ?? prevHue };
};

/**
 * 판 위 자리(0~1) → 색. 세로 = 밝기, 가로 = 그 밝기에서 화면(sRGB)이 낼 수 있는 채도 중 몇 %인지.
 * 그래서 판은 빈칸 없이 꽉 찬 사각형이고 어느 점이든 낼 수 있는 색이다
 */
export function areaToLch(x: number, y: number, h: number): Lch {
    const l = 1 - y;
    return { l, c: x * maxChroma(l, h), h };
}

/** 색 → 판 위 자리 (areaToLch의 반대) */
export function lchToArea({ l, c, h }: Lch) {
    const max = maxChroma(l, h);
    return { x: max > 0 ? Math.min(1, c / max) : 0, y: 1 - l };
}

/** 밝기(세로) × 상대 채도(가로) 평면 */
function drawArea(canvas: HTMLCanvasElement, h: number) {
    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    const img = ctx.createImageData(AREA_W, AREA_H);
    for (let y = 0; y < AREA_H; y++) {
        const L = 1 - y / (AREA_H - 1);
        const max = maxChroma(L, h);
        for (let x = 0; x < AREA_W; x++) {
            const rgb = oklchToRgb(L, (x / (AREA_W - 1)) * max, h);
            img.data.set([rgb[0] * 255, rgb[1] * 255, rgb[2] * 255, 255], (y * AREA_W + x) * 4);
        }
    }
    ctx.putImageData(img, 0, 0);
}

/** 색상 띠 (밝기·채도는 고정, 범위 안으로 맞춘다) */
function drawHue(canvas: HTMLCanvasElement) {
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
function useDrag(onPoint: (x: number, y: number) => void) {
    return (e: ReactPointerEvent<HTMLElement>) => {
        const el = e.currentTarget;
        el.setPointerCapture?.(e.pointerId);
        const at = (ev: { clientX: number; clientY: number }) => {
            const r = el.getBoundingClientRect();
            onPoint(
                Math.min(1, Math.max(0, (ev.clientX - r.left) / r.width)),
                Math.min(1, Math.max(0, (ev.clientY - r.top) / r.height)),
            );
        };
        at(e);
        const move = (ev: PointerEvent) => at(ev);
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

function PickerPanel({ value, onChange }: { value: string; onChange: (hex: string) => void }) {
    const { t } = useTranslation('gui');
    const [lch, setLch] = useState(() => fromHex(value));
    const [hexText, setHexText] = useState(value);
    const areaRef = useRef<HTMLCanvasElement>(null);
    const hueRef = useRef<HTMLCanvasElement>(null);
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
        (next: Lch) => {
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

    const onArea = useDrag((x, y) => emit(areaToLch(x, y, lch.h)));
    const at = lchToArea(lch);
    const onHue = useDrag((x) => emit({ ...lch, h: x * 360 }));

    const nudge = (e: KeyboardEvent, key: string) => {
        const step = e.shiftKey ? 10 : 1;
        const map: Record<string, Record<string, number>> = {
            l: { ArrowUp: 0.01, ArrowDown: -0.01 },
            c: { ArrowRight: 0.01, ArrowLeft: -0.01 },
            h: { ArrowRight: 1, ArrowLeft: -1, ArrowUp: 1, ArrowDown: -1 },
        };
        if (key === 'area') {
            const dl = map.l[e.key];
            const dc = map.c[e.key];
            if (dl === undefined && dc === undefined) return;
            e.preventDefault();
            // 방향키는 판 위 자리를 옮긴다 (가로 1칸 = 그 밝기 최대 채도의 1%)
            const clamp = (v: number) => Math.min(1, Math.max(0, v));
            emit(areaToLch(clamp(at.x + (dc || 0) * step), clamp(at.y - (dl || 0) * step), lch.h));
        } else {
            const dh = map.h[e.key];
            if (dh === undefined) return;
            e.preventDefault();
            emit({ ...lch, h: (lch.h + dh * step + 360) % 360 });
        }
    };

    const commitHex = (text: string) => {
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
                        left: `${at.x * 100}%`,
                        top: `${at.y * 100}%`,
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
interface OklchPickerProps {
    value: string;
    onChange: (hex: string) => void;
    variant?: 'swatch' | 'field';
    label?: string;
    className?: string;
}

export function OklchPicker({ value, onChange, variant = 'field', label, className }: OklchPickerProps) {
    const [open, setOpen] = useState(false);
    return (
        <Popover
            open={open}
            onOpenChange={setOpen}
            trigger={
                <button
                    type="button"
                    className={clsx('oklch-trigger', `oklch-trigger-${variant}`, className, { open })}
                    aria-label={label}
                    data-tooltip={label}
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
            }
            label={label || ''}
            className="oklch-pop"
            sideOffset={GAP}
            autoFocus={false}
        >
            <PickerPanel value={value} onChange={onChange} />
        </Popover>
    );
}

export default OklchPicker;
