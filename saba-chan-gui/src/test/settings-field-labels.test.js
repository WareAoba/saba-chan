import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SettingsField } from '../components/ServerSettingsModal';

// 서버 설정 칸의 이름표가 입력 칸과 이어져야 화면 낭독기가 읽고, 이름표를 눌러도 칸으로 간다 (L18 i)
describe('서버 설정 칸의 이름표', () => {
    for (const field_type of ['text', 'password', 'number', 'file', 'select', 'boolean']) {
        it(`${field_type} 칸은 이름표로 찾을 수 있다`, () => {
            render(
                <SettingsField
                    field={{ name: 'motd', label: 'Message', field_type, options: ['a', 'b'] }}
                    value=""
                    modNs="game"
                    onChange={vi.fn()}
                />,
            );
            expect(screen.getByLabelText(/Message/)).toBeTruthy();
        });
    }
});

// 그리지 못하는 type(스키마 이전 모듈의 'textarea' 등)도 글자 칸으로 보여야 설정이 화면에서 사라지지 않는다 (MK2)
describe('모르는 설정 타입', () => {
    it('글자 칸으로 보이고 값을 바꿀 수 있다', () => {
        const onChange = vi.fn();
        render(
            <SettingsField
                field={{ name: 'motd', label: 'Message', field_type: 'textarea' }}
                value="hello"
                modNs="game"
                onChange={onChange}
            />,
        );
        const input = screen.getByLabelText(/Message/);
        expect(input.value).toBe('hello');
        fireEvent.change(input, { target: { value: 'bye' } });
        expect(onChange).toHaveBeenCalledWith('motd', 'bye');
    });
});
