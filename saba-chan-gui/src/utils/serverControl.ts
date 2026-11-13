import type { ApiBody, ModuleAgreement } from '../types/daemon';
import { failed } from './apiResult';

/** 데몬이 동의를 요구한 약관 한 건 (label은 모듈 언어로 옮긴 이름) */
export type Agreement = ModuleAgreement & { label?: string };

/**
 * 켜기·끄기·다시 켜기 답이 뜻하는 것 (앱 serverActions와 웹 useServerControl이 같이 쓴다. 화면 없는 순수 함수).
 * 무엇을 보일지(토스트·창)는 부른 쪽이 정한다
 */
export type ControlOutcome =
    /** 콘솔도 REST도 없어 저장하고 끌 길이 없다: 데몬은 서버를 그대로 뒀다. 저장 없이 끌지 묻고 force로 다시 보낸다 */
    | { kind: 'forceStop' }
    /** 모듈이 선언한 약관(EULA 등)에 아직 동의하지 않았다. 목록이 비었으면 물을 것이 없다 */
    | { kind: 'agreements'; agreements: Agreement[] }
    /** 그사이 다른 쪽에서 정지·시작·삭제했다: 실패가 아니라 그만둔 것 */
    | { kind: 'cancelled'; code: 'start_cancelled' | 'restart_cancelled' }
    /** 서버가 제때 꺼지지 않아 데몬이 다시 켜지 않았다 */
    | { kind: 'stillStopping' }
    /** 이미 켜져 있었다: "켰어요"로 알리지 않는다 */
    | { kind: 'alreadyRunning' }
    | { kind: 'failed' }
    | { kind: 'ok' };

export function controlOutcome(res: ApiBody | null | undefined): ControlOutcome {
    const code = res?.error_code;
    if (code === 'graceful_stop_unavailable') return { kind: 'forceStop' };
    if (code === 'agreement_required') {
        return { kind: 'agreements', agreements: Array.isArray(res?.agreements) ? res.agreements : [] };
    }
    if (code === 'start_cancelled' || code === 'restart_cancelled') return { kind: 'cancelled', code };
    if (code === 'still_stopping') return { kind: 'stillStopping' };
    if (failed(res)) return { kind: 'failed' };
    return res?.already_running === true ? { kind: 'alreadyRunning' } : { kind: 'ok' };
}
