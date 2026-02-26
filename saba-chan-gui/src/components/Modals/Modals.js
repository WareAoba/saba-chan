import clsx from 'clsx';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';
import { useModalClose } from '../../hooks/useModalClose';

// Success Modal - 자동으로 2초 후 닫힘
export function SuccessModal({ title, message, onClose }) {
    const { isClosing, requestClose } = useModalClose(onClose);

    useEffect(() => {
        const timer = setTimeout(requestClose, 2000);
        return () => clearTimeout(timer);
    }, [requestClose]);

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal success-modal" onClick={(e) => e.stopPropagation()}>
                <img src="./success.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
            </div>
        </div>
    );
}

// Failure Modal - 수동으로 닫기
export function FailureModal({ title, message, onClose }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal failure-modal" onClick={(e) => e.stopPropagation()}>
                <img src="./panic.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <button className="modal-button failure-button" onClick={requestClose}>
                    {t('modals.close')}
                </button>
            </div>
        </div>
    );
}

// Notification Modal - 정보 표시
export function NotificationModal({ title, message, onClose }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal notification-modal" onClick={(e) => e.stopPropagation()}>
                <img src="./notice.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <button className="modal-button notification-button" onClick={requestClose}>
                    {t('modals.confirm')}
                </button>
            </div>
        </div>
    );
}

// Question Modal - 확인/취소 또는 커스텀 버튼
export function QuestionModal({ title, message, detail, onConfirm, onCancel, buttons }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onCancel);

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal question-modal" onClick={(e) => e.stopPropagation()}>
                <img src="./question.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                {detail && <p className="modal-detail">{detail}</p>}
                <div className="modal-buttons-group">
                    {buttons ? (
                        // 커스텀 버튼 배열
                        buttons.map((btn, idx) => (
                            <button
                                key={idx}
                                className={clsx('modal-button', idx === 0 ? 'question-confirm' : 'question-cancel')}
                                onClick={btn.action}
                            >
                                {btn.label}
                            </button>
                        ))
                    ) : (
                        // 기본 확인/취소 버튼
                        <>
                            <button className="modal-button question-confirm" onClick={onConfirm}>
                                {t('modals.confirm')}
                            </button>
                            <button className="modal-button question-cancel" onClick={requestClose}>
                                {t('modals.cancel')}
                            </button>
                        </>
                    )}
                </div>
            </div>
        </div>
    );
}
