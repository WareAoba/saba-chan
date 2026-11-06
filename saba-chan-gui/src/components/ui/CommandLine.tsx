import type { TFunction } from 'i18next';
import { type InputHTMLAttributes, type Ref, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import type { CommandInput } from '../../types/daemon';
import { applySuggestion, type LineProblem, type LineState } from '../../utils/commandSyntax';
import { commandInputLabel, commandText } from '../../utils/serverView';
import './CommandLine.css';

/**
 * 명령 줄 입력 부품 (사용자 11-06 #4): 앱 명령 탭과 웹 명령 탭이 함께 쓴다. 화면 상태·API는 쓰지 않는다.
 * 줄을 읽는 일은 utils/commandSyntax `readCommandLine`이 하고, 여기서는 그 결과를 그린다.
 */

interface FieldProps extends InputHTMLAttributes<HTMLInputElement> {
    state: LineState;
    inputRef?: Ref<HTMLInputElement>;
}

/** 친 글자를 색으로 나눠 보이는 입력 칸: 같은 글자를 색칠한 줄을 뒤에 깔고 칸의 글자는 투명하게 둔다 */
export function CommandLineField({ state, inputRef, className, onScroll, ...props }: FieldProps) {
    const hl = useRef<HTMLPreElement>(null);
    const follow = (el: HTMLInputElement) => {
        if (hl.current) hl.current.scrollLeft = el.scrollLeft;
    };
    return (
        <span className="cmdline-field">
            <pre className="cmdline-hl" aria-hidden="true" ref={hl}>
                {/* 한 줄로 묶는다: flex 상자 바로 아래의 빈칸만 있는 글자는 그려지지 않아 낱말이 붙어 버린다 */}
                <span className="cmdline-hl-text">
                    {state.tokens.map((tk, i) =>
                        tk.kind === 'space' ? (
                            tk.text
                        ) : (
                            // biome-ignore lint/suspicious/noArrayIndexKey: 친 줄을 낱말로 나눈 것이라 자리가 곧 이름이다
                            <span key={i} className={`tok-${tk.kind}`}>
                                {tk.text}
                            </span>
                        ),
                    )}
                </span>
            </pre>
            <input
                {...props}
                ref={inputRef}
                className={className ? `cmdline-input ${className}` : 'cmdline-input'}
                onScroll={(e) => {
                    follow(e.currentTarget);
                    onScroll?.(e);
                }}
                onKeyUp={(e) => {
                    follow(e.currentTarget);
                    props.onKeyUp?.(e);
                }}
            />
        </span>
    );
}

function rangeText(input: CommandInput, t: TFunction) {
    if (input.min != null && input.max != null) return t('command_syntax.range_both', { min: input.min, max: input.max });
    if (input.min != null) return t('command_syntax.range_min', { min: input.min });
    return t('command_syntax.range_max', { max: input.max });
}

/** 틀린 칸의 이유 (데몬 검사 규칙과 같은 이름) */
export function problemText(problem: LineProblem, label: string, t: TFunction) {
    const values = { label, value: problem.value };
    if (problem.rule === 'out_of_range' && problem.input) {
        return t('command_syntax.out_of_range', { ...values, range: rangeText(problem.input, t) });
    }
    if (problem.rule === 'not_option') {
        return t('command_syntax.not_option', { ...values, options: (problem.input?.options || []).join(' · ') });
    }
    return t(`command_syntax.${problem.rule}`, values);
}

interface HelpProps {
    text: string;
    state: LineState;
    moduleName: string;
    /** 권하는 이름·값을 눌렀을 때 바뀐 줄 */
    onPick: (text: string) => void;
}

/** 입력 칸 아래: 명령 모양(지금 칸 강조), 맞았는지·빠진 칸·틀린 이유, 이어 칠 이름이나 고를 값 */
export function CommandLineHelp({ text, state, moduleName, onPick }: HelpProps) {
    const { t } = useTranslation('gui');
    const { command, problem, missing, suggestions } = state;
    if (!text.trim()) return null;
    const labelOf = (input: CommandInput) => (command ? commandInputLabel(t, moduleName, command.name, input) : input.name);

    let tone = 'neutral';
    let message: string;
    if (!command) message = t(suggestions ? 'command_syntax.more_commands' : 'command_syntax.unknown');
    else if (problem) {
        tone = 'bad';
        message = `✕ ${problemText(problem, problem.input ? labelOf(problem.input) : '', t)}`;
    } else if (state.picking) {
        // 고를 값을 아직 치는 중: 아래 값 중에서 고르게 한다
        tone = 'need';
        message = `${labelOf(state.picking)}: ${t('command_syntax.options')}`;
    } else if (missing) {
        tone = 'need';
        message = t('command_syntax.missing', { label: labelOf(missing) });
    } else {
        tone = 'ok';
        message = `✓ ${t('command_syntax.ready')}`;
    }

    return (
        <div className="cmdline-help">
            {command && (
                <div className="cmdline-sig">
                    {commandText(t, moduleName, command).label} · <span className="cmdline-sig-name">{command.name}</span>
                    {(command.inputs || []).map((input, k) => {
                        const inner = input.options?.length ? `${labelOf(input)}: ${input.options.join('|')}` : labelOf(input);
                        const shape = input.required && input.default == null ? `<${inner}>` : `[${inner}]`;
                        return (
                            <span key={input.name} className={k === state.current ? 'cmdline-sig-cur' : undefined}>
                                {' '}
                                {shape}
                            </span>
                        );
                    })}
                </div>
            )}
            <div className={`cmdline-msg tone-${tone}`} role="status">
                {message}
            </div>
            {suggestions && (
                <div className="cmdline-suggest" aria-label={t(`command_syntax.${suggestions.kind === 'command' ? 'more_commands' : 'options'}`)}>
                    {suggestions.values.slice(0, 12).map((v) => (
                        <button
                            key={v}
                            type="button"
                            onMouseDown={(e) => e.preventDefault()}
                            onClick={() => onPick(applySuggestion(text, state, v))}
                        >
                            {v}
                        </button>
                    ))}
                </div>
            )}
        </div>
    );
}
