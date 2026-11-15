import type { TFunction } from 'i18next';
import { useUIStore } from '../stores/useUIStore';
import type { ApiBody } from '../types/daemon';
import { announceLastUpdateResult, updateResultMessage } from '../components/Modals/PatchNotesModal';

/**
 * 업데이트를 마쳤다: 결과를 토스트로만 띄운다. 알림함의 업데이트 결과(패치노트) 알림은 데몬이 하나만 둔다 (`notices.rs`).
 * 적용이 실패했으면(데몬은 실패해도 다시 뜬다) 실패로 알린다
 */
export async function announceUpdateCompleted(data: ApiBody | null | undefined, t: TFunction) {
    const res = await announceLastUpdateResult();
    // 사용자가 되돌린 것은 업데이트가 아니다: 되돌린 결과 표시만 둔다
    if (res?.result?.result === 'reverted') return;
    // 그 뒤의 업데이트: 컴포넌트 탭 위에 남은 되돌린 결과 줄은 지난 일이다
    useUIStore.getState().setRevertResult(null);
    const failed = res?.result?.success === false;
    if (typeof window.showToast === 'function') {
        if (failed) window.showToast(updateResultMessage(res, t), 'error', 5000);
        else window.showToast(data?.message || t('updates.completed'), 'success', 5000);
    }
}
