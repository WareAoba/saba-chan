import type { TFunction } from 'i18next';
import { useUIStore } from '../stores/useUIStore';
import type { ApiBody } from '../types/daemon';
import { announceLastUpdateResult, updateResultMessage } from '../components/Modals/PatchNotesModal';

/**
 * 업데이트 확인 결과가 오면 알림함의 "업데이트 N개 발견" 알림을 지운다. 사바쨩 업데이트는 데몬이 묻지 않고 적용하고
 * 상단 바 업데이트 알약(UpdatePill)이 보이므로 새로 만들지 않는다. 예전 판이 남긴 알림은 localStorage에 남아 있어 지운다
 */
export function syncUpdateNotice(_data: ApiBody | null | undefined, _t: TFunction) {
    window.__sabaNotice?.removeNoticesByAction('Updater', 'openUpdateModal');
}

/**
 * 업데이트를 마쳤다: 결과를 토스트로만 띄운다. 알림함에는 업데이트 결과(패치노트) 알림 하나만 두어 여러 번 업데이트해도
 * 쌓이지 않는다. 성공했으면 남아 있던 "업데이트 N개 발견" 알림을 지운다(아직 받을 것이 남았으면 다음 확인이 다시 단다).
 * 적용이 실패했으면(데몬은 실패해도 다시 뜬다) 실패로 알리고 업데이트 알림은 둔다
 */
export async function announceUpdateCompleted(data: ApiBody | null | undefined, t: TFunction) {
    const res = await announceLastUpdateResult(t);
    // 사용자가 되돌린 것은 업데이트가 아니다: 되돌린 결과 알림만 둔다
    if (res?.result?.result === 'reverted') return;
    // 그 뒤의 업데이트: 컴포넌트 탭 위에 남은 되돌린 결과 줄은 지난 일이다
    useUIStore.getState().setRevertResult(null);
    const failed = res?.result?.success === false;
    if (!failed) window.__sabaNotice?.removeNoticesByAction('Updater', 'openUpdateModal');
    if (typeof window.showToast === 'function') {
        if (failed) window.showToast(updateResultMessage(res, t), 'error', 5000);
        else window.showToast(data?.message || t('updates.completed'), 'success', 5000);
    }
}
