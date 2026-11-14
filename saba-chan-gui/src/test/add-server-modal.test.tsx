/**
 * 서버 추가 창 (사용자 11-11 A안): 고르기 화면 없이 게임 목록부터. 가져오기·직접 만들기도 같은 줄의 칸이고,
 * 창은 공통 Dialog(반투명 모드에서 유리)다.
 */
import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentType } from 'react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceModal as AddInstanceModalImpl } from '../components/Modals/AddInstanceModal';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const AddInstanceModal = AddInstanceModalImpl as unknown as ComponentType<Record<string, unknown>>;

const minecraft = { name: 'minecraft', version: '0.1.0', game_name: 'Minecraft', settings: { fields: [] } };

const openModal = () =>
    render(<AddInstanceModal isOpen onClose={vi.fn()} extensions={[minecraft]} servers={[]} onAddServer={vi.fn()} />);

describe('서버 추가 창', () => {
    it('공통 대화상자에 게임 목록부터 보이고, 가져오기·직접 만들기 칸이 같은 줄에 있다', () => {
        openModal();
        const dialog = screen.getByRole('dialog');
        expect(dialog.classList.contains('saba-dialog')).toBe(true);
        expect(screen.getByRole('heading', { name: 'Add Server' })).toBeTruthy();
        expect(screen.getByRole('button', { name: /Minecraft/ })).toBeTruthy();
        expect(screen.getByRole('button', { name: /Import a server you have/ })).toBeTruthy();
        expect(screen.getByRole('button', { name: /Make one yourself/ })).toBeTruthy();
    });

    it('게임을 찾는 칸으로 목록을 좁힌다', () => {
        render(
            <AddInstanceModal
                isOpen
                onClose={vi.fn()}
                extensions={[minecraft, { ...minecraft, name: 'palworld', game_name: 'Palworld' }]}
                servers={[]}
                onAddServer={vi.fn()}
            />,
        );
        fireEvent.change(screen.getByRole('searchbox', { name: 'Find a game' }), { target: { value: 'pal' } });
        expect(screen.queryByRole('button', { name: /Minecraft/ })).toBeNull();
        expect(screen.getByRole('button', { name: /Palworld/ })).toBeTruthy();
    });

    it('가져오기 칸은 같은 창에서 가져오기 화면을 열고, 이전으로 게임 목록에 돌아온다', async () => {
        openModal();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Import a server you have/ })));
        expect(screen.getByRole('heading', { name: 'Import Existing Server' })).toBeTruthy();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Back' })));
        expect(screen.getByRole('button', { name: /Minecraft/ })).toBeTruthy();
    });

    it('직접 만들기 칸은 만드는 방법을 펼쳐 보인다', async () => {
        window.api.makerInfo = vi.fn().mockResolvedValue({ success: true, repo: 'https://github.com/x/y', guide: 'g' }) as never;
        openModal();
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Make one yourself/ })));
        expect(await screen.findByText('git clone https://github.com/x/y')).toBeTruthy();
    });

    it('상세 설정에서 찾기 창이 떠 있으면 Esc는 찾기 창만 닫고 서버 추가 창은 남는다', async () => {
        window.api.moduleListVersions = vi.fn().mockResolvedValue({ versions: [] }) as never;
        const withField = {
            ...minecraft,
            settings: { fields: [{ name: 'max_players', field_type: 'number', label: 'Max Players', default: 20 }] },
        };
        const onClose = vi.fn();
        render(<AddInstanceModal isOpen onClose={onClose} extensions={[withField]} servers={[]} onAddServer={vi.fn()} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Minecraft/ })));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Detailed settings/ })));
        await act(async () => fireEvent.keyDown(window, { key: 'f', ctrlKey: true }));
        const find = screen.getByRole('searchbox', { name: /Find/ });
        await act(async () => fireEvent.keyDown(find, { key: 'Escape' }));
        expect(screen.queryByRole('search')).toBeNull();
        expect(onClose).not.toHaveBeenCalled();
        expect(screen.getByRole('dialog')).toBeTruthy();
    });
});
