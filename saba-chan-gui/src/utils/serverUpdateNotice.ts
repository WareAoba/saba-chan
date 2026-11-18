import type { TFunction } from 'i18next';
import type { Notice } from '../types/global';
import { safeShowToast } from './helpers';
import { removeNotice } from './noticeInbox';

/**
 * 데몬에 게임 서버 업데이트를 시키고 결과(실패·게임 안 예고·시작)를 토스트로 알린다. 시작(또는 예약)됐으면 true.
 * 켜진 서버는 데몬이 게임 안에 예고한 뒤 끄고 업데이트하고 다시 켠다 (끝나면 instance.updated·instance.update_failed가 따로 온다).
 * 알림함과 서버 상세의 업데이트 단추가 함께 쓴다
 */
export async function applyServerUpdate(instanceId: string, name: string, t: TFunction): Promise<boolean> {
    const result = await window.api.instanceApplyUpdate(instanceId);
    if (!result?.success) {
        safeShowToast(t('events.server_update_apply_failed', { name, error: result?.error || '' }).trim(), 'error', 5000);
        return false;
    }
    if (result.scheduled && result.warn_minutes > 0) {
        safeShowToast(t('events.server_update_scheduled', { name, minutes: result.warn_minutes }), 'info', 5000);
    } else {
        safeShowToast(t('events.server_update_started', { name }), 'info', 5000);
    }
    return true;
}

/** 업데이트 알림을 눌렀다: 업데이트를 시키고, 시작(또는 예약)됐을 때만 알림을 지운다 (거절되면 다시 시도할 수 있게 남긴다) */
export async function applyServerUpdateFromNotice(notice: Notice, t: TFunction) {
    const data = (notice.data || {}) as { instanceId?: string; name?: string };
    if (!data.instanceId) return;
    if (await applyServerUpdate(data.instanceId, data.name || data.instanceId, t)) removeNotice(notice.id);
}
