import type { ApiBody } from '../types/daemon';

/**
 * window.api 결과가 실패인지. 브리지는 실패하면 error를 싣고(api/daemon `failureBody`),
 * 데몬 본문은 경로마다 ok:false·success:false 중 하나를 싣거나 error·error_code만 싣는다.
 * 명령 라우트는 서버가 거절한 명령(모듈의 실패 모양)에 `rejected`를 싣는다.
 */
export const failed = (res: ApiBody | null | undefined) =>
    !res || res.ok === false || res.success === false || Boolean(res.error || res.error_code || res.rejected);
