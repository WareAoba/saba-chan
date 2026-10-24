import { act, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DEBOUNCE_MS, LaunchPreview, joinArgs, launchDraft } from '../components/servers/LaunchPreview';
import { settleLaunchArgs } from '../hooks/useServerSettings';

const fields = [
    { name: 'port', field_type: 'number' },
    { name: 'ram', field_type: 'text', label: 'RAM' },
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

    it('값이 바뀌면 잠깐 기다렸다 한 번만 묻고, 추가 인자 중 설정에 맞는 것은 칸에 반영한다', async () => {
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({
            success: true,
            has_launch_args: true,
            command_line: 'java -Xmx4G -jar server.jar --port 25570 -fast',
            settings_from_args: { ram: '4G' },
            extra_args: ['-fast'],
            errors: [],
        });
        const onSettings = vi.fn();
        const props = {
            serverId: 's1',
            fields,
            values: { port: '25570' },
            onExtraArgsChange: () => {},
            onSettingsFromArgs: onSettings,
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
        expect(window.showToast).not.toHaveBeenCalled();
    });

    it('입력칸을 아직 모르면 저장된 추가 인자로 묻고, 돌려받은 글자로 칸을 채운다 (설정 칸은 건드리지 않는다)', async () => {
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({
            success: true,
            has_launch_args: true,
            command_line: 'java -Xmx4G -jar server.jar "two words"',
            settings_from_args: { ram: '4G' },
            extra_args: ['two words'],
            extra_args_text: '-Xmx4G "two words"',
            errors: [],
        });
        const onLoaded = vi.fn();
        const onSettings = vi.fn();
        render(
            <LaunchPreview
                serverId="s1"
                fields={fields}
                values={{}}
                onExtraArgsChange={() => {}}
                onExtraArgsLoaded={onLoaded}
                onSettingsFromArgs={onSettings}
            />,
        );
        await settle();
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledWith('s1', { nogui: false }, undefined);
        expect(onLoaded).toHaveBeenCalledWith('-Xmx4G "two words"');
        expect(onSettings).not.toHaveBeenCalled();
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
        // 어느 칸인지 이름표로 알린다 (모듈 번역이 없으면 module.toml의 label, 그것도 없으면 이름)
        expect(window.showToast.mock.calls[0][0]).toBe('Some values are not in the right format: RAM');
        rerender(<LaunchPreview {...props} extraArgs="-Xmx4096 " />);
        await settle();
        expect(window.showToast).toHaveBeenCalledTimes(1);
        expect(screen.queryByText(/memory_size/)).toBeNull(); // 칸마다 형식 설명을 보이지 않는다
    });

    it('저장 직전에 추가 인자를 다시 나눠, 방금 고친 인자도 빠지지 않는다', async () => {
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({
            success: true,
            has_launch_args: true,
            settings_from_args: { ram: '6G' },
            extra_args: ['-fast'],
        });
        const res = await settleLaunchArgs('s1', fields, { port: '1', _launch_extra_args: '-Xmx6G -fast' });
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledWith('s1', { port: 1, nogui: false }, '-Xmx6G -fast');
        expect(res).toEqual({ extra_args: ['-fast'], settings: { ram: '6G' } });
        window.api.instanceLaunchPreview = vi.fn().mockResolvedValue({ success: true, has_launch_args: false });
        expect(await settleLaunchArgs('s1', fields, {})).toBeNull();
        // 입력칸을 받기 전이면 데몬이 저장된 추가 인자로 나누게 undefined를 넘긴다
        expect(window.api.instanceLaunchPreview).toHaveBeenCalledWith('s1', { nogui: false }, undefined);
    });
});
