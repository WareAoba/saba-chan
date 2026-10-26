/**
 * 모듈 지원 OS (D3·M1): 데몬 OS에서 돌아가지 않는 모듈은 새 서버 만들기에서 고를 수 없다
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';

const palworld = {
    name: 'palworld',
    version: '0.1.5',
    game_name: 'Palworld',
    settings: { fields: [] },
    platforms: ['windows'],
    platform_supported: false,
};

describe('새 서버 만들기 — 지원 OS', () => {
    it('이 데몬의 OS를 지원하지 않는 모듈은 이유와 함께 막는다', () => {
        render(<AddInstanceNewServer extensions={[palworld]} servers={[]} onAddServer={vi.fn()} />);
        const card = screen.getByRole('button', { name: /Palworld/ });
        expect(card).toBeDisabled();
        expect(card).toHaveTextContent('Not on this OS');
        fireEvent.click(card);
        expect(screen.queryByRole('button', { name: /Add/i })).toBeNull();
    });
});
