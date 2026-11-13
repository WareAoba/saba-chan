import { useEffect } from 'react';
import type { TFunction } from 'i18next';
import { useTranslation } from 'react-i18next';
import { Icon } from '../Icon';
import { Dialog } from '../ui/Dialog';
import { announceRevert } from './AppVersion';
import { ConfirmIcon } from './Modals';
import { PATCH_NOTES_ACTION, updateResultMessage } from './NoticeModal';

export { updateResultMessage };

/**
 * 시작할 때 마지막 업데이트 결과를 묻고, 있으면 알림을 남긴다 (데몬 GET /api/updates/last-result).
 * 알림은 같은 액션끼리 하나로 합쳐서, 확인 전에 다시 켜도 쌓이지 않는다.
 * 사용자가 되돌린 결과는 데몬이 확인한 것으로 남기므로 확인한 것까지 받아 따로 한 번 알린다(업데이트로 알리지 않는다).
 */
export async function announceLastUpdateResult(t: TFunction) {
    if (!window.api?.updaterLastResult || !window.__sabaNotice) return null;
    try {
        const res = await window.api.updaterLastResult({ includeSeen: true });
        if (!res?.ok || !res.result) return null;
        if (res.result.result === 'reverted') {
            announceRevert(res.result, t);
            return res;
        }
        if (res.acknowledged) return null;
        window.__sabaNotice.addNotice({
            message: updateResultMessage(res, t),
            type: res.result.success ? 'success' : 'error',
            source: 'Updater',
            action: PATCH_NOTES_ACTION,
            dedup: true,
            data: { version: res.version || null, notes: res.notes ?? null, result: res.result },
        });
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
        Promise.resolve(window.api?.updaterLastResult?.({ clear: true })).catch(() => {});
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
