import { announceLastUpdateResult } from '../components/Modals/PatchNotesModal';

/**
 * 업데이트 상태를 알림함에 맞춘다: 받을 것이 있으면 "업데이트 N개 발견" 알림을 하나만 두고,
 * 없어지면(적용을 마쳤다) 그 알림을 지운다. 알림은 localStorage에 남으므로 지우지 않으면 계속 보인다
 */
export function syncUpdateNotice(data, t) {
    const notice = window.__sabaNotice;
    if (!notice) return;
    const count = data?.count || data?.updates_available || 0;
    if (count === 0) {
        notice.removeNoticesByAction('Updater', 'openUpdateModal');
        return;
    }
    const names = data.names || data.update_names || [];
    notice.addNotice({
        message: `📦 ${t('updates.found', { count, names: names.join(', ') || t('updates.check_required') })}`,
        type: 'info',
        source: 'Updater',
        action: 'openUpdateModal',
        dedup: true,
    });
}

/**
 * 업데이트를 마쳤다: 완료는 토스트로만 띄운다. 알림함에는 업데이트 결과(패치노트) 알림 하나만 두어 여러 번 업데이트해도
 * 쌓이지 않는다. 남아 있던 "업데이트 N개 발견" 알림은 지운다. 아직 받을 것이 남았으면 다음 확인이 다시 단다
 */
export async function announceUpdateCompleted(data, t) {
    window.__sabaNotice?.removeNoticesByAction('Updater', 'openUpdateModal');
    if (typeof window.showToast === 'function') {
        window.showToast(data?.message || t('updates.completed'), 'success', 5000);
    }
    await announceLastUpdateResult(t);
}
