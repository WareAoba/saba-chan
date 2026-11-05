import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
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
});
