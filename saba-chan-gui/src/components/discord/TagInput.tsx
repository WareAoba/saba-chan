import clsx from 'clsx';
import { useState } from 'react';
import { Icon } from '../Icon';
import { splitWords, WORD_COMMAS } from './instanceWords';

interface TagInputProps {
    value: string[];
    onChange: (value: string[]) => void;
    placeholder?: string;
    ariaLabel?: string;
    invalid?: string[];
    /** 띄어 쓴 말을 한 낱말로 받는다 (쉼표로만 나눈다). 서버를 부르는 말·명령 낱말 */
    allowSpaces?: boolean;
}

/**
 * 단어 칩 편집기: Enter·쉼표(allowSpaces가 아니면 스페이스도)로 추가, ×나 빈 칸에서 Backspace로 지운다.
 * 호출 접두사처럼 공백이 들어갈 수 없는 칸은 공백도 구분자로 쓴다.
 */
export function TagInput({ value, onChange, placeholder, ariaLabel, invalid = [], allowSpaces = false }: TagInputProps) {
    const [draft, setDraft] = useState('');
    const bad = new Set(invalid.map((w) => w.toLowerCase()));

    const commit = (text: string) => {
        const words = allowSpaces
            ? splitWords(text)
            : text
                  .split(/[\s,，、､]+/)
                  .map((w) => w.trim())
                  .filter(Boolean);
        if (words.length === 0) return;
        const have = new Set(value.map((w) => w.toLowerCase()));
        const added = words.filter((w) => {
            const k = w.toLowerCase();
            if (have.has(k)) return false;
            have.add(k);
            return true;
        });
        if (added.length) onChange([...value, ...added]);
        setDraft('');
    };

    return (
        // biome-ignore lint/a11y/noStaticElementInteractions lint/a11y/useKeyWithClickEvents: 빈 곳을 눌러도 입력칸에 커서가 가게 하는 보조 동작 (키보드는 input이 직접 받는다)
        <div className="word-input" onClick={(e) => e.currentTarget.querySelector('input')?.focus()}>
            {value.map((word) => (
                <span key={word} className={clsx('word-chip', { bad: bad.has(word.toLowerCase()) })} data-find="value">
                    {word}
                    <button
                        type="button"
                        className="word-chip-x"
                        aria-label={`${word} ×`}
                        onClick={(e) => {
                            e.stopPropagation();
                            onChange(value.filter((w) => w !== word));
                        }}
                    >
                        <Icon name="x" size={12} />
                    </button>
                </span>
            ))}
            <input
                value={draft}
                aria-label={ariaLabel}
                placeholder={value.length === 0 ? placeholder : ''}
                onChange={(e) => {
                    const v = e.target.value;
                    const last = v.slice(-1);
                    if (WORD_COMMAS.test(last) || (!allowSpaces && /\s/.test(last))) commit(v);
                    else setDraft(v);
                }}
                onKeyDown={(e) => {
                    // 한글 조합 중 Enter는 조합을 끝내는 키다: 여기서 넣으면 마지막 글자가 따로 남는다
                    if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                    if (e.key === 'Enter') {
                        e.preventDefault();
                        commit(draft);
                    } else if (e.key === 'Backspace' && draft === '' && value.length > 0) {
                        onChange(value.slice(0, -1));
                    }
                }}
                onBlur={() => commit(draft)}
            />
        </div>
    );
}
