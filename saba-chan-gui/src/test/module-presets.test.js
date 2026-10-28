import { fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';
import { submitWizard } from './addServerWizard';

const minecraft = {
    name: 'minecraft',
    version: '0.1.0',
    game_name: 'Minecraft',
    settings: { fields: [] },
    presets: [
        { id: 'friends', label: 'Friends', description: 'Small server for friends', settings: { max_players: 8 } },
        { id: 'hardcore', settings: { hardcore: true } },
    ],
};

const submit = submitWizard;

describe('새 서버 만들기 — 모듈 추천 프리셋', () => {
    it('프리셋이 있으면 고르게 하고, 고른 id만 preset으로 보낸다', async () => {
        const onAddServer = vi.fn().mockResolvedValue(undefined);
        render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={onAddServer} />);
        fireEvent.click(screen.getByText('Minecraft'));

        // 문구가 없으면 id를 보인다
        expect(screen.getByRole('tab', { name: 'hardcore' })).toBeTruthy();
        fireEvent.click(screen.getByRole('tab', { name: 'Friends' }));
        expect(screen.getByText('Small server for friends')).toBeTruthy();
        await submit();

        const payload = onAddServer.mock.calls[0][0];
        expect(payload.preset).toBe('friends');
        // 값 풀기는 데몬이 한다
        expect(payload.module_settings).toEqual({});
    });

    it('기본은 프리셋 없음', async () => {
        const onAddServer = vi.fn().mockResolvedValue(undefined);
        render(<AddInstanceNewServer extensions={[minecraft]} servers={[]} onAddServer={onAddServer} />);
        fireEvent.click(screen.getByText('Minecraft'));
        await submit();
        expect(onAddServer.mock.calls[0][0]).not.toHaveProperty('preset');
    });

    it('프리셋이 없는 모듈에는 줄이 없다', () => {
        render(
            <AddInstanceNewServer
                extensions={[{ ...minecraft, presets: undefined }]}
                servers={[]}
                onAddServer={vi.fn()}
            />,
        );
        fireEvent.click(screen.getByText('Minecraft'));
        expect(screen.queryByRole('tablist', { name: 'Recommended setup' })).toBeNull();
    });
});
