import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import CustomDropdown from '../components/CustomDropdown/CustomDropdown';
import { SabaCheckbox, SabaToggle } from '../components/ui/SabaUI';

describe('keyboard and label controls', () => {
    it.each([SabaToggle, SabaCheckbox])('renders a checkbox without nesting labels', (Control) => {
        const onChange = vi.fn();
        const { container } = render(<label htmlFor="music-control">Music<Control id="music-control" checked={false} onChange={onChange} /></label>);
        expect(container.querySelector('label label')).toBeNull();
        fireEvent.click(screen.getByRole('checkbox', { name: 'Music' }));
        expect(onChange).toHaveBeenCalledWith(true);
    });

    it('exposes dropdown options as native keyboard controls', () => {
        const onChange = vi.fn();
        render(<CustomDropdown value="a" onChange={onChange} options={[{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }]} />);
        fireEvent.click(screen.getByRole('button', { name: 'Alpha' }));
        const option = screen.getByRole('button', { name: 'Beta' });
        expect(option.tagName).toBe('BUTTON');
        fireEvent.click(option);
        expect(onChange).toHaveBeenCalledWith('b');
    });
});
