/**
 * 중앙화된 에러 핸들링 유틸리티
 * 네트워크 에러 및 일반 에러를 사용자 친화적인 메시지로 변환
 */

/**
 * 에러 메시지를 사용자 친화적인 형태로 변환
 * @param {Error|string} error - 에러 객체 또는 메시지
 * @param {string} context - 에러 발생 컨텍스트 (예: '서버 시작', '모듈 로드')
 * @returns {string} 사용자에게 표시할 에러 메시지
 */
export function formatErrorMessage(error, context = '') {
    let errorMsg = typeof error === 'string' ? error : error.message;
    
    // 네트워크 에러 구분
    if (errorMsg.includes('ECONNREFUSED') || error.code === 'ECONNREFUSED') {
        return '데몬에 연결할 수 없습니다. 데몬이 실행중인지 확인해주세요';
    }
    
    if (errorMsg.includes('ETIMEDOUT') || error.code === 'ETIMEDOUT') {
        return '응답 시간 초과. 서버 상태를 확인해주세요';
    }
    
    if (errorMsg.includes('ENOTFOUND') || error.code === 'ENOTFOUND') {
        return '서버를 찾을 수 없습니다. 네트워크 설정을 확인해주세요';
    }
    
    // 컨텍스트와 함께 메시지 반환
    return context ? `${context} 실패: ${errorMsg}` : errorMsg;
}

/**
 * HTTP 상태 코드를 사용자 친화적인 메시지로 변환
 * @param {number} statusCode - HTTP 상태 코드
 * @param {object} moduleErrors - 모듈별 에러 메시지 정의
 * @returns {string} 사용자에게 표시할 에러 메시지
 */
export function formatHttpError(statusCode, moduleErrors = {}) {
    switch (statusCode) {
        case 400:
            return moduleErrors.bad_request || '잘못된 요청입니다';
        case 401:
        case 403:
            return moduleErrors.auth_failed || '인증에 실패했습니다. 비밀번호를 확인해주세요';
        case 404:
            return moduleErrors.not_found || '요청한 리소스를 찾을 수 없습니다';
        case 500:
            return moduleErrors.internal_server_error || '서버 내부 오류가 발생했습니다';
        case 503:
            return moduleErrors.server_not_running || '서버가 응답하지 않습니다';
        default:
            return `HTTP 오류 (${statusCode})`;
    }
}

/**
 * 재시도 로직을 포함한 API 호출 헬퍼
 * @param {Function} fn - 실행할 비동기 함수
 * @param {number} maxRetries - 최대 재시도 횟수
 * @param {number} initialDelay - 초기 지연 시간 (ms)
 * @returns {Promise} 함수 실행 결과
 */
export async function retryWithBackoff(fn, maxRetries = 3, initialDelay = 500) {
    for (let i = 0; i < maxRetries; i++) {
        try {
            return await fn();
        } catch (error) {
            if (i === maxRetries - 1) {
                throw error;
            }
            const delay = initialDelay * Math.pow(2, i);
            console.warn(`Attempt ${i + 1} failed, retrying in ${delay}ms...`, error.message);
            await new Promise((resolve) => setTimeout(resolve, delay));
        }
    }
}

/**
 * 특정 조건이 충족될 때까지 폴링
 * @param {Function} checkFn - 조건을 확인하는 함수 (true 반환 시 성공)
 * @param {number} interval - 폴링 간격 (ms)
 * @param {number} timeout - 최대 대기 시간 (ms)
 * @returns {Promise<boolean>} 조건 충족 여부
 */
export async function pollUntil(checkFn, interval = 500, timeout = 10000) {
    const startTime = Date.now();
    
    while (Date.now() - startTime < timeout) {
        try {
            const result = await checkFn();
            if (result) {
                return true;
            }
        } catch (error) {
            // 에러 무시하고 계속 시도
        }
        await new Promise(resolve => setTimeout(resolve, interval));
    }
    
    return false;
}
