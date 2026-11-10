import { fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import { Popover } from '../components/ui/Popover';
import { clickOutside } from './outside';

function Demo() {
    const [open, setOpen] = useState(false);
    return (
        <>
            <Popover
                open={open}
                onOpenChange={setOpen}
                label="Panel"
                className="demo-pop"
                trigger={
                    <button type="button" aria-label="Open">
                        o
                    </button>
                }
            >
                <button type="button">inside</button>
            </Popover>
            <p>outside</p>
        </>
    );
}

// 동작은 Radix Popover가 맡는다. 앱 쪽 계약(트리거로 열고 Esc·바깥으로 닫고, 이름 붙은 창, 앱 모양 클래스)만 본다
describe('Popover', () => {
    it('트리거로 열면 이름 붙은 작은 창을 앱 모양으로 띄우고 Esc로 닫는다', () => {
        render(<Demo />);
        fireEvent.click(screen.getByLabelText('Open'));
        const dialog = screen.getByRole('dialog', { name: 'Panel' });
        expect(dialog.classList).toContain('saba-popover');
        expect(dialog.classList).toContain('demo-pop');
        expect(screen.getByLabelText('Open').getAttribute('aria-expanded')).toBe('true');
        fireEvent.keyDown(dialog, { key: 'Escape' });
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('바깥을 누르면 닫는다', async () => {
        render(<Demo />);
        fireEvent.click(screen.getByLabelText('Open'));
        await clickOutside(screen.getByText('outside'));
        expect(screen.queryByRole('dialog')).toBeNull();
    });
});
