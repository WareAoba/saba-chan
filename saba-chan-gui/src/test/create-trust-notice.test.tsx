import { act, fireEvent, render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { AddInstanceMigration as AddInstanceMigrationImpl } from '../components/Modals/AddInstanceMigration';
import { AddInstanceNewServer as AddInstanceNewServerImpl } from '../components/Modals/AddInstanceNewServer';
import { toConfirmStep } from './addServerWizard';
import type { ComponentType } from 'react';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const AddInstanceMigration = AddInstanceMigrationImpl as unknown as ComponentType<Record<string, unknown>>;

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const AddInstanceNewServer = AddInstanceNewServerImpl as unknown as ComponentType<Record<string, unknown>>;

const renderNew = async (trust: string) => {
    render(
        <AddInstanceNewServer
            extensions={[{ name: 'game', version: '1.0.0', trust }]}
            servers={[]}
            onAddServer={vi.fn()}
        />,
    );
    fireEvent.click(screen.getByRole('button', { name: /game/ }));
    await toConfirmStep();
};

describe('서버 만들 때 신뢰 안내', () => {
    it('확인 안 된 모듈이면 게임 서버는 격리되지 않는다고 알린다', async () => {
        await renderNew('unverified');
        const notice = screen.getByRole('note');
        expect(notice.textContent).toMatch(/not from the official catalog/);
        expect(notice.textContent).toMatch(/game server .*is not isolated/);
    });

    it('설치 뒤 바뀐 모듈이면 바뀌었다고 알린다', async () => {
        await renderNew('modified');
        expect(screen.getByRole('note').textContent).toMatch(/changed after installing/);
    });

    it('확인된 모듈이나 trust가 없는 옛 데몬이면 알리지 않는다', async () => {
        await renderNew('verified');
        expect(screen.queryByRole('note')).toBeNull();
    });

    it('기존 서버 가져오기에서도 확인 안 된 모듈이면 알린다', async () => {
        window.api.openFolderDialog = vi.fn().mockResolvedValue('/srv/old') as never;
        window.api.migrationScanDir = vi.fn().mockResolvedValue({ ok: true, matches: [{ name: 'game' }] }) as never;
        render(
            <AddInstanceMigration
                extensions={[{ name: 'game', trust: 'unverified' }]}
                servers={[]}
                onAddServer={vi.fn()}
                onStepChange={vi.fn()}
            />,
        );
        await act(async () => fireEvent.click(screen.getByRole('button', { name: 'Browse' })));
        await act(async () => fireEvent.click(await screen.findByRole('button', { name: /Next/i })));
        expect(screen.getByRole('note').textContent).toMatch(/game server .*is not isolated/);
        delete (window.api as Partial<typeof window.api>).openFolderDialog;
        delete (window.api as Partial<typeof window.api>).migrationScanDir;
    });
});
