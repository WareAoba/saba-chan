import clsx from 'clsx';
import { type ReactNode, useId, useState } from 'react';
import { Icon } from '../Icon';

interface DisclosureProps {
    /** 머리 글자 (아이콘을 앞에 둘 수 있다) */
    summary: ReactNode;
    children?: ReactNode;
    defaultOpen?: boolean;
    className?: string;
}

/**
 * 공통 펼치기. 브라우저 기본 <details>의 ▶ 대신 앱 아이콘 화살표가 돌고, 머리는 단추라 키보드로 연다.
 * 접혀 있어도 내용을 그려 둔다 (Ctrl+F가 안을 센다).
 */
export function Disclosure({ summary, children, defaultOpen = false, className }: DisclosureProps) {
    const [open, setOpen] = useState(defaultOpen);
    const id = useId();
    return (
        <div className={clsx('saba-disclosure', className, { open })}>
            <button
                type="button"
                className="saba-disclosure-head"
                aria-expanded={open}
                aria-controls={id}
                onClick={() => setOpen((o) => !o)}
            >
                <span className="saba-disclosure-chevron" aria-hidden="true">
                    <Icon name="chevronRight" size="sm" />
                </span>
                <span className="saba-disclosure-summary">{summary}</span>
            </button>
            <div id={id} className="saba-disclosure-body" hidden={!open}>
                {children}
            </div>
        </div>
    );
}
