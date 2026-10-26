import { render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { QuestionModal } from '../components/Modals/Modals';

describe('질문 창 버튼', () => {
    it('버튼이 셋 이상이면 세로로 쌓는다 (긴 이름이 창 밖으로 나가지 않게)', () => {
        const buttons = ['a', 'b', 'c'].map((label) => ({ label, action: vi.fn() }));
        const { container } = render(<QuestionModal title="t" message="m" buttons={buttons} onCancel={vi.fn()} />);
        expect(container.querySelector('.modal-buttons-group').classList.contains('stacked')).toBe(true);
    });

    it('확인·취소 두 개는 한 줄에 둔다', () => {
        const { container } = render(<QuestionModal title="t" message="m" onConfirm={vi.fn()} onCancel={vi.fn()} />);
        expect(container.querySelector('.modal-buttons-group').classList.contains('stacked')).toBe(false);
    });
});
