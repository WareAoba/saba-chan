import { fireEvent, render } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it } from 'vitest';
import { SabaNumberInput } from '../components/ui/SabaUI';

// 기본 다이얼 대신 붙인 단추도 직접 입력처럼 onChange를 부른다 (사용자 11-07)
function Harness() {
    const [v, setV] = useState('5');
    return <SabaNumberInput aria-label="n" step={5} min={0} value={v} onChange={(e) => setV(e.target.value)} />;
}

describe('SabaNumberInput', () => {
    it('올림·내림 단추가 step만큼 값을 바꾼다', () => {
        const { getByLabelText, container } = render(<Harness />);
        const [up, down] = container.querySelectorAll('.saba-number-steps button');
        fireEvent.click(up);
        expect((getByLabelText('n') as HTMLInputElement).value).toBe('10');
        fireEvent.click(down);
        fireEvent.click(down);
        expect((getByLabelText('n') as HTMLInputElement).value).toBe('0');
    });
});
