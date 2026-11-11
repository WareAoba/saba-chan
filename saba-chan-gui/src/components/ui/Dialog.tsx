import clsx from 'clsx';
import { Dialog as D } from 'radix-ui';
import type { ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';

interface DialogProps {
    open: boolean;
    onOpenChange: (open: boolean) => void;
    title: ReactNode;
    /** 제목 아래 한 줄 설명 (읽어 주는 설명이 된다) */
    description?: ReactNode;
    /** 바닥 줄 단추 자리 (오른쪽 정렬) */
    footer?: ReactNode;
    /** 머리 왼쪽 그림·아이콘 자리 */
    media?: ReactNode;
    size?: 'sm' | 'md' | 'lg';
    /** false면 Esc·바깥 누르기로 닫지 않고 닫기(×) 단추를 감춘다 (진행 중 작업, 꼭 답해야 하는 질문) */
    dismissible?: boolean;
    /** Esc를 창보다 먼저 받을 것이 있을 때 (창 안 찾기 창 닫기). preventDefault하면 창은 닫히지 않는다 */
    onEscapeKeyDown?: (e: KeyboardEvent) => void;
    className?: string;
    children?: ReactNode;
}

/**
 * 공통 대화상자. 포커스 가두기·Esc·바깥 누르기·뒤 화면 막기·닫힘 움직임은 Radix Dialog가 맡고,
 * 껍데기(머리·본문·바닥)와 모양은 .saba-dialog(SabaUI.css)가 맡는다. Portal로 body에 그린다.
 */
export function Dialog({
    open,
    onOpenChange,
    title,
    description,
    footer,
    media,
    size = 'md',
    dismissible = true,
    onEscapeKeyDown,
    className,
    children,
}: DialogProps) {
    const { t } = useTranslation('gui');
    const guard = (e: Event) => {
        if (!dismissible) e.preventDefault();
    };
    return (
        <D.Root open={open} onOpenChange={onOpenChange}>
            <D.Portal>
                <D.Overlay className="saba-dialog-overlay" />
                <D.Content
                    className={clsx('saba-dialog', size, className)}
                    // 처음 포커스는 창 자체에 둔다 (닫기 단추에 링이 먼저 뜨지 않게). Tab이면 안쪽 첫 칸으로 간다
                    onOpenAutoFocus={(e) => {
                        e.preventDefault();
                        (e.currentTarget as HTMLElement | null)?.focus();
                    }}
                    onEscapeKeyDown={(e) => {
                        onEscapeKeyDown?.(e);
                        guard(e);
                    }}
                    onPointerDownOutside={guard}
                    onInteractOutside={guard}
                    {...(description ? {} : { 'aria-describedby': undefined })}
                >
                    <div className="saba-dialog-head">
                        {media && <div className="saba-dialog-media">{media}</div>}
                        <div className="saba-dialog-heading">
                            <D.Title className="saba-dialog-title">{title}</D.Title>
                            {description && <D.Description className="saba-dialog-desc">{description}</D.Description>}
                        </div>
                        {dismissible && (
                            <D.Close className="icon-btn saba-dialog-x" aria-label={t('modals.close')}>
                                <Icon name="close" size="sm" />
                            </D.Close>
                        )}
                    </div>
                    {children != null && <div className="saba-dialog-body">{children}</div>}
                    {footer && <div className="saba-dialog-footer">{footer}</div>}
                </D.Content>
            </D.Portal>
        </D.Root>
    );
}
