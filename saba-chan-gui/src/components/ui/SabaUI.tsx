/**
 * 🐟 사바쨩 — Unified UI Primitives
 *
 * <SabaToggle>   — on/off toggle switch (sm | md | lg)
 * <SabaCheckbox> — styled checkbox with animated checkmark
 * <SabaSpinner>  — consistent loading ring (xs | sm | md | lg | xl)
 * <SabaNumberInput> — number input with app-styled step buttons
 * <SabaSlider>   — range slider (sunken track filled with accent, raised knob)
 */

import { type CSSProperties, type InputHTMLAttributes, type ReactNode, useRef } from 'react';
import { Icon } from '../Icon';
import './SabaUI.css';

/* ── Toggle Switch ────────────────────────────────────────── */

/**
 * @param {Object}   props
 * @param {boolean}  props.checked
 * @param {Function} props.onChange
 * @param {boolean}  [props.disabled]
 * @param {'sm'|'md'|'lg'} [props.size='md']
 * @param {string}   [props.className]
 * @param {string}   [props.title]
 */
interface SabaToggleProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'onChange' | 'size' | 'title'> {
    checked: boolean;
    onChange: (checked: boolean) => void;
    size?: 'sm' | 'md' | 'lg';
    title?: string;
}

export function SabaToggle({ checked, onChange, disabled, size, className, title, ...rest }: SabaToggleProps) {
    const cls = ['saba-toggle', size && size !== 'md' ? size : '', disabled ? 'disabled' : '', className || '']
        .filter(Boolean)
        .join(' ');
    return (
        <label className={cls} data-tooltip={title}>
            <input
                type="checkbox"
                checked={checked}
                onChange={(e) => onChange(e.target.checked)}
                disabled={disabled}
                {...rest}
            />
            <span className="saba-toggle-track" />
        </label>
    );
}

/* ── Checkbox ─────────────────────────────────────────────── */

/**
 * 뉴모피즘 체크박스 (16px 한 크기). children을 주면 글자까지 누르는 범위가 되고,
 * 글자가 여러 줄이어도 상자는 첫 줄에 맞춘다.
 * @param {Object}   props
 * @param {boolean}  props.checked
 * @param {Function} props.onChange
 * @param {boolean}  [props.disabled]
 * @param {string}   [props.className]
 * @param {string}   [props.title]
 * @param {import('react').ReactNode} [props.children] — 상자 옆 글자
 */
interface SabaCheckboxProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'onChange' | 'size' | 'title'> {
    checked: boolean;
    onChange: (checked: boolean) => void;
    /** 받아도 쓰지 않는다 (16px 한 크기) */
    size?: string;
    title?: string;
    children?: ReactNode;
}

export function SabaCheckbox({
    checked,
    onChange,
    disabled,
    size: _size,
    className,
    title,
    children,
    ...rest
}: SabaCheckboxProps) {
    const cls = ['saba-checkbox', disabled ? 'disabled' : '', className || ''].filter(Boolean).join(' ');
    return (
        <label className={cls} data-tooltip={title}>
            <input
                type="checkbox"
                checked={checked}
                onChange={(e) => onChange(e.target.checked)}
                disabled={disabled}
                {...rest}
            />
            <span className="saba-checkbox-box" aria-hidden="true">
                <svg viewBox="0 0 16 16" focusable="false">
                    <path d="M4 8.2 6.8 11 12 5" />
                </svg>
            </span>
            {children != null && <span className="saba-checkbox-label">{children}</span>}
        </label>
    );
}

/* ── Spinner ──────────────────────────────────────────────── */

/**
 * @param {Object}   props
 * @param {'xs'|'sm'|'md'|'lg'|'xl'} [props.size='md']
 * @param {boolean}  [props.light]     — white on dark background
 * @param {string}   [props.className]
 */
interface SabaSpinnerProps extends React.HTMLAttributes<HTMLSpanElement> {
    size?: 'xs' | 'sm' | 'md' | 'lg' | 'xl';
    light?: boolean;
}

export function SabaSpinner({ size, light, className, ...rest }: SabaSpinnerProps) {
    const cls = ['saba-spinner', size && size !== 'md' ? size : '', light ? 'light' : '', className || '']
        .filter(Boolean)
        .join(' ');
    return <span className={cls} role="status" aria-label="Loading" {...rest} />;
}

/* ── Number Input ─────────────────────────────────────────── */

/**
 * 숫자 입력칸. 브라우저 기본 다이얼 대신 앱 모양의 올림·내림 단추를 붙인다 (사용자 11-07).
 * 단추는 stepUp/stepDown 뒤 input 이벤트를 보내므로 onChange는 직접 입력과 같게 불린다.
 * 키보드는 입력칸의 화살표 키를 그대로 쓰므로 단추는 탭 순서와 읽기에서 뺀다.
 */
export function SabaNumberInput({ className, disabled, ...rest }: Omit<InputHTMLAttributes<HTMLInputElement>, 'type'>) {
    const ref = useRef<HTMLInputElement>(null);
    const step = (dir: 1 | -1) => {
        const el = ref.current;
        if (!el || disabled || el.readOnly) return;
        if (dir > 0) el.stepUp();
        else el.stepDown();
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.focus();
    };
    return (
        <span className={['saba-number', disabled ? 'disabled' : ''].filter(Boolean).join(' ')}>
            <input ref={ref} type="number" className={className} disabled={disabled} {...rest} />
            <span className="saba-number-steps" aria-hidden="true">
                <button type="button" tabIndex={-1} disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={() => step(1)}>
                    <Icon name="chevronUp" size={10} />
                </button>
                <button type="button" tabIndex={-1} disabled={disabled} onMouseDown={(e) => e.preventDefault()} onClick={() => step(-1)}>
                    <Icon name="chevronDown" size={10} />
                </button>
            </span>
        </span>
    );
}

/* ── Slider ───────────────────────────────────────────────── */

interface SabaSliderProps extends Omit<InputHTMLAttributes<HTMLInputElement>, 'type' | 'value' | 'onChange' | 'min' | 'max'> {
    value: number;
    min: number;
    max: number;
    onChange: (value: number) => void;
}

/** 막대 칸: 파인 홈에 강조색이 값만큼 차고(--fill), 볼록한 손잡이를 잡는다 */
export function SabaSlider({ value, min, max, onChange, className, style, ...rest }: SabaSliderProps) {
    const fill = max > min ? ((value - min) / (max - min)) * 100 : 0;
    return (
        <input
            type="range"
            className={['saba-slider', className].filter(Boolean).join(' ')}
            min={min}
            max={max}
            value={value}
            onChange={(e) => onChange(Number(e.target.value))}
            style={{ ...style, '--fill': `${fill}%` } as CSSProperties}
            {...rest}
        />
    );
}
