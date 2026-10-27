/**
 * 콘솔 시각 표시 (사용자 10-27): 끔 / 24시간 / 12시간, 날짜 함께. 데몬이 준 줄 시각(unix 초)을 이 PC 시간대로 보인다
 */
import { render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import { ServerConsole } from '../components/servers/ServerConsole';
import { useSettingsStore } from '../stores/useSettingsStore';

const at = new Date(2026, 9, 27, 15, 4, 5).getTime() / 1000;
const state = { lines: [{ id: 0, timestamp: at, content: 'Done (3.2s)!', level: 'info' }], input: '' };

const stampText = () => {
    render(
        <ServerConsole
            server={{ id: 'm', name: 'MC', status: 'running' }}
            state={state}
            openConsole={() => {}}
            setConsoleInput={() => {}}
            sendConsoleCommand={() => {}}
        />,
    );
    return document.querySelector('.console-time')?.textContent ?? null;
};

describe('콘솔 시각', () => {
    afterEach(() => useSettingsStore.getState()._resetForTest());

    it('기본은 시각을 붙이지 않는다', () => {
        expect(stampText()).toBeNull();
    });

    it('24시간제', () => {
        useSettingsStore.setState({ consoleTimestamps: '24h' });
        expect(stampText()).toContain('15:04:05');
    });

    it('12시간제에 날짜도', () => {
        useSettingsStore.setState({ consoleTimestamps: '12h', consoleTimestampDate: true });
        const text = stampText();
        expect(text).toMatch(/3:04:05/);
        expect(text).not.toContain('15:04');
        expect(text).toMatch(/10.*27|27.*10/);
    });
});
