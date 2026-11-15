import { useEffect } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { Dialog } from '../ui/Dialog';
import { announceRevert } from './AppVersion';
import { ConfirmIcon } from './Modals';
import { updateResultMessage } from './NoticeModal';

export { updateResultMessage };

/**
 * 마지막 업데이트 결과를 묻는다 (데몬 GET /api/updates/last-result). 알림함 알림은 데몬이 넣는다 (`notices.rs`).
 * 사용자가 되돌린 결과는 데몬이 확인한 것으로 남기므로 확인한 것까지 받아, 컴포넌트 탭 위에 한 번 보인다.
 */
export async function announceLastUpdateResult() {
    try {
        const res = await window.api.updaterLastResult({ includeSeen: true });
        if (!res?.success || !res.result) return null;
        if (res.result.result === 'reverted') {
            announceRevert(res.result);
            return res;
        }
        if (res.acknowledged) return null;
        return res;
    } catch {
        return null;
    }
}

/** 패치노트: 알림을 누르면 열린다. 열면 데몬에 확인했다고 알린다 */
interface PatchNotesData {
    version?: string | null;
    notes?: string | null;
    result?: { success?: boolean; failed?: string[] };
}

interface PatchNotesModalProps {
    data?: PatchNotesData;
    onClose: () => void;
}

export function PatchNotesModal({ data, onClose }: PatchNotesModalProps) {
    const { t } = useTranslation('gui');
    const result = data?.result || {};
    const failed = result.failed || [];

    useEffect(() => {
        Promise.resolve(window.api.updaterLastResult({ clear: true })).catch(() => {});
    }, []);

    return (
        <Dialog
            open
            onOpenChange={(o) => !o && onClose()}
            size="md"
            className="patch-notes"
            media={
                result.success === false ? (
                    <ConfirmIcon icon="alertTriangle" tone="warn" />
                ) : (
                    <ConfirmIcon icon="rocket" tone="info" />
                )
            }
            title={data?.version ? t('patch_notes.title_version', { version: data.version }) : t('patch_notes.title')}
            footer={
                <button type="button" className="btn btn-primary" onClick={onClose}>
                    {t('modals.confirm')}
                </button>
            }
        >
                {result.success === false && (
                    <div className="notice tone-warn" role="alert">
                        <Icon name="alertTriangle" size="sm" />
                        <span>
                            {/* result.message는 업데이터의 진단 문장(영어)이라 보이지 않는다 */}
                            {t('patch_notes.failed')}
                            {failed.length > 0 && (
                                <ul className="patch-notes-failed">
                                    {failed.map((f) => (
                                        <li key={f}>{f}</li>
                                    ))}
                                </ul>
                            )}
                        </span>
                    </div>
                )}
                {result.success !== false && failed.length > 0 && (
                    <div className="notice tone-warn" role="alert">
                        <Icon name="alertTriangle" size="sm" />
                        <span>
                            {t('patch_notes.partial', { count: failed.length })}
                            <ul className="patch-notes-failed">
                                {failed.map((f) => (
                                    <li key={f}>{f}</li>
                                ))}
                            </ul>
                        </span>
                    </div>
                )}
                {data?.notes ? (
                    <div className="patch-notes-body">{data.notes}</div>
                ) : (
                    <p className="muted">{t('patch_notes.no_notes')}</p>
                )}
        </Dialog>
    );
}
