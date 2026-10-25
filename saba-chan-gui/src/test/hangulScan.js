const HANGUL = /[가-힣ㄱ-ㆎ]/;

/** 주석을 같은 길이의 빈칸으로 지운다 (줄 번호와 위치를 지키려고). 문자열·템플릿(${} 중첩)·정규식 안은 그대로 둔다 */
function blankComments(src) {
    const out = src.split('');
    const stack = []; // 템플릿 안의 ${ 깊이: 각 항목은 그 식의 { 깊이
    let i = 0;
    let prev = ''; // 마지막 의미 있는 글자 (정규식/나눗셈 가리기)
    const blank = (a, b) => {
        for (let k = a; k < b; k += 1) if (out[k] !== '\n') out[k] = ' ';
    };
    const skipQuoted = (q) => {
        i += 1;
        while (i < src.length && src[i] !== q && src[i] !== '\n') i += src[i] === '\\' ? 2 : 1;
        i += 1;
    };
    // 템플릿 글자 부분: 닫는 ` 이나 ${ 까지
    const skipTemplate = () => {
        while (i < src.length) {
            if (src[i] === '\\') i += 2;
            else if (src[i] === '`') {
                i += 1;
                return;
            } else if (src[i] === '$' && src[i + 1] === '{') {
                i += 2;
                stack.push(0);
                return;
            } else i += 1;
        }
    };
    while (i < src.length) {
        const c = src[i];
        if (c === '/' && src[i + 1] === '/') {
            const end = src.indexOf('\n', i);
            const j = end < 0 ? src.length : end;
            blank(i, j);
            i = j;
        } else if (c === '/' && src[i + 1] === '*') {
            const end = src.indexOf('*/', i + 2);
            const j = end < 0 ? src.length : end + 2;
            blank(i, j);
            i = j;
        } else if (c === "'" || c === '"') {
            skipQuoted(c);
            prev = 'a';
        } else if (c === '`') {
            i += 1;
            skipTemplate();
            prev = 'a';
        } else if (c === '/' && (prev === '' || '(,=:[!&|?{};+-*%<>~^'.includes(prev))) {
            // 정규식 리터럴
            i += 1;
            let inClass = false;
            while (i < src.length && src[i] !== '\n') {
                if (src[i] === '\\') i += 1;
                else if (src[i] === '[') inClass = true;
                else if (src[i] === ']') inClass = false;
                else if (src[i] === '/' && !inClass) break;
                i += 1;
            }
            i += 1;
            prev = 'a';
        } else {
            if (stack.length && c === '{') stack[stack.length - 1] += 1;
            else if (stack.length && c === '}') {
                if (stack[stack.length - 1] === 0) {
                    stack.pop();
                    i += 1;
                    skipTemplate();
                    prev = 'a';
                    continue;
                }
                stack[stack.length - 1] -= 1;
            }
            if (!/\s/.test(c)) prev = /[\w$)\]]/.test(c) ? 'a' : c;
            i += 1;
        }
    }
    return out.join('');
}

/** pos 를 감싼 괄호들 중에 t( 호출이 있는가 (기본 문구는 번역 키가 없을 때만 보인다) */
function insideT(src, pos) {
    let depth = 0;
    for (let i = pos - 1; i >= 0 && pos - i < 2000; i -= 1) {
        const c = src[i];
        if (c === ')') depth += 1;
        else if (c === '(') {
            if (depth > 0) depth -= 1;
            else if (/(^|[^\w$.])t\s*$/.test(src.slice(Math.max(0, i - 3), i)) || /\.t\s*$/.test(src.slice(Math.max(0, i - 6), i)))
                return true;
        }
    }
    return false;
}

/** 주석과 t() 인자 밖의 한글 줄: [{ line, text }] */
export function hangulOutsideT(src) {
    const code = blankComments(src);
    const lines = code.split('\n');
    const found = [];
    let offset = 0;
    lines.forEach((line, idx) => {
        for (let k = 0; k < line.length; k += 1) {
            if (HANGUL.test(line[k]) && !insideT(code, offset + k)) {
                found.push({ line: idx + 1, text: line.trim().slice(0, 120) });
                break;
            }
        }
        offset += line.length + 1;
    });
    return found;
}
