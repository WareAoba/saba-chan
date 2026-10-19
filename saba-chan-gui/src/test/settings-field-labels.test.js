import { render, screen } from '@testing-library/react';
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
