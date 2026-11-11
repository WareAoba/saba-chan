import clsx from 'clsx';
import { type MouseEventHandler, type ReactNode, useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { Icon } from '../Icon';
import { Dialog } from '../ui/Dialog';

type Tone = 'ok' | 'stop' | 'warn' | 'info';

/** 확인창 머리 왼쪽의 상태 아이콘 (사용자 결정 2026-11-11: 그림 대신 아이콘) */
export function ConfirmIcon({ icon, tone }: { icon: string; tone: Tone }) {
    return (
        <span className={`saba-confirm-icon tone-${tone}`} aria-hidden="true">
            <Icon name={icon} size="lg" />
        </span>
    );
}

interface MessageModalProps {
    title?: ReactNode;
    message?: ReactNode;
    onClose: () => void;
}

interface QuestionButton {
    label: ReactNode;
    action?: MouseEventHandler<HTMLButtonElement>;
}

interface QuestionModalProps {
    title?: ReactNode;
    message?: ReactNode;
    detail?: ReactNode;
    onConfirm?: MouseEventHandler<HTMLButtonElement>;
    onCancel: () => void;
    buttons?: QuestionButton[];
}

// Success Modal - 자동으로 2초 후 닫힘
export function SuccessModal({ title, message, onClose }: MessageModalProps) {
    useEffect(() => {
        const timer = setTimeout(onClose, 2000);
        return () => clearTimeout(timer);
    }, [onClose]);

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onClose()}
            size="sm"
            className="saba-confirm"
            title={title}
            description={message}
            media={<ConfirmIcon icon="checkCircle" tone="ok" />}
        />
    );
}

// Failure Modal - 수동으로 닫기
export function FailureModal({ title, message, onClose }: MessageModalProps) {
    const { t } = useTranslation('gui');
    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onClose()}
            size="sm"
            className="saba-confirm"
            title={title}
            description={message}
            media={<ConfirmIcon icon="xCircle" tone="stop" />}
            footer={
                <button type="button" className="btn btn-primary" onClick={onClose}>
                    {t('modals.close')}
                </button>
            }
        />
    );
}

// Notification Modal - 정보 표시
export function NotificationModal({ title, message, onClose }: MessageModalProps) {
    const { t } = useTranslation('gui');
    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onClose()}
            size="sm"
            className="saba-confirm"
            title={title}
            description={message}
            media={<ConfirmIcon icon="info" tone="info" />}
            footer={
                <button type="button" className="btn btn-primary" onClick={onClose}>
                    {t('modals.confirm')}
                </button>
            }
        />
    );
}

// Question Modal - 확인/취소 또는 커스텀 버튼
export function QuestionModal({ title, message, detail, onConfirm, onCancel, buttons }: QuestionModalProps) {
    const { t } = useTranslation('gui');
    // 버튼이 셋 이상이면 세로로 쌓고 주 버튼(첫 버튼)을 위에. 둘 이하면 한 줄에 취소 왼쪽·주 버튼 오른쪽
    const stacked = (buttons?.length ?? 0) > 2;
    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onCancel()}
            size="sm"
            className={clsx('saba-confirm', { stacked })}
            title={title}
            description={message}
            media={<ConfirmIcon icon="alertCircle" tone="warn" />}
            footer={
                buttons ? (
                    (stacked ? buttons : [...buttons].reverse()).map((btn) => (
                        <button
                            key={String(btn.label)}
                            type="button"
                            className={clsx('btn', btn === buttons[0] ? 'btn-primary' : 'btn-ghost')}
                            onClick={btn.action}
                        >
                            {btn.label}
                        </button>
                    ))
                ) : (
                    <>
                        <button type="button" className="btn btn-ghost" onClick={onCancel}>
                            {t('modals.cancel')}
                        </button>
                        <button type="button" className="btn btn-primary" onClick={onConfirm}>
                            {t('modals.confirm')}
                        </button>
                    </>
                )
            }
        >
            {detail && <p className="modal-detail">{detail}</p>}
        </Dialog>
    );
}
