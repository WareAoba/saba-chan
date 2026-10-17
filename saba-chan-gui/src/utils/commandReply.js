/**
 * 명령 결과에서 서버가 돌려준 답을 줄 배열로.
 * 데몬은 RCON 응답이나 명령 뒤 콘솔 줄을 `data.response`(문자열)로 준다. REST 모듈은 객체를 줄 수 있다.
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
