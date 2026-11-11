import type { TFunction } from 'i18next';
import type { Notice } from '../types/global';
import { safeShowToast } from './helpers';

/** 누르면 그 게임 서버를 업데이트하는 알림 (켤 때 안내·자동 업데이트 대기). data: {instanceId, name} */
export const SERVER_UPDATE_ACTION = 'applyServerUpdate';

/**
 * 업데이트 알림을 눌렀다: 데몬에 업데이트를 시킨다. 켜진 서버는 데몬이 게임 안에 예고한 뒤 끄고 업데이트하고 다시 켠다.
 * 알림은 지우고 결과는 토스트로 알린다 (끝나면 instance.updated·instance.update_failed가 따로 온다)
 */
export async function applyServerUpdateFromNotice(notice: Notice, t: TFunction) {
    const data = (notice.data || {}) as { instanceId?: string; name?: string };
    if (!data.instanceId || !window.api?.instanceApplyUpdate) return;
    const name = data.name || data.instanceId;
    window.__sabaNotice?.removeNotice(notice.id);
    const result = await window.api.instanceApplyUpdate(data.instanceId);
    if (!result?.success) {
        safeShowToast(t('events.server_update_apply_failed', { name, error: result?.error || '' }).trim(), 'error', 5000);
    } else if (result.scheduled && result.warn_minutes > 0) {
        safeShowToast(t('events.server_update_scheduled', { name, minutes: result.warn_minutes }), 'info', 5000);
    } else {
        safeShowToast(t('events.server_update_started', { name }), 'info', 5000);
    }
}
