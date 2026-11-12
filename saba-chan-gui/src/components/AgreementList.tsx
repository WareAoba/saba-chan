import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Icon } from './Icon';
import { ConfirmIcon } from './Modals/Modals';
import { Dialog } from './ui/Dialog';
import { SabaCheckbox } from './ui/SabaUI';
import type { TFunction } from 'i18next';

/** module.toml [[agreements]] 한 항목 */
export interface Agreement {
    id: string;
    label?: string;
    url?: string;
}

/** 약관 이름: 모듈 locales의 agreements.<id> → 데몬이 준 label → id */
export const agreementName = (t: TFunction, module: string, a: Agreement) =>
    t(`mod_${module}:agreements.${a.id}`, { defaultValue: a.label || a.id });

export const allChecked = (agreements: Agreement[], checked: Record<string, boolean>) =>
    agreements.every((a) => checked[a.id]);

/**
 * 모듈이 선언한 약관 목록 (module.toml [[agreements]]). 약관마다 원문 링크와 동의 체크를 둔다.
 * 사람이 직접 읽고 동의해야 하는 것이라 미리 체크해 두지 않는다.
 */
interface AgreementListProps {
    module: string;
    agreements: Agreement[];
    checked: Record<string, boolean>;
    onToggle: (id: string, on: boolean) => void;
    disabled?: boolean;
}

export function AgreementList({ module, agreements, checked, onToggle, disabled }: AgreementListProps) {
    const { t } = useTranslation('gui');
    return (
        <ul className="agreement-list">
            {agreements.map((a) => {
                const name = agreementName(t, module, a);
                return (
                    <li key={a.id} className="agreement-item">
                        <SabaCheckbox
                            className="agreement-check"
                            checked={!!checked[a.id]}
                            onChange={(on) => onToggle(a.id, on)}
                            disabled={disabled}
                        >
                            {t('agreements.accept', { name })}
                        </SabaCheckbox>
                        {a.url && (
                            <button
                                type="button"
                                className="btn btn-ghost btn-sm agreement-read"
                                onClick={() => window.api?.shellOpenExternal?.(a.url as string)}
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
interface AgreementDialogProps {
    title: string;
    message: string;
    module: string;
    agreements: Agreement[];
    /** 목록 아래 한 줄 (합쳐 보기: 동의가 어느 PC에 기록되는지) */
    note?: string;
    onAccept: (ids: string[]) => void;
    onCancel: () => void;
}

export function AgreementDialog({ title, message, module, agreements, note, onAccept, onCancel }: AgreementDialogProps) {
    const { t } = useTranslation('gui');
    const [checked, setChecked] = useState<Record<string, boolean>>({});
    const ready = allChecked(agreements, checked);

    return (
        <Dialog
            open
            onOpenChange={(open) => !open && onCancel()}
            className="saba-confirm agreement-modal"
            title={title}
            description={message}
            media={<ConfirmIcon icon="alertCircle" tone="warn" />}
            footer={
                <>
                    <button type="button" className="btn btn-ghost" onClick={onCancel}>
                        {t('modals.cancel')}
                    </button>
                    <button
                        type="button"
                        className="btn btn-primary"
                        disabled={!ready}
                        onClick={() => onAccept(agreements.map((a) => a.id))}
                    >
                        {t('agreements.accept_and_start')}
                    </button>
                </>
            }
        >
            <AgreementList
                module={module}
                agreements={agreements}
                checked={checked}
                onToggle={(id, on) => setChecked((prev) => ({ ...prev, [id]: on }))}
            />
            {note && <p className="modal-detail">{note}</p>}
        </Dialog>
    );
}
