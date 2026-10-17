import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEBOUNCE_MS, LaunchPreview, launchDraft, launchPreviewAvailable } from '../components/servers/LaunchPreview';

const fields = [
    { name: 'port', field_type: 'number', launch_arg: true },
    { name: 'motd', field_type: 'text' },
    { name: 'nogui', field_type: 'boolean', launch_arg: true },
];

describe('LaunchPreview', () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => {
        vi.useRealTimers();
        delete window.api.instanceLaunchPreview;
    });

    it('실행 인자 필드가 있을 때만 쓰고, 그 필드의 저장 전 값만 보낸다', () => {
        window.api.instanceLaunchPreview = vi.fn();
        expect(launchPreviewAvailable(fields)).toBe(true);
        expect(launchPreviewAvailable([{ name: 'motd' }])).toBe(false);
        expect(launchDraft(fields, { port: '25566', motd: 'hi', nogui: 'true' })).toEqual({ port: 25566, nogui: true });
    });

    it('값이 바뀌면 잠깐 기다렸다 한 번만 묻고, 못 구하면 이유를 보인다', async () => {
        window.api.instanceLaunchPreview = vi
            .fn()
            .mockResolvedValueOnce({ success: true, command_line: 'java -jar server.jar --port 25566 nogui' })
            .mockResolvedValueOnce({
                success: false,
                action_required: 'server_jar_not_found',
                message: 'No server jar',
            });
        const { rerender } = render(<LaunchPreview serverId="s1" fields={fields} values={{ port: '25565' }} />);
        rerender(<LaunchPreview serverId="s1" fields={fields} values={{ port: '25566', nogui: true }} />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
        });
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledTimes(1);
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledWith('s1', {
            settings: { port: 25566, nogui: true },
            extra_args: '',
        });
        expect(screen.getByText('java -jar server.jar --port 25566 nogui')).toBeTruthy();

        // 실행 인자가 아닌 필드만 바뀌면 다시 묻지 않는다
        rerender(<LaunchPreview serverId="s1" fields={fields} values={{ port: '25566', nogui: true, motd: 'x' }} />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
        });
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledTimes(1);

        rerender(<LaunchPreview serverId="s1" fields={fields} values={{ port: '1', nogui: true }} />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
        });
        expect(screen.getByText(/Needs attention before launch: No server jar/)).toBeTruthy();
    });

    it('추가 인자는 맞는 필드에 반영하고, 타입 오류와 남는 인자를 보인다', async () => {
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({
            success: true,
            command_line: 'srv --port abc -fast',
            fields_from_args: { port: 25570 },
            unknown_args: ['-fast'],
            errors: [{ arg: '--port', field: 'port', expected: 'number', got: 'abc' }],
        });
        const onFields = vi.fn();
        const props = { serverId: 's1', fields, values: {}, onExtraArgsChange: () => {}, onFieldsFromArgs: onFields };
        const { rerender } = render(<LaunchPreview {...props} extraArgs="" />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
        });
        expect(onFields).not.toHaveBeenCalled(); // 추가 인자가 그대로면 필드를 건드리지 않는다
        rerender(<LaunchPreview {...props} extraArgs="--port abc -fast" />);
        await act(async () => {
            await vi.advanceTimersByTimeAsync(DEBOUNCE_MS);
        });
        expect(onFields).toHaveBeenCalledWith({ port: 25570 });
        expect(screen.getByText(/--port: port expects a number \(got abc\)/)).toBeTruthy();
        expect(screen.getByText(/passed as is: -fast/)).toBeTruthy();
    });
});
