import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { InstanceWords } from '../components/InstanceWords';
import { initI18n } from '../i18n';
import { mockApi } from './mocks';

const flush = () => act(async () => {});
const ITEM = {
    id: 's1',
    name: 'Mc',
    defaultWords: ['Mc'],
    words: ['craft'],
    commands: [{ name: 'start', label: 'Start', risk: 'safe', moduleAliases: [], aliases: ['on'] }],
};

describe('디스코드 호출어 (서버 상세 › 사바쨩)', () => {
    beforeAll(() => initI18n('en'));

    it('낱말을 더하고 빼서 저장하면 GUI 와 같은 모양으로 보낸다', async () => {
        const api = {
            instanceWords: vi.fn().mockResolvedValue(ITEM),
            instanceWordsSave: vi.fn(async (_id, body) => ({ success: true, ...body })),
        };
        render(<InstanceWords api={mockApi(api)} server={{ id: 's1' }} />);
        await flush();
        const input = screen.getByRole('textbox', { name: /Mc/ });
        fireEvent.change(input, { target: { value: 'mine' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        fireEvent.click(screen.getByRole('button', { name: 'Remove on' }));
        fireEvent.click(screen.getByRole('button', { name: 'Save words' }));
        await flush();
        expect(api.instanceWordsSave).toHaveBeenCalledWith('s1', { words: ['craft', 'mine'], commands: { start: [] } });
        // 저장하면 저장 버튼이 사라진다 (고친 것이 없다)
        expect(screen.queryByRole('button', { name: 'Save words' })).toBeNull();
    });

    it('데몬이 겹친다고 하면 그 칸 아래에 알린다', async () => {
        const api = {
            instanceWords: vi.fn().mockResolvedValue(ITEM),
            instanceWordsSave: vi.fn().mockResolvedValue({
                success: false,
                error_code: 'validation_failed',
                details: [{ field: 'words[1]', word: 'pal', conflictWith: 'Palworld' }],
            }),
        };
        render(<InstanceWords api={mockApi(api)} server={{ id: 's1' }} />);
        await flush();
        const input = screen.getByRole('textbox', { name: /Mc/ });
        fireEvent.change(input, { target: { value: 'pal' } });
        fireEvent.keyDown(input, { key: 'Enter' });
        fireEvent.click(screen.getByRole('button', { name: 'Save words' }));
        await flush();
        expect(screen.getByText(/Palworld/)).toBeTruthy();
    });
});
