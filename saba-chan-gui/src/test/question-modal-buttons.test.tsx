import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { FailureModal, NotificationModal, QuestionModal, SuccessModal } from '../components/Modals/Modals';

// 확인창은 공통 Dialog 위에 상태 아이콘 + 앱 공통 단추로 그린다 (사용자 결정 2026-11-11: B안)
describe('확인창', () => {
    it('버튼이 셋 이상이면 세로로 쌓고 주 버튼(첫 버튼)을 위에 둔다', () => {
        const buttons = ['a', 'b', 'c'].map((label) => ({ label, action: vi.fn() }));
        render(<QuestionModal title="t" message="m" buttons={buttons} onCancel={vi.fn()} />);
        const dialog = screen.getByRole('dialog', { name: 't' });
        expect(dialog).toHaveClass('stacked');
        const names = within(dialog)
            .getAllByRole('button')
            .filter((b) => b.classList.contains('btn'))
            .map((b) => b.textContent);
        expect(names).toEqual(['a', 'b', 'c']);
        expect(within(dialog).getByRole('button', { name: 'a' })).toHaveClass('btn-primary');
    });

    it('확인·취소 두 개는 한 줄에, 취소가 왼쪽이고 Esc는 취소다', () => {
        const onCancel = vi.fn();
        const onConfirm = vi.fn();
        render(<QuestionModal title="t" message="m" onConfirm={onConfirm} onCancel={onCancel} />);
        const dialog = screen.getByRole('dialog', { name: 't' });
        expect(dialog).not.toHaveClass('stacked');
        const btns = within(dialog)
            .getAllByRole('button')
            .filter((b) => b.classList.contains('btn'));
        expect(btns.map((b) => b.textContent)).toEqual(['Cancel', 'Confirm']);
        fireEvent.click(btns[1]);
        expect(onConfirm).toHaveBeenCalled();
        fireEvent.keyDown(dialog, { key: 'Escape' });
        expect(onCancel).toHaveBeenCalled();
    });

    it('옛 PNG 그림 대신 상태 아이콘을 쓴다', () => {
        const { unmount } = render(<FailureModal title="f" message="m" onClose={vi.fn()} />);
        const dialog = screen.getByRole('dialog', { name: 'f' });
        expect(dialog.querySelector('img')).toBeNull();
        expect(dialog.querySelector('.saba-confirm-icon.tone-stop')).not.toBeNull();
        unmount();
        render(<NotificationModal title="n" message="m" onClose={vi.fn()} />);
        expect(screen.getByRole('dialog', { name: 'n' }).querySelector('.saba-confirm-icon.tone-info')).not.toBeNull();
    });

    it('성공 창은 2초 뒤 저절로 닫힌다', () => {
        vi.useFakeTimers();
        const onClose = vi.fn();
        render(<SuccessModal title="s" message="m" onClose={onClose} />);
        expect(screen.getByRole('dialog', { name: 's' }).querySelector('.saba-confirm-icon.tone-ok')).not.toBeNull();
        act(() => vi.advanceTimersByTime(2000));
        expect(onClose).toHaveBeenCalled();
        vi.useRealTimers();
    });
});
