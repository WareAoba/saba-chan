import clsx from 'clsx';

/**
 * 상태 점 + 글자 (gui-design-reference §8 status-chip, N4: 색·점·글자 3중 표시)
 * tone: ok | stop | warn | info | idle
 */
export function StatusChip({ tone = 'idle', label, detail, pulse = false, className, title }) {
    return (
        <span className={clsx('status-chip', `tone-${tone}`, { pulse }, className)} data-tooltip={title}>
            <span className="status-chip-dot" aria-hidden="true" />
            <span className="status-chip-label">{label}</span>
            {detail && <span className="status-chip-detail">{detail}</span>}
        </span>
    );
}

/**
 * 눌린 홈 안의 세그먼트 탭 (N2). items: [{ id, label, icon? }]
 */
export function Segmented({ items, value, onChange, ariaLabel }) {
    return (
        <div className="segmented" role="tablist" aria-label={ariaLabel}>
            {items.map((item) => (
                <button
                    key={item.id}
                    type="button"
                    role="tab"
                    aria-selected={value === item.id}
                    className={clsx('segmented-item', { active: value === item.id })}
                    onClick={() => onChange(item.id)}
                >
                    {item.icon}
                    <span>{item.label}</span>
                </button>
            ))}
        </div>
    );
}
