/**
 * 사바 스토리지 모듈 탭의 "직접 만들기": 데몬이 알려 준 저장소로 에이전트별 설치 명령과 첫 부탁을 보여 준다.
 * GUI는 메이커를 실행하지 않고 데몬 GET /api/maker만 부른다.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import i18n from '../i18n';
import { ModuleMakerCard } from '../components/ModuleMakerCard';

const INFO = {
    ok: true,
    repo: 'https://github.com/WareAoba/saba-chan-modules',
    guide: 'https://saba-chan.online/guide/module-maker',
};

beforeEach(() => {
    window.api = { makerInfo: vi.fn().mockResolvedValue(INFO) };
    window.electron = { shell: { openExternal: vi.fn() } };
});

afterEach(() => {
    delete window.api;
    delete window.electron;
});

const lines = () => [...document.querySelectorAll('.cli-line code')].map((c) => c.textContent);

async function open() {
    render(<ModuleMakerCard />);
    expect(window.api.makerInfo).not.toHaveBeenCalled();
    await act(async () => fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:saba_storage.maker_open') })));
    await waitFor(() => expect(lines().length).toBeGreaterThan(0));
}

describe('모듈 메이커 카드', () => {
    it('열 때 데몬에 묻고 Claude Code는 데몬이 준 저장소를 받아 그 안에서 쓰게 한다', async () => {
        await open();
        expect(window.api.makerInfo).toHaveBeenCalledTimes(1);
        expect(lines()).toContain('git clone https://github.com/WareAoba/saba-chan-modules');
        expect(screen.getByText(i18n.t('gui:saba_storage.maker_claude_note'))).toBeTruthy();
        // 플러그인 마켓은 아직 열지 않았다
        expect(lines().some((l) => l.startsWith('/plugin'))).toBe(false);
    });

    it('에이전트를 바꾸면 그 에이전트의 설치 방법을 보여 준다', async () => {
        await open();
        await act(async () => fireEvent.click(screen.getByRole('tab', { name: 'Codex' })));
        expect(lines().some((l) => l.includes('WareAoba/saba-chan-modules') && l.includes('$skill-installer'))).toBe(true);
        await act(async () => fireEvent.click(screen.getByRole('tab', { name: i18n.t('gui:saba_storage.maker_other') })));
        expect(lines()).toContain('git clone https://github.com/WareAoba/saba-chan-modules');
    });

    it('게임 이름을 넣으면 첫 부탁에 들어간다', async () => {
        await open();
        await act(async () =>
            fireEvent.change(screen.getByLabelText(i18n.t('gui:saba_storage.maker_game_label')), {
                target: { value: 'Valheim' },
            }),
        );
        expect(lines()).toContain(i18n.t('gui:saba_storage.maker_prompt_text', { game: 'Valheim' }));
    });

    it('안내 링크는 데몬이 준 주소를 연다', async () => {
        await open();
        fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:saba_storage.maker_guide') }));
        expect(window.electron.shell.openExternal).toHaveBeenCalledWith(INFO.guide);
    });

    it('데몬이 답하지 못하면 이유를 알린다', async () => {
        window.api.makerInfo = vi.fn().mockResolvedValue({ error: 'daemon down' });
        render(<ModuleMakerCard />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: i18n.t('gui:saba_storage.maker_open') })));
        await waitFor(() => expect(screen.getByText(/daemon down/)).toBeTruthy());
        expect(lines()).toEqual([]);
    });
});
