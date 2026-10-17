import clsx from 'clsx';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useModalClose } from '../../hooks/useModalClose';

export function ModalOverlay({ className, onClose, children }) {
    const { t } = useTranslation('gui');
    return (
        <div className={className}>
            <button type="button" className="modal-backdrop" onClick={onClose} aria-label={t('modals.close')} />
            {children}
        </div>
    );
}

// Success Modal - 자동으로 2초 후 닫힘
export function SuccessModal({ title, message, onClose }) {
    const { isClosing, requestClose } = useModalClose(onClose);

    useEffect(() => {
        const timer = setTimeout(requestClose, 2000);
        return () => clearTimeout(timer);
    }, [requestClose]);

    return (
        <ModalOverlay  className={clsx('modal-overlay', { closing: isClosing })} onClose={requestClose}>
            <div className="modal success-modal" >
                <img src="./success.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
            </div>
        </ModalOverlay>
    );
}

// Failure Modal - 수동으로 닫기
export function FailureModal({ title, message, onClose }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);

    return (
        <ModalOverlay  className={clsx('modal-overlay', { closing: isClosing })} onClose={requestClose}>
            <div className="modal failure-modal" >
                <img src="./panic.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <button type="button" className="modal-button failure-button" onClick={requestClose}>
                    {t('modals.close')}
                </button>
            </div>
        </ModalOverlay>
    );
}

// Notification Modal - 정보 표시
export function NotificationModal({ title, message, onClose }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);

    return (
        <ModalOverlay  className={clsx('modal-overlay', { closing: isClosing })} onClose={requestClose}>
            <div className="modal notification-modal" >
                <img src="./notice.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <button type="button" className="modal-button notification-button" onClick={requestClose}>
                    {t('modals.confirm')}
                </button>
            </div>
        </ModalOverlay>
    );
}

// Question Modal - 확인/취소 또는 커스텀 버튼
export function QuestionModal({ title, message, detail, onConfirm, onCancel, buttons }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onCancel);

    return (
        <ModalOverlay  className={clsx('modal-overlay', { closing: isClosing })} onClose={requestClose}>
            <div className="modal question-modal" >
                <img src="./question.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                {detail && <p className="modal-detail">{detail}</p>}
                <div className="modal-buttons-group">
                    {buttons ? (
                        // 커스텀 버튼 배열
                        buttons.map((btn, idx) => (
                            <button type="button"
                                key={btn.label}
                                className={clsx('modal-button', idx === 0 ? 'question-confirm' : 'question-cancel')}
                                onClick={btn.action}
                            >
                                {btn.label}
                            </button>
                        ))
                    ) : (
                        // 기본 확인/취소 버튼
                        <>
                            <button type="button" className="modal-button question-confirm" onClick={onConfirm}>
                                {t('modals.confirm')}
                            </button>
                            <button type="button" className="modal-button question-cancel" onClick={requestClose}>
                                {t('modals.cancel')}
                            </button>
                        </>
                    )}
                </div>
            </div>
        </ModalOverlay>
    );
}
