import type { CommandInput } from '../types/daemon';
import type { CommandField } from './commandRisk';

/**
 * 명령 줄 읽기 (사용자 11-06 #4): 앱·웹의 명령 입력 줄을 데몬(`crate::commands` `Request::parse`·`bind`·`check`)이
 * 읽는 방식 그대로 읽어 칸마다 맞았는지 표시한다. 첫 낱말은 명령 이름이나 별명(대소문자 무시), 그다음은 칸 순서이고
 * 마지막 칸이 남은 말을 모두 받는다. 모듈이 알려 준 칸 정보(type·min·max·options·required·default)만 쓴다.
 * 목록에 없는 명령은 막지 않는다: 데몬이 그 줄을 그대로 콘솔에 보낸다.
 */

/** 직접 치는 명령(콘솔로 가는 명령). 나머지(rest·module)는 칸이 있는 입력 창으로 받는다 */
export const isLineCommand = (c: { method?: string | null }) => c.method === 'console';

export type TokenKind = 'space' | 'name' | 'ok' | 'bad' | 'extra' | 'free';

export interface LineProblem {
    rule: 'not_number' | 'out_of_range' | 'not_option' | 'not_boolean' | 'too_many';
    input: CommandInput | null;
    value: string;
}

export interface LineState {
    tokens: { text: string; kind: TokenKind }[];
    command: CommandField | null;
    /** 지금 치는 칸 (없으면 -1) */
    current: number;
    problem: LineProblem | null;
    /** 기본값 없이 비어 있는 첫 필수 칸 */
    missing: CommandInput | null;
    /** 이어 칠 수 있는 명령 이름, 또는 지금 칸에 고를 수 있는 값 */
    suggestions: { kind: 'command' | 'option'; values: string[] } | null;
    /** 고를 값이 정해진 칸을 아직 치는 중이다 (친 앞부분이 어떤 값의 앞부분이라 틀렸다고 하지 않는다) */
    picking: CommandInput | null;
}

const BOOLEAN_WORDS = ['true', 'false', 'on', 'off', 'yes', 'no', '1', '0'];

/** 칸 값 검사 (데몬 commands::check와 같은 규칙). 맞으면 null */
function problemOf(input: CommandInput, value: string): LineProblem['rule'] | null {
    if (input.type === 'number') {
        const n = Number(value.trim());
        if (value.trim() === '' || !Number.isFinite(n)) return 'not_number';
        if ((input.min != null && n < input.min) || (input.max != null && n > input.max)) return 'out_of_range';
    } else if (input.type === 'boolean') {
        if (!BOOLEAN_WORDS.includes(value.trim().toLowerCase())) return 'not_boolean';
    }
    if (Array.isArray(input.options) && input.options.length && !input.options.includes(value)) return 'not_option';
    return null;
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** 이름이나 별명으로 명령을 찾는다 (데몬 commands::find) */
export function findCommand(commands: CommandField[], word: string) {
    return (
        commands.find((c) => same(c.name, word)) ||
        commands.find((c) => (c.aliases || []).some((a) => same(String(a), word))) ||
        null
    );
}

export function readCommandLine(text: string, commands: CommandField[]): LineState {
    const pieces = text.match(/\S+|\s+/g) || [];
    const words = pieces.filter((p) => /\S/.test(p));
    const typingNext = /\s$/.test(text);
    const command = words.length ? findCommand(commands, words[0]) : null;
    const state: LineState = {
        tokens: [],
        command,
        current: -1,
        problem: null,
        missing: null,
        suggestions: null,
        picking: null,
    };

    if (!command) {
        state.tokens = pieces.map((p) => ({ text: p, kind: /\S/.test(p) ? 'free' : 'space' }));
        if (words.length === 1 && !typingNext) {
            const head = words[0].toLowerCase();
            const values = commands
                .map((c) => c.name)
                .filter((n) => n.toLowerCase().startsWith(head) && !same(n, head));
            if (values.length) state.suggestions = { kind: 'command', values };
        }
        return state;
    }

    const inputs = command.inputs || [];
    const args = words.slice(1);
    const last = inputs.length - 1;
    // 낱말 i가 들어가는 칸 (마지막 칸은 남은 말을 모두 받는다. 칸이 없으면 -1 = 남는 말)
    const slotOf = (i: number) => (last < 0 ? -1 : Math.min(i, last));
    // 칸마다 받은 값 (데몬 bind와 같다)
    const values = inputs.map((_, k) => (k === last ? args.slice(k).join(' ') : (args[k] ?? '')));
    const badSlots = new Set<number>();
    // 마지막 낱말을 아직 치는 중인 칸 (뒤에 빈칸이 없다)
    const typingSlot = !typingNext && args.length ? slotOf(args.length - 1) : -1;
    inputs.forEach((input, k) => {
        if (k >= args.length || state.problem) return;
        const rule = problemOf(input, values[k]);
        const lower = values[k].toLowerCase();
        if (
            rule === 'not_option' &&
            k === typingSlot &&
            (input.options || []).some((o) => o.toLowerCase().startsWith(lower))
        ) {
            state.picking = input;
            return;
        }
        if (rule) {
            badSlots.add(k);
            state.problem = { rule, input, value: values[k] };
        }
    });
    if (last < 0 && args.length) state.problem = { rule: 'too_many', input: null, value: args.join(' ') };

    let w = -1;
    state.tokens = pieces.map((p) => {
        if (!/\S/.test(p)) return { text: p, kind: 'space' };
        w += 1;
        if (w === 0) return { text: p, kind: 'name' };
        const slot = slotOf(w - 1);
        if (slot < 0) return { text: p, kind: 'extra' };
        return { text: p, kind: badSlots.has(slot) ? 'bad' : 'ok' };
    });

    if (inputs.length) {
        state.current = typingNext ? Math.min(args.length, last) : Math.max(0, Math.min(args.length - 1, last));
        if (words.length === 1 && !typingNext) state.current = -1;
    }
    state.missing = inputs.find((input, k) => k >= args.length && input.required && input.default == null) || null;

    const cur = state.current >= 0 ? inputs[state.current] : null;
    if (cur && Array.isArray(cur.options) && cur.options.length) {
        const typed = typingNext || args.length <= state.current ? '' : (args[state.current] ?? '');
        const values = cur.options.filter((o) => o.toLowerCase().startsWith(typed.toLowerCase()) && o !== typed);
        if (values.length) state.suggestions = { kind: 'option', values };
    }
    return state;
}

/** 권하는 이름·값을 고르면 친 줄이 어떻게 바뀌는지: 명령 이름은 첫 낱말을, 값은 지금 치는 낱말을 바꾸고 빈칸을 붙인다 */
export function applySuggestion(text: string, state: LineState, value: string) {
    if (state.suggestions?.kind === 'command') return `${value} `;
    // 빈칸 뒤면 새 낱말로 붙이고, 아니면 치던 낱말을 바꾼다
    return /\s$/.test(text) || !text ? `${text}${value} ` : `${text.replace(/\S+$/, '')}${value} `;
}

/** 입력 창 칸 값 → 명령 인자. 빈 칸은 빼고, 숫자 칸은 숫자로, 켜고 끄는 칸은 참거짓으로 (손대지 않은 필수 칸은 false) */
export function commandArgs(inputs: CommandInput[] = [], values: Record<string, unknown> = {}) {
    const args: Record<string, unknown> = {};
    for (const input of inputs) {
        const raw = values[input.name];
        if (input.type === 'boolean') {
            // 손대지 않은 칸은 꺼진 칸이다: 필수면 false 로 보낸다 (빼면 데몬이 missing_argument 로 거절한다)
            if (raw !== undefined && raw !== '') args[input.name] = raw === true || raw === 'true';
            else if (input.required) args[input.name] = false;
            continue;
        }
        const value = String(raw ?? '').trim();
        if (!value) continue;
        args[input.name] = input.type === 'number' && Number.isFinite(Number(value)) ? Number(value) : value;
    }
    return args;
}

/** 필수 칸이 비었는지 (기본값이 있으면 데몬이 채운다) */
export const missingInputs = (inputs: CommandInput[] = [], values: Record<string, unknown> = {}) =>
    inputs.filter(
        (i) => i.required && i.default == null && i.type !== 'boolean' && !String(values[i.name] ?? '').trim(),
    );
