/**
 * 🐟 사바쨩 — Unified UI Primitives
 *
 * <SabaToggle>   — on/off toggle switch (sm | md | lg)
 * <SabaCheckbox> — styled checkbox with animated checkmark
 * <SabaSpinner>  — consistent loading ring (xs | sm | md | lg | xl)
 */

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
export function SabaToggle({ checked, onChange, disabled, size, className, title, ...rest }) {
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
 * @param {Object}   props
 * @param {boolean}  props.checked
 * @param {Function} props.onChange
 * @param {boolean}  [props.disabled]
 * @param {'sm'|'md'} [props.size='md']
 * @param {string}   [props.className]
 * @param {string}   [props.title]
 */
export function SabaCheckbox({ checked, onChange, disabled, size, className, title, ...rest }) {
    const cls = ['saba-checkbox', size === 'sm' ? 'sm' : '', disabled ? 'disabled' : '', className || '']
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
            <span className="saba-checkbox-box" />
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
export function SabaSpinner({ size, light, className, ...rest }) {
    const cls = ['saba-spinner', size && size !== 'md' ? size : '', light ? 'light' : '', className || '']
        .filter(Boolean)
        .join(' ');
    return <span className={cls} role="status" aria-label="Loading" {...rest} />;
}
