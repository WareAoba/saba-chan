import type { TFunction } from 'i18next';
import type { Agreement, ApiBody } from '../types/daemon';
import { failed } from './apiResult';
import { busyMessage, installFailureText } from './helpers';
import { blockedText } from './trustNotice';

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

/**
 * 게임 서버 설치(`moduleInstallServer`) 답의 실패 문구, 성공이면 null.
 * 서버 목록의 "실행 파일 없음 → 새로 설치"와 설정 화면의 버전 설치가 같이 쓴다 (`translate`는 translateError)
 */
export function installFailure(t: TFunction, res: ApiBody | null | undefined, translate: (message?: string) => string) {
    const blocked = blockedText(res, t);
    if (blocked) return blocked;
    if (!failed(res)) return null;
    const code = res?.error_code;
    if (code === 'extension_required') {
        const extensions = (Array.isArray(res?.missing_extensions) ? res.missing_extensions : []).join(', ');
        return res?.error || t('server_settings.extension_required_for_install', { extensions });
    }
    if (code === 'version_unknown') return t('servers.version_unknown', { error: res?.error });
    return busyMessage(t, code, 'install_server_running') || installFailureText(t, res ?? undefined, translate);
}
