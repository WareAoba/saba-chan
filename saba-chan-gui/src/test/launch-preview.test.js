import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEBOUNCE_MS, LaunchPreview, joinArgs, launchDraft } from '../components/servers/LaunchPreview';

const fields = [
    { name: 'port', field_type: 'number' },
    { name: 'ram', field_type: 'text' },
    { name: 'nogui', field_type: 'boolean' },
];
const settle = () =>
    act(async () => {
        await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
    });

describe('LaunchPreview', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        window.showToast = vi.fn();
    });
    afterEach(() => {
        vi.useRealTimers();
        delete window.api.instanceLaunchPreview;
        delete window.showToast;
    });

    it('저장 전 값을 저장 때처럼 바꾸고, 저장된 추가 인자는 따옴표를 살려 입력칸에 보인다', () => {
        expect(launchDraft(fields, { port: '25566', ram: '', nogui: 'true' })).toEqual({ port: 25566, nogui: true });
        expect(joinArgs(['-a', 'two words'])).toBe('-a "two words"');
    });

    it('실행 인자를 쓰지 않는 모듈이면 아무것도 띄우지 않는다', async () => {
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({ success: true, has_launch_args: false });
        const { container } = render(<LaunchPreview serverId="s1" fields={fields} values={{}} />);
        await settle();
        expect(container.innerHTML).toBe('');
    });

    it('값이 바뀌면 잠깐 기다렸다 한 번만 묻고, 추가 인자는 칸에 반영하고 남는 것만 저장용으로 올린다', async () => {
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({
            success: true,
            has_launch_args: true,
            command_line: 'java -Xmx4G -jar server.jar --port 25570 -fast',
            settings_from_args: { ram: '4G' },
            extra_args: ['-fast'],
            errors: [],
        });
        const onSettings = vi.fn();
        const onLeftover = vi.fn();
        const props = {
            serverId: 's1',
            fields,
            values: { port: '25570' },
            onExtraArgsChange: () => {},
            onSettingsFromArgs: onSettings,
            onLeftoverArgs: onLeftover,
        };
        const { rerender } = render(<LaunchPreview {...props} extraArgs="" />);
        rerender(<LaunchPreview {...props} extraArgs="-Xmx4G -fast" />);
        await settle();
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledTimes(1);
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledWith(
            's1',
            { port: 25570, nogui: false },
            '-Xmx4G -fast',
        );
        expect(screen.getByText('java -Xmx4G -jar server.jar --port 25570 -fast')).toBeTruthy();
        expect(onSettings).toHaveBeenCalledWith({ ram: '4G' });
        expect(onLeftover).toHaveBeenCalledWith(['-fast']);
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('형식이 틀리면 토스트 하나만 띄우고, 같은 오류로는 다시 띄우지 않는다', async () => {
        const bad = {
            success: true,
            has_launch_args: true,
            command_line: 'java -Xmx4096 -jar server.jar',
            settings_from_args: {},
            extra_args: [],
            errors: [{ arg: '-Xmx4096', field: 'ram', expected: 'memory_size', got: '4096' }],
        };
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue(bad);
        const props = { serverId: 's1', fields, values: {}, onExtraArgsChange: () => {} };
        const { rerender } = render(<LaunchPreview {...props} extraArgs="-Xmx4096" />);
        await settle();
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(window.showToast.mock.calls[0][0]).toBe('Invalid format.');
        rerender(<LaunchPreview {...props} extraArgs="-Xmx4096 " />);
        await settle();
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(screen.queryByText(/memory_size/)).toBeNull(); // 칸마다 형식 설명을 보이지 않는다
    });
});
