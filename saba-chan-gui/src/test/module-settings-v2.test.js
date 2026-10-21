import { act, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { changedForCli } from '../components/CliLine';
import { SettingsField } from '../components/ServerSettingsModal';
import { settleLaunchArgs, useServerSettings } from '../hooks/useServerSettings';
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

    it('여러 줄 글은 줄바꿈을 받는 칸이다 (글자 수는 브라우저 maxLength가 아니라 검사가 데몬처럼 센다)', () => {
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
        // 브라우저 maxLength는 UTF-16 단위라 한도 2인 칸에 이모지 두 글자를 막는다
        expect(box.hasAttribute('maxlength')).toBe(false);
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

    // 데몬은 배열 항목을 그대로 둔다. 앞뒤 공백도 값이다 (공백만 있는 줄은 빈 줄로 본다)
    it('목록 값은 문자열 배열로 열고, 빈 줄만 뺀 배열로 저장한다', async () => {
        const { result } = setup(vi.fn());
        await act(async () => result.current.handleOpenSettings(server));
        expect(result.current.settingsValues.ops).toEqual(['alice']);
        act(() => result.current.handleSettingChange('ops', ['alice', '', '  ', ' bob ']));
        await act(async () => result.current.handleSaveSettings());
        expect(window.api.instanceUpdateSettings.mock.calls[0][1].ops).toEqual(['alice', ' bob ']);
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
        expect(onRestart).toHaveBeenCalledWith('S', { confirmed: true });
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

describe('CLI로 보기: list·multiline 값 (CLI server set 문법)', () => {
    const fields = [
        { name: 'admins', field_type: 'list' },
        { name: 'motd', field_type: 'multiline' },
    ];

    it('목록은 쉼표로 잇고, 항목에 쉼표가 있으면 JSON 배열로 쓴다. 빈 줄은 뺀다', () => {
        expect(changedForCli({ admins: ['Steve', '', 'Alex'] }, { admins: [] }, fields)).toEqual({
            admins: 'Steve,Alex',
        });
        expect(changedForCli({ admins: ['a,b', 'c'] }, {}, fields)).toEqual({ admins: '["a,b","c"]' });
        expect(changedForCli({ admins: [] }, { admins: ['x'] }, fields)).toEqual({ admins: '' });
    });

    it('앞뒤 공백이 있는 항목은 CLI가 쉼표 경로에서 다듬으므로 JSON 배열로 쓴다', () => {
        expect(changedForCli({ admins: [' a', 'b'] }, {}, fields)).toEqual({ admins: '[" a","b"]' });
    });

    it('여러 줄 글은 역슬래시를 두 번 쓴 뒤 줄바꿈을 \\n으로 쓴다', () => {
        expect(changedForCli({ motd: 'a\\b\nc' }, {}, fields)).toEqual({ motd: 'a\\\\b\\nc' });
    });
});

// 리뷰 559 P3와 Codex 지적: 데몬과 같은 기준으로 미리 막고, 값 모양을 잃지 않는다
describe('데몬과 같은 기준 (리뷰 후속)', () => {
    it('같은 항목이 두 번이면 칸에 오류를 단다 (데몬은 invalid_format으로 거절)', () => {
        const fields = [{ name: 'admins', field_type: 'list' }];
        const res = validateAllSettings(fields, { admins: ['a', 'b', 'a'] });
        expect(res.errors.admins).toMatchObject({ errorType: 'duplicate_item', value: 'a' });
    });

    it('여러 줄 글자 수는 데몬처럼 글자 단위로 센다', () => {
        const fields = [{ name: 'motd', field_type: 'multiline', max_length: 2 }];
        expect(validateAllSettings(fields, { motd: '😀😀' }).valid).toBe(true);
        expect(validateAllSettings(fields, { motd: '😀😀😀' }).valid).toBe(false);
    });

    it('숫자 비교는 십진수만 수로 본다 (Rust f64 파싱과 같게)', () => {
        const fields = [
            { name: 'port', field_type: 'number' },
            { name: 'note', field_type: 'text', visible_when: { field: 'port', equals: 25565 } },
        ];
        expect(hiddenFields(fields, { port: '0x63DD' })).toEqual(['note']);
        expect(hiddenFields(fields, { port: '25565.0' })).toEqual([]);
    });

    it('CLI로 보기: 쉼표 위치만 바뀐 목록도 바뀐 값이고, [로 시작하는 항목은 JSON 배열로 낸다', () => {
        const fields = [{ name: 'admins', field_type: 'list' }];
        expect(changedForCli({ admins: ['a', 'b'] }, { admins: ['a,b'] }, fields)).toEqual({ admins: 'a,b' });
        expect(changedForCli({ admins: ['[1]'] }, {}, fields)).toEqual({ admins: '["[1]"]' });
        expect(changedForCli({ admins: ['a', ''] }, { admins: ['a'] }, fields)).toEqual({});
    });

    it('선택지 글자를 눌러도 체크된다', () => {
        const onChange = vi.fn();
        render(
            <SettingsField
                field={{ name: 'perms', label: 'Perms', field_type: 'list', options: ['read', 'write'] }}
                value={[]}
                modNs="game"
                onChange={onChange}
            />,
        );
        fireEvent.click(screen.getByText('write'));
        expect(onChange).toHaveBeenCalledWith('perms', ['write']);
    });

    it('실행 인자에서 읽은 목록 값은 배열 그대로 둔다', async () => {
        window.api = {
            instanceLaunchPreview: vi.fn().mockResolvedValue({
                success: true,
                has_launch_args: true,
                settings_from_args: { admins: ['a,b', 'c'], ram: 6 },
                extra_args: [],
            }),
        };
        const res = await settleLaunchArgs('s1', [], {});
        expect(res.settings).toEqual({ admins: ['a,b', 'c'], ram: '6' });
    });
});

// 데몬 #498 b55e95e3: visible_when이 공통 칸(INSTANCE_FIELDS)을 가리키면 인스턴스 칸의 지금 값으로 숨김을 정한다
describe('공통 칸을 가리키는 visible_when', () => {
    it.each([
        ['memory_limit_mb', 'number', 2048, 1024],
        ['working_dir', 'text', 'D:/new', 'C:/old'],
    ])('%s는 module_settings에 남은 옛 값이 아니라 인스턴스 칸 값을 쓴다', async (name, field_type, now, stale) => {
        const fields = [
            { name, field_type },
            { name: 'note', field_type: 'text', visible_when: { field: name, equals: now } },
        ];
        const server = {
            id: 's',
            name: 'S',
            module: 'game',
            status: 'stopped',
            [name]: now,
            module_settings: { [name]: stale },
        };
        window.api = {
            serverList: vi.fn().mockResolvedValue(null),
            instanceReadLiveSettings: vi.fn().mockResolvedValue({ success: true, settings: {} }),
        };
        const { result } = renderHook(() =>
            useServerSettings({
                servers: [server],
                modules: [{ name: 'game', settings: { fields } }],
                setModal: vi.fn(),
                setProgressBar: vi.fn(),
                fetchServers: vi.fn(),
            }),
        );
        await act(async () => result.current.handleOpenSettings(server));
        expect(hiddenFields(fields, result.current.settingsValues)).toEqual([]);
    });
});
