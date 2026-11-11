import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { Dialog } from '../components/ui/Dialog';

function Demo({ dismissible, onOpenChange }: { dismissible?: boolean; onOpenChange?: (o: boolean) => void }) {
    const [open, setOpen] = useState(true);
    return (
        <>
            <button type="button">behind</button>
            <Dialog
                open={open}
                onOpenChange={(o) => {
                    onOpenChange?.(o);
                    setOpen(o);
                }}
                title="Delete server"
                description="This cannot be undone."
                dismissible={dismissible}
                footer={
                    <button type="button" onClick={() => setOpen(false)}>
                        OK
                    </button>
                }
            >
                <input aria-label="name" />
            </Dialog>
        </>
    );
}

// 동작(포커스 가두기·Esc·뒤 화면 막기)은 Radix Dialog가 맡는다. 앱 쪽 계약(이름·설명·껍데기·닫기 규칙)만 본다
describe('Dialog', () => {
    it('제목·설명으로 이름 붙은 모달을 머리·본문·바닥 껍데기에 그린다', () => {
        render(<Demo />);
        const dialog = screen.getByRole('dialog', { name: 'Delete server' });
        expect(dialog).toHaveAccessibleDescription('This cannot be undone.');
        expect(dialog.classList).toContain('saba-dialog');
        expect(dialog.querySelector('.saba-dialog-body input')).toBeTruthy();
        expect(dialog.querySelector('.saba-dialog-footer')?.textContent).toBe('OK');
        // 뒤 화면은 읽기·누르기에서 빠진다
        expect(screen.queryByRole('button', { name: 'behind' })).toBeNull();
    });

    it('Esc와 닫기 단추로 닫는다', () => {
        render(<Demo />);
        fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('dismissible=false면 Esc·바깥으로 닫지 않고 닫기 단추도 없다 (진행 중 작업)', () => {
        const onOpenChange = vi.fn();
        render(<Demo dismissible={false} onOpenChange={onOpenChange} />);
        fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
        expect(onOpenChange).not.toHaveBeenCalled();
        expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: 'OK' }));
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('닫기 단추가 있다', () => {
        render(<Demo />);
        fireEvent.click(screen.getByRole('button', { name: 'Close' }));
        expect(screen.queryByRole('dialog')).toBeNull();
    });
});
