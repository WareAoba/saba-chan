import type { ApiBody } from '../types/daemon';

/**
 * window.api 결과가 실패인지. 브리지는 실패하면 error를 싣고(api/daemon `failureBody`),
 * 데몬 본문은 실패면 success:false·error·error_code를 싣는다. ok:false는 셸 명령(업데이터 띄우기·창)의 결과다.
 */
export const failed = (res: ApiBody | null | undefined) =>
    !res || res.ok === false || res.success === false || Boolean(res.error || res.error_code);
