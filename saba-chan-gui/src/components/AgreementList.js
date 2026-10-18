import clsx from 'clsx';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { useModalClose } from '../hooks/useModalClose';

/** 약관 이름: 모듈 locales의 agreements.<id> → 데몬이 준 label → id */
export const agreementName = (t, module, a) => t(`mod_${module}:agreements.${a.id}`, { defaultValue: a.label || a.id });

export const allChecked = (agreements, checked) => agreements.every((a) => checked[a.id]);

/**
 * 모듈이 선언한 약관 목록 (module.toml [[agreements]]). 약관마다 원문 링크와 동의 체크를 둔다.
 * 사람이 직접 읽고 동의해야 하는 것이라 미리 체크해 두지 않는다.
 */
export function AgreementList({ module, agreements, checked, onToggle, disabled }) {
    const { t } = useTranslation('gui');
    return (
        <ul className="agreement-list">
            {agreements.map((a) => {
                const name = agreementName(t, module, a);
                return (
                    <li key={a.id} className="agreement-item">
                        <label className={clsx('agreement-check', { disabled })}>
                            <input
                                type="checkbox"
                                checked={!!checked[a.id]}
                                onChange={(e) => onToggle(a.id, e.target.checked)}
                                disabled={disabled}
                            />
                            <span>{t('agreements.accept', { name })}</span>
                        </label>
                        {a.url && (
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm agreement-read"
                                onClick={() => window.electron?.shell?.openExternal?.(a.url)}
                                aria-label={t('agreements.read', { name })}
                            >
                                <Icon name="externalLink" size="sm" /> {t('agreements.read_short')}
                            </button>
                        )}
                    </li>
                );
            })}
        </ul>
    );
}

/** 켜려는데 동의하지 않은 약관이 있을 때 (데몬 409 agreement_required) */
export function AgreementDialog({ title, message, module, agreements, onAccept, onCancel }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onCancel);
    const [checked, setChecked] = useState({});
    const ready = allChecked(agreements, checked);

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div className="modal question-modal agreement-modal" onClick={(e) => e.stopPropagation()}>
                <img src="./question.png" alt="" className="modal-illustration" />
                <h2 className="modal-title">{title}</h2>
                <p className="modal-message">{message}</p>
                <AgreementList
                    module={module}
                    agreements={agreements}
                    checked={checked}
                    onToggle={(id, on) => setChecked((prev) => ({ ...prev, [id]: on }))}
                />
                <div className="modal-buttons-group">
                    <button
                        type="button"
                        className="modal-button question-confirm"
                        disabled={!ready}
                        onClick={() => onAccept(agreements.map((a) => a.id))}
                    >
                        {t('agreements.accept_and_start')}
                    </button>
                    <button type="button" className="modal-button question-cancel" onClick={requestClose}>
                        {t('modals.cancel')}
                    </button>
                </div>
            </div>
        </div>
    );
}
