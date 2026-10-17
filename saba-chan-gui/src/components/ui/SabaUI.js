import './SabaUI.css';

function ChoiceInput({ base, track, checked, onChange, disabled, size, className, title, ...rest }) {
    const cls = [base, size, disabled ? 'disabled' : '', className]
        .filter(Boolean)
        .join(' ');
    return (
        <span className={cls} data-tooltip={title}>
            <input
                type="checkbox"
                checked={checked}
                onChange={(e) => onChange(e.target.checked)}
                disabled={disabled}
                aria-label={title}
                {...rest}
            />
            <span className={track} />
        </span>
    );
}

export function SabaToggle({ size, ...props }) {
    return <ChoiceInput base="saba-toggle" track="saba-toggle-track" size={size !== 'md' ? size : ''} {...props} />;
}

export function SabaCheckbox({ size, ...props }) {
    return <ChoiceInput base="saba-checkbox" track="saba-checkbox-box" size={size === 'sm' ? 'sm' : ''} {...props} />;
}

export function SabaSpinner({ size, light, className, ...rest }) {
    const cls = ['saba-spinner', size && size !== 'md' ? size : '', light ? 'light' : '', className || '']
        .filter(Boolean)
        .join(' ');
    return <output className={cls} aria-label="Loading" {...rest} />;
}
