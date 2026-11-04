/**
 * 🐟 사바쨩 — Unified UI Primitives
 *
 * <SabaToggle>   — on/off toggle switch (sm | md | lg)
 * <SabaCheckbox> — styled checkbox with animated checkmark
 * <SabaSpinner>  — consistent loading ring (xs | sm | md | lg | xl)
 */

import type { InputHTMLAttributes, ReactNode } from 'react';
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
