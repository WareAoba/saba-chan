import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { splitWords } from '../components/discord/instanceWords';
import { TagInput } from '../components/discord/TagInput';

describe('단어 칩 입력', () => {
    it('한글 조합 중 Enter는 단어를 넣지 않는다 (마지막 글자가 따로 남지 않게)', () => {
        const onChange = vi.fn();
        render(<TagInput value={[]} onChange={onChange} ariaLabel="words" />);
        const input = screen.getByLabelText('words');
        fireEvent.change(input, { target: { value: '시자' } });
        fireEvent.keyDown(input, { key: 'Enter', isComposing: true, keyCode: 229 });
        expect(onChange).not.toHaveBeenCalled();
        fireEvent.change(input, { target: { value: '시작' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        expect(onChange).toHaveBeenCalledWith(['시작']);
    });

    // 사용자 11-06 #2: 서버를 부르는 말은 띄어 써도 되고 쉼표는 못 쓴다 (쉼표는 낱말을 나누는 기호)
    it('띄어쓰기를 받는 칸은 쉼표로만 나누고 사이 공백은 한 칸으로 줄인다', () => {
        const onChange = vi.fn();
        render(<TagInput value={[]} onChange={onChange} ariaLabel="words" allowSpaces />);
        const input = screen.getByLabelText('words');
        fireEvent.change(input, { target: { value: '우리 마크 ' } });
        expect(onChange).not.toHaveBeenCalled();
        fireEvent.change(input, { target: { value: '우리  마크,' } });
        expect(onChange).toHaveBeenLastCalledWith(['우리 마크']);
    });
});

describe('낱말 나누기', () => {
    it('쉼표(전각·일본식 포함)로 나누고 빈 것은 뺀다', () => {
        expect(splitWords(' 우리  마크 ,마크、서바，  ,')).toEqual(['우리 마크', '마크', '서바']);
    });
});
