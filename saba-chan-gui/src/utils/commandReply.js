/**
 * 명령 결과에서 서버가 돌려준 답을 줄 배열로.
 * 데몬은 명령 뒤 콘솔 줄을 `data.response`(문자열)로 준다. REST 모듈은 객체를 줄 수 있다.
 */
export function commandReplyLines(result) {
    const r = result?.data?.response;
    let text = '';
    if (typeof r === 'string') text = r;
    else if (r && typeof r === 'object') {
        if (typeof r.raw === 'string') text = r.raw;
        else if (Object.keys(r).length) text = JSON.stringify(r);
    } else if (r == null && typeof result?.message === 'string') {
        text = result.message;
    }
    return text
        .split(/\r?\n/)
        .map((l) => l.replace(/\s+$/, ''))
        .filter(Boolean);
}

/** 콘솔이 없는 서버 (사바쨩 밖에서 켰거나 관리 모드가 아님): 데몬 409 no_command_path */
export const isNoCommandPath = (result) =>
    result?.error_code === 'no_command_path' || result?.error === 'no_command_path';
