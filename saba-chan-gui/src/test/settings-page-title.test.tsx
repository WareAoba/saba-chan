import fs from 'node:fs';
import path from 'node:path';
import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps, ComponentType } from 'react';
import { describe, expect, it } from 'vitest';
import SettingsModalImpl from '../components/Modals/SettingsModal';

const SettingsModal = SettingsModalImpl as ComponentType<Partial<ComponentProps<typeof SettingsModalImpl>>>;
const SRC = path.resolve(__dirname, '..');

// 설정 화면 제목은 고른 탭 이름 하나 (사용자 결정 2026-11-11: "설정" 아래 탭 이름이 또 나오던 이중 제목 정리)
describe('설정 화면 제목과 줄 아이콘', () => {
    it('큰 제목은 고른 탭 이름 하나만 있다', async () => {
        const { container } = render(<SettingsModal />);
        await act(async () => fireEvent.click(container.querySelector('#settings-tab-appearance') as HTMLElement));
        expect(screen.getByRole('heading', { level: 2, name: 'Appearance' })).toBeInTheDocument();
        expect(screen.queryByRole('heading', { name: 'Settings' })).toBeNull();
    });

    it('설정 줄 제목(.setting-title)에는 아이콘을 두지 않는다. 카드 제목 아이콘만 남긴다', () => {
        const files = [
            ...fs.readdirSync(path.join(SRC, 'components/Modals/settings')).map((f) => `components/Modals/settings/${f}`),
            'components/Modals/AutostartSetting.tsx',
            'components/Modals/NetTraceSetting.tsx',
            'components/Modals/WindowBackdropSetting.tsx',
            'components/Modals/RemoteDaemonSection.tsx',
        ].filter((f) => f.endsWith('.tsx'));
        const hits: string[] = [];
        for (const f of files) {
            const lines = fs.readFileSync(path.join(SRC, f), 'utf8').split('\n');
            lines.forEach((line, i) => {
                if (line.includes('className="setting-title"') && /<Icon\b/.test(`${line}\n${lines[i + 1] ?? ''}`)) {
                    hits.push(`${f}:${i + 1}`);
                }
            });
        }
        expect(hits).toEqual([]);
    });
});
