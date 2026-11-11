import clsx from 'clsx';
import type { ReactNode } from 'react';

/**
 * 상태 점 + 글자 (gui-design-reference §8 status-chip, N4: 색·점·글자 3중 표시)
 * tone: ok | stop | warn | info | idle
 */
interface StatusChipProps {
    tone?: 'ok' | 'stop' | 'warn' | 'info' | 'idle';
    label: ReactNode;
    detail?: ReactNode;
    pulse?: boolean;
    className?: string;
    title?: string;
}

export function StatusChip({ tone = 'idle', label, detail, pulse = false, className, title }: StatusChipProps) {
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
interface SegmentedProps {
    items: { id: string; label: ReactNode; icon?: ReactNode }[];
    value: string;
    onChange: (id: string) => void;
    ariaLabel?: string;
    /** 가이드가 짚는 자리 이름의 앞부분: 탭마다 `data-spot="<앞부분>.<id>"` (화면 강조, guides/Spotlight) */
    spot?: string;
}

export function Segmented({ items, value, onChange, ariaLabel, spot }: SegmentedProps) {
    return (
        <div className="segmented" role="tablist" aria-label={ariaLabel}>
            {items.map((item) => (
                <button
                    key={item.id}
                    data-spot={spot ? `${spot}.${item.id}` : undefined}
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
