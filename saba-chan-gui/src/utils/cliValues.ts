import type { SettingField } from '../types/daemon';
import { listItems } from './validation';

/** "CLI로 보기"(`components/CliLine`)에 덧붙일 설정 값. 설정 스토어도 쓰므로 화면 없는 함수로 둔다 */

export const SECRET_KEY = /(password|passwd|token|secret)$/i;

/**
 * 설정 화면에서 CLI 줄에 덧붙일 값: 화면을 연 뒤 바꾼 것만 (`_`로 시작하는 GUI 내부 값 제외).
 * 비밀번호 칸(field_type 'password')과 *_password·*_token·*_secret 키는 값 대신 `<이름>`을 둔다.
 */
export function changedForCli(
    values: Record<string, unknown>,
    openedWith: Record<string, unknown>,
    fields: Pick<SettingField, 'name' | 'field_type'>[] = [],
) {
    const secret = new Set(fields.filter((f) => f.field_type === 'password').map((f) => f.name));
    const types = Object.fromEntries(fields.map((f) => [f.name, f.field_type]));
    // CLI에 낼 모양으로 비교한다 (목록 ['a,b']와 ['a','b']는 다른 값)
    const same = (k: string, v: unknown) => String(cliValue(types[k], v) ?? '') === String(cliValue(types[k], openedWith[k]) ?? '');
    return Object.fromEntries(
        Object.entries(values)
            .filter(([k, v]) => !k.startsWith('_') && !same(k, v))
            .map(([k, v]) => [k, secret.has(k) || SECRET_KEY.test(k) ? `<${k}>` : cliValue(types[k], v)]),
    );
}

/** CLI `server set` 문법: list는 `a,b`(항목에 쉼표가 있으면 JSON 배열), multiline은 `\`→`\\` 뒤 줄바꿈→`\n` */
function cliValue(fieldType: string | undefined, v: unknown) {
    if (fieldType === 'list') {
        const items = listItems(v);
        // 쉼표나 앞뒤 공백이 든 항목, [로 시작하는 목록은 CLI 쉼표 경로가 그대로 두지 않아 JSON 배열로
        return items.some((i: string) => i.includes(',') || i !== i.trim()) || items[0]?.startsWith('[')
            ? JSON.stringify(items)
            : items.join(',');
    }
    if (fieldType === 'multiline')
        return String(v ?? '')
            .replace(/\\/g, '\\\\')
            .replace(/\n/g, '\\n');
    return v;
}
