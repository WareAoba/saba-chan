import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { SettingsField } from '../components/ServerSettingsModal';
import { useServerSettings } from '../hooks/useServerSettings';
import { failedModuleReason } from '../utils/failedModules';
import { hiddenFields, validateAllSettings } from '../utils/validation';

// 모듈 설정 스키마 2 (데몬 #498, 02 §1.2): list·multiline 칸, visible_when 숨김, restart_required 안내, 로드 실패 이유

describe('visible_when 숨김 (데몬 validator::hidden_fields와 같은 계산)', () => {
    const fields = [
        { name: 'pvp', field_type: 'boolean', default: false },
        { name: 'port', field_type: 'number', default: 25565 },
        { name: 'mode', field_type: 'select', options: ['a', 'b', 'c'] },
        { name: 'pvp_rules', field_type: 'text', visible_when: { field: 'pvp', equals: true } },
        { name: 'port_note', field_type: 'text', visible_when: { field: 'port', equals: 25565 } },
        { name: 'mode_note', field_type: 'text', visible_when: { field: 'mode', in: ['a', 'b'] } },
    ];

    it('값은 그 설정 타입으로 바꿔 비교하고, 값이 없으면 기본값으로, 기본값도 없으면 숨긴다', () => {
        expect(hiddenFields(fields, {})).toEqual(['pvp_rules', 'mode_note']);
        expect(hiddenFields(fields, { pvp: 'true', port: '25565', mode: 'b' })).toEqual([]);
        expect(hiddenFields(fields, { pvp: true, port: ' 25565 ', mode: 'c' })).toEqual(['mode_note']);
        expect(hiddenFields(fields, { pvp: 'false', port: 25566, mode: 'a' })).toEqual(['pvp_rules', 'port_note']);
    });

    it('숨긴 설정은 필수여도 검사하지 않는다', () => {
        const required = [
            { name: 'pvp', field_type: 'boolean', default: false },
            { name: 'pvp_rules', field_type: 'text', required: true, visible_when: { field: 'pvp', equals: true } },
        ];
        expect(validateAllSettings(required, { pvp: 'false' }).valid).toBe(true);
        expect(validateAllSettings(required, { pvp: 'true' }).errors.pvp_rules.errorType).toBe('required');
    });
});

describe('list·multiline 값 검사', () => {
    it('목록은 빈 줄을 빼고 항목 수(min_items·max_items)와 선택지를 본다', () => {
        const field = { name: 'ops', field_type: 'list', min_items: 1, max_items: 2, options: ['x', 'y', 'z'] };
        expect(validateAllSettings([field], { ops: ['x', ''] }).valid).toBe(true);
        expect(validateAllSettings([field], { ops: [] }).errors.ops.errorType).toBe('item_count');
        expect(validateAllSettings([field], { ops: ['x', 'y', 'z'] }).errors.ops.errorType).toBe('item_count');
        expect(validateAllSettings([field], { ops: ['q'] }).errors.ops.errorType).toBe('invalid_option');
    });

    it('여러 줄 글은 max_length 글자까지다', () => {
        const field = { name: 'motd', field_type: 'multiline', max_length: 5 };
        expect(validateAllSettings([field], { motd: 'ab\ncd' }).valid).toBe(true);
        expect(validateAllSettings([field], { motd: 'abc\ndef' }).errors.motd.errorType).toBe('too_long');
    });
});

describe('list·multiline 칸', () => {
    it('선택지가 있는 목록은 고르는 칸이고, 고른 것을 선택지 순서의 배열로 넘긴다', () => {
        const onChange = vi.fn();
        render(
            <SettingsField
                field={{ name: 'mods', label: 'Mods', field_type: 'list', options: ['a', 'b', 'c'] }}
                value={['c']}
                modNs="game"
                onChange={onChange}
            />,
        );
        fireEvent.click(screen.getByRole('checkbox', { name: 'a' }));
        expect(onChange).toHaveBeenCalledWith('mods', ['a', 'c']);
        fireEvent.click(screen.getByRole('checkbox', { name: 'c' }));
        expect(onChange).toHaveBeenLastCalledWith('mods', []);
    });

    it('선택지가 없는 목록은 한 줄에 하나씩 쓰는 칸이다', () => {
        const onChange = vi.fn();
        render(
            <SettingsField
                field={{ name: 'ops', label: 'Operators', field_type: 'list' }}
                value={['alice', 'bob']}
                modNs="game"
                onChange={onChange}
            />,
        );
        const box = screen.getByLabelText(/Operators/);
        expect(box.value).toBe('alice\nbob');
        fireEvent.change(box, { target: { value: 'alice\nbob\n' } });
        expect(onChange).toHaveBeenCalledWith('ops', ['alice', 'bob', '']);
    });

    it('여러 줄 글은 줄바꿈을 받는 칸이고 max_length를 넘겨 쓸 수 없다', () => {
        render(
            <SettingsField
                field={{ name: 'motd', label: 'Message', field_type: 'multiline', max_length: 80 }}
                value={'a\nb'}
                modNs="game"
                onChange={vi.fn()}
            />,
        );
        const box = screen.getByLabelText(/Message/);
        expect(box.tagName).toBe('TEXTAREA');
        expect(box.value).toBe('a\nb');
        expect(box.maxLength).toBe(80);
    });
});

describe('설정 저장 (list 값, restart_required)', () => {
    const modules = [
        {
            name: 'game',
            settings: {
                fields: [
                    { name: 'ops', field_type: 'list' },
                    { name: 'motd', field_type: 'multiline', restart_required: true },
                ],
            },
        },
    ];
    const server = { id: 's', name: 'S', module: 'game', status: 'running', module_settings: { ops: ['alice'] } };

    function setup(onRestart) {
        const setModal = vi.fn();
        const hook = renderHook(() =>
            useServerSettings({
                servers: [server],
                modules,
                setModal,
                setProgressBar: vi.fn(),
                fetchServers: vi.fn(),
                onRestart,
            }),
        );
        return { ...hook, setModal };
    }

    beforeEach(() => {
        window.showToast = vi.fn();
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: { ops: ['alice'] } }),
            instanceUpdateSettings: vi.fn().mockResolvedValue({ success: true, synced: true, restart_required: [] }),
        };
    });

    it('목록 값은 문자열 배열로 열고, 빈 줄을 뺀 배열로 저장한다', async () => {
        const { result } = setup(vi.fn());
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues.ops).toEqual(['alice']);
        act(() => result.current.handleSettingChange('ops', ['alice', '', 'bob ']));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1].ops).toEqual(['alice', 'bob']);
    });

    it('다시 켜야 적용되는 설정을 바꿨으면 그렇다고 알리고 바로 다시 켤 수 있게 한다', async () => {
        window.api.instanceUpdateSettings.mockResolvedValue({
            success: true,
            synced: true,
            restart_required: ['motd'],
        });
        const onRestart = vi.fn();
        const { result, setModal } = setup(onRestart);
        await act(async () => result.current.handleOpenSettings(server));
        act(() => result.current.handleSettingChange('motd', 'hi'));
        await act(async () => result.current.handleSaveSettings());
        const modal = setModal.mock.calls.at(-1)[0];
        expect(modal.type).toBe('question');
        expect(modal.message).toMatch(/restart/i);
        expect(modal.message).toContain('motd');
        await act(async () => modal.buttons[0].action());
        expect(onRestart).toHaveBeenCalledWith('S');
    });

    it('restart_required가 비어 있으면 평소처럼 저장 완료다', async () => {
        const { result, setModal } = setup(vi.fn());
        await act(async () => result.current.handleOpenSettings(server));
        await act(async () => result.current.handleSaveSettings());
        expect(setModal.mock.calls.at(-1)[0].type).toBe('success');
    });
});

describe('모듈 로드 실패 이유 (MK2)', () => {
    const t = (key, vars) => `${key} ${JSON.stringify(vars)}`;

    it('스키마에 맞지 않으면 문제 목록을 보인다', () => {
        const f = {
            path: 'modules/game',
            error: 'raw',
            error_code: 'module_schema_invalid',
            problems: ['settings.mode: unknown type', 'settings.x: bad'],
        };
        const text = failedModuleReason(f, t);
        expect(text).toContain('modules.schema_invalid');
        expect(text).toContain('settings.mode: unknown type');
        expect(text).not.toContain('raw');
    });

    it('데몬보다 새 스키마면 필요한 스키마 버전과 업데이트 안내를 보인다', () => {
        const f = { path: 'modules/game', error: 'raw', error_code: 'module_schema_unsupported', required: 3 };
        const text = failedModuleReason(f, t);
        expect(text).toContain('modules.schema_unsupported');
        expect(text).toContain('3');
    });
});
