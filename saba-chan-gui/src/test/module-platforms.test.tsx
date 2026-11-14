/**
 * 모듈 지원 OS (D3·M1): 데몬 OS에서 돌아가지 않는 모듈은 새 서버 만들기에서 고를 수 없다
 */
import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';

import type { ModuleInfo } from '../types/daemon';

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
        render(
            <AddInstanceNewServer
                extensions={[palworld] as unknown as ModuleInfo[]}
                onAddServer={vi.fn()}
                onClose={undefined as never}
            />,
        );
        const card = screen.getByRole('button', { name: /Palworld/ });
        expect(card).toBeDisabled();
        expect(card).toHaveTextContent('Not on this OS');
        fireEvent.click(card);
        expect(screen.queryByRole('button', { name: /Add/i })).toBeNull();
    });

    it('CPU가 맞지 않는 모듈은 OS가 아니라 CPU 미지원으로 알린다', () => {
        const arch = { ...palworld, platforms: undefined, architectures: ['x86_64'], unsupported_reason: 'arch' };
        render(
            <AddInstanceNewServer
                extensions={[arch] as unknown as ModuleInfo[]}
                onAddServer={vi.fn()}
                onClose={undefined as never}
            />,
        );
        const card = screen.getByRole('button', { name: /Palworld/ });
        expect(card).toBeDisabled();
        expect(card).toHaveTextContent('Not on this CPU');
        expect(card.getAttribute('data-tooltip')).toContain('x86_64');
    });
});
