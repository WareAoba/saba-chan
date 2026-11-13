import type { Agreement, ApiBody } from '../types/daemon';
import { failed } from './apiResult';

/**
 * 켜기·끄기·다시 켜기 답이 뜻하는 것 (앱 serverActions와 웹 useServerControl이 같이 쓴다. 화면 없는 순수 함수).
 * 무엇을 보일지(토스트·창)는 부른 쪽이 정한다
 */
export type ControlOutcome =
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
    if (code === 'agreement_required') {
        return { kind: 'agreements', agreements: Array.isArray(res?.agreements) ? res.agreements : [] };
    }
    if (code === 'start_cancelled' || code === 'restart_cancelled') return { kind: 'cancelled', code };
    if (code === 'still_stopping') return { kind: 'stillStopping' };
    if (failed(res)) return { kind: 'failed' };
    return res?.already_running === true ? { kind: 'alreadyRunning' } : { kind: 'ok' };
}
