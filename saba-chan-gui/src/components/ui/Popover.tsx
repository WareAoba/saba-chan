import clsx from 'clsx';
import { Popover as P } from 'radix-ui';
import type { ReactElement, ReactNode, RefObject } from 'react';

interface PopoverProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    /** 누르면 여닫는 단추. 단추 요소 하나를 그대로 넘긴다 */
    trigger?: ReactElement;
    /** trigger 대신: 이미 있는 요소 옆에 띄운다 (여는 것은 부르는 쪽이 정한다). 이 요소를 누르는 것은 바깥으로 치지 않는다 */
    anchorRef?: RefObject<HTMLElement | null>;
    /** 창 이름 (읽어 주는 이름) */
    label: string;
    className?: string;
    align?: 'start' | 'center' | 'end';
    side?: 'top' | 'bottom' | 'left' | 'right';
    sideOffset?: number;
    /** 열 때 창 안으로 포커스를 옮길지 (기본: 옮긴다) */
    autoFocus?: boolean;
    /** 창이 처음 그려질 때 (안의 요소에 직접 포커스를 줄 때: 이벤트를 막고 옮긴다) */
    onOpenAutoFocus?: (e: Event) => void;
    children: ReactNode;
}

/**
 * 단추 옆에 뜨는 작은 창. 여닫기·Esc·바깥 누르기·포커스·화면 밖 회피는 Radix Popover가 맡고,
 * 모양은 .saba-popover(SabaUI.css)와 className이 맡는다. 창은 Portal로 body에 그린다.
 */
export function Popover({
    open,
    onOpenChange,
    trigger,
    anchorRef,
    label,
    className,
    align = 'end',
    side = 'bottom',
    sideOffset = 10,
    autoFocus = true,
    onOpenAutoFocus,
    children,
}: PopoverProps) {
    return (
        <P.Root open={open} onOpenChange={onOpenChange}>
            {trigger ? (
                <P.Trigger asChild>{trigger}</P.Trigger>
            ) : (
                anchorRef && <P.Anchor virtualRef={anchorRef as RefObject<HTMLElement>} />
            )}
            <P.Portal>
                <P.Content
                    className={clsx('saba-popover', className)}
                    aria-label={label}
                    side={side}
                    align={align}
                    sideOffset={sideOffset}
                    collisionPadding={8}
                    onOpenAutoFocus={onOpenAutoFocus ?? (autoFocus ? undefined : (e) => e.preventDefault())}
                    onInteractOutside={(e) => {
                        if (anchorRef?.current?.contains(e.target as Node)) e.preventDefault();
                    }}
                >
                    {children}
                </P.Content>
            </P.Portal>
        </P.Root>
    );
}
