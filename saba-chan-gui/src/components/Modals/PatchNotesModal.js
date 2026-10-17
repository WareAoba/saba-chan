import clsx from 'clsx';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { useModalClose } from '../../hooks/useModalClose';
import { Icon } from '../Icon';
import { PATCH_NOTES_ACTION } from './NoticeModal';

/** 업데이트 결과 마커 → 알림 한 줄 */
export function updateResultMessage(res, t) {
    const r = res?.result || {};
    const version = res?.version || '';
    if (r.success) return t('patch_notes.notice_success', { version: version || t('patch_notes.unknown_version') });
    return t('patch_notes.notice_failed', { count: (r.failed || []).length || 1 });
}

/**
 * 시작할 때 마지막 업데이트 결과를 묻고, 있으면 알림을 남긴다 (데몬 GET /api/updates/last-result).
 * 알림은 같은 액션끼리 하나로 합쳐서, 확인 전에 다시 켜도 쌓이지 않는다.
 */
export async function announceLastUpdateResult(t) {
    if (!window.api?.updaterLastResult || !window.__sabaNotice) return null;
    try {
        const res = await window.api.updaterLastResult();
        if (!res?.ok || !res.result) return null;
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
export function PatchNotesModal({ data, onClose }) {
    const { t } = useTranslation('gui');
    const { isClosing, requestClose } = useModalClose(onClose);
    const result = data?.result || {};
    const failed = result.failed || [];

    useEffect(() => {
        Promise.resolve(window.api?.updaterLastResult?.({ clear: true })).catch(() => {});
    }, []);

    return (
        <div className={clsx('modal-overlay', { closing: isClosing })} onClick={requestClose}>
            <div
                className="modal patch-notes-modal"
                role="dialog"
                aria-label={t('patch_notes.title')}
                onClick={(e) => e.stopPropagation()}
            >
                <h2 className="modal-title">
                    {data?.version ? t('patch_notes.title_version', { version: data.version }) : t('patch_notes.title')}
                </h2>
                {result.success === false && (
                    <div className="notice tone-warn" role="alert">
                        <Icon name="alertTriangle" size="sm" />
                        <span>
                            {result.message || t('patch_notes.failed')}
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
                    <p className="modal-message">{t('patch_notes.no_notes')}</p>
                )}
                <button type="button" className="btn btn-primary" onClick={requestClose}>
                    {t('modals.confirm')}
                </button>
            </div>
        </div>
    );
}
