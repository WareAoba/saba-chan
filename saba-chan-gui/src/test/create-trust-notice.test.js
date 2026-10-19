import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceMigration } from '../components/Modals/AddInstanceMigration';
import { AddInstanceNewServer } from '../components/Modals/AddInstanceNewServer';

const renderNew = (trust) => {
    render(
        <AddInstanceNewServer
            extensions={[{ name: 'game', version: '1.0.0', trust }]}
            servers={[]}
            onAddServer={vi.fn()}
        />,
    );
    fireEvent.click(screen.getByRole('button', { name: /game/ }));
};

describe('서버 만들 때 신뢰 안내', () => {
    it('확인 안 된 모듈이면 게임 서버는 격리되지 않는다고 알린다', () => {
        renderNew('unverified');
        const notice = screen.getByRole('note');
        expect(notice.textContent).toMatch(/not from the official catalog/);
        expect(notice.textContent).toMatch(/game server .*is not isolated/);
    });

    it('설치 뒤 바뀐 모듈이면 바뀌었다고 알린다', () => {
        renderNew('modified');
        expect(screen.getByRole('note').textContent).toMatch(/changed after installing/);
    });

    it('확인된 모듈이나 trust가 없는 옛 데몬이면 알리지 않는다', () => {
        renderNew('verified');
        expect(screen.queryByRole('note')).toBeNull();
    });

    it('기존 서버 가져오기에서도 확인 안 된 모듈이면 알린다', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old');
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'game' }] });
        render(
            <AddInstanceMigration
                extensions={[{ name: 'game', trust: 'unverified' }]}
                servers={[]}
                onAddServer={vi.fn()}
                onStepChange={vi.fn()}
            />,
        );
        await act(async () => fireEvent.click(screen.getByTitle('Browse')));
        await act(async () => fireEvent.click(await screen.findByRole('button', { name: /Next/i })));
        expect(screen.getByRole('note').textContent).toMatch(/game server .*is not isolated/);
        delete window.api.openFolderDialog;
        delete window.api.migrationScanDir;
    });
});
