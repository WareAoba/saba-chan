import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { CommandOutput } from '../components/CommandsPanel';
import { initI18n } from '../i18n';

// 값으로 답하는 조회(지표·설정 보기·플레이어 목록)는 데몬이 정리한 결과(output)를 앱과 같이 보인다
beforeAll(() => initI18n('en'));
afterEach(cleanup);

const field = (key, label, value) => ({ key, label, value });

describe('명령 결과 표시', () => {
    it('이름: 값과 8줄 넘는 결과의 더 보기', () => {
        const many = Array.from({ length: 10 }, (_, i) => field(`k${i}`, `Key ${i}`, `v${i}`));
        render(<CommandOutput output={{ fields: many }} />);
        expect(screen.getByText('Key 0')).toBeTruthy();
        expect(screen.getByText('v7')).toBeTruthy();
        expect(screen.queryByText('Key 8')).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: /2 more/ }));
        expect(screen.getByText('Key 9')).toBeTruthy();
    });

    it('목록 결과는 표로', () => {
        const table = { key: 'players', label: 'Players', columns: [{ key: 'name', label: 'Name' }], rows: [['Kim']] };
        render(<CommandOutput output={{ table }} />);
        expect(screen.getByText('Players (1)')).toBeTruthy();
        expect(screen.getByRole('cell', { name: 'Kim' })).toBeTruthy();
    });
});
