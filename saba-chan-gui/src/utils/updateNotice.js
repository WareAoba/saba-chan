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
