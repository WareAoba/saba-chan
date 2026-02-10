import React, { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import './Modals.css';

// Success Modal - 자동으로 2초 후 닫힘
export function SuccessModal({ title, message, onClose }) {
    useEffect(() => {
        const timer = setTimeout(onClose, 2000);
        return () => clearTimeout(timer);
    }, [onClose]);

    return (
        <div className="modal-overlay" onClick={onClose}>
            <div className="modal success-modal" onClick={e => e.stopPropagation()}>
                <div className="modal-icon success-icon">✓</div>
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
            </div>
        </div>
    );
}

// Failure Modal - 수동으로 닫기
export function FailureModal({ title, message, onClose }) {
    const { t } = useTranslation('gui');
    
    return (
        <div className="modal-overlay" onClick={onClose}>
            <div className="modal failure-modal" onClick={e => e.stopPropagation()}>
                <div className="modal-icon failure-icon">✕</div>
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <button className="modal-button failure-button" onClick={onClose}>
                    {t('modals.close')}
                </button>
            </div>
        </div>
    );
}

// Notification Modal - 정보 표시
export function NotificationModal({ title, message, onClose }) {
    const { t } = useTranslation('gui');
    
    return (
        <div className="modal-overlay" onClick={onClose}>
            <div className="modal notification-modal" onClick={e => e.stopPropagation()}>
                <div className="modal-icon notification-icon">ℹ</div>
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <button className="modal-button notification-button" onClick={onClose}>
                    {t('modals.confirm')}
                </button>
            </div>
        </div>
    );
}

// Question Modal - 확인/취소 또는 커스텀 버튼
export function QuestionModal({ title, message, detail, onConfirm, onCancel, buttons }) {
    const { t } = useTranslation('gui');
    
    return (
        <div className="modal-overlay" onClick={onCancel}>
            <div className="modal question-modal" onClick={e => e.stopPropagation()}>
                <div className="modal-icon question-icon">?</div>
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                {detail && <p className="modal-detail">{detail}</p>}
                <div className="modal-buttons-group">
                    {buttons ? (
                        // 커스텀 버튼 배열
                        buttons.map((btn, idx) => (
                            <button 
                                key={idx} 
                                className={`modal-button ${idx === 0 ? 'question-confirm' : 'question-cancel'}`}
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
                            <button className="modal-button question-cancel" onClick={onCancel}>
                                {t('modals.cancel')}
                            </button>
                        </>
                    )}
                </div>
            </div>
        </div>
    );
}
