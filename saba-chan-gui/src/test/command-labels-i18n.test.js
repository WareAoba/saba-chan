import { fireEvent, render, screen } from '@testing-library/react';
import i18n from 'i18next';
import { beforeAll, describe, expect, it } from 'vitest';
import CommandModal from '../components/Modals/CommandModal';
import { commandText } from '../utils/serverView';

// PC 실측: 명령 탭이 모듈 locales의 '정지' 대신 명령 이름(stop)을 보여 줬다
const server = { id: 'l1', name: 'L', module: 'mcl', status: 'running' };
const modules = [
    { name: 'mcl', commands: { fields: [{ name: 'stop', label: 'Stop', description: 'Saves and stops' }] } },
];

beforeAll(() => {
    i18n.addResourceBundle(
        'en',
        'mod_mcl',
        { commands: { stop: { label: 'Halt', description: 'Saves the world and halts' } } },
        true,
        true,
    );
});

describe('명령 이름 번역', () => {
    it('모듈 locales의 명령 문구를 쓰고, 없으면 모듈이 준 문구를 쓴다', () => {
        const t = i18n.getFixedT('en', 'gui');
        expect(commandText(t, 'mcl', modules[0].commands.fields[0])).toEqual({
            label: 'Halt',
            description: 'Saves the world and halts',
        });
        expect(commandText(t, 'mcl', { name: 'say', label: 'Say', description: 'd' })).toEqual({
            label: 'Say',
            description: 'd',
        });
        expect(commandText(t, 'mcl', { name: 'list' })).toEqual({ label: 'list', description: '' });
    });

    it('명령 탭 목록이 번역한 이름을 보여 주고, 고르면 명령어를 채운다', () => {
        render(<CommandModal server={server} modules={modules} />);
        const option = screen.getByRole('option');
        expect(option.textContent).toContain('Halt');
        expect(option.textContent).toContain('Saves the world and halts');
        fireEvent.click(option);
        expect(screen.getByRole('textbox').value).toBe('stop ');
    });
});
