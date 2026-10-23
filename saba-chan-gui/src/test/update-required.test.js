/**
 * 사바쨩이 낮아서 거절된 경우(릴레이, 모듈, 익스텐션)는 일반 실패 대신 "업데이트가 필요해요"로 보인다.
 * 테스트 언어는 영어라 "Update required"를 본다.
 */
import { act, renderHook } from '@testing-library/react';
import i18n from 'i18next';
import { describe, expect, it, vi } from 'vitest';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';
import { failedModuleReason } from '../utils/failedModules';
import { updateRequiredText } from '../utils/updateRequired';

const t = i18n.t.bind(i18n);

describe('업데이트가 필요해요', () => {
    it('릴레이가 이 앱 버전을 받지 않으면 최소 버전과 함께 알린다', () => {
        const text = updateRequiredText(t, { lastErrorCode: 'agent_too_old', minVersion: '0.1.10' });
        expect(text).toMatch(/^Update required/);
        expect(text).toContain('0.1.10');
    });

    it('모듈이 더 새 saba-core를 요구하면 그 버전을 알린다', () => {
        const body = {
            error_code: 'module_dependency_unsatisfied',
            missing_dependencies: ['saba-core >=0.1.10 (installed 0.1.9)'],
        };
        const text = updateRequiredText(t, body, 'palworld');
        expect(text).toMatch(/^Update required/);
        expect(text).toContain('palworld');
        expect(text).toContain('0.1.10');
        expect(text).not.toContain('>=');
    });

    it('모듈이 익스텐션만 요구하면 업데이트 안내가 아니다', () => {
        const body = {
            error_code: 'module_dependency_unsatisfied',
            missing_dependencies: ['ext-steamcmd >=1.0.0 (not installed)'],
        };
        expect(updateRequiredText(t, body, 'palworld')).toBeNull();
    });

    it('데몬보다 새 모듈 형식이면 버전 없이 알린다', () => {
        const text = updateRequiredText(t, { error_code: 'module_schema_unsupported', required: 3 }, 'ark');
        expect(text).toMatch(/^Update required/);
        expect(text).toContain('ark');
    });

    it('익스텐션이 더 새 앱을 요구하면(min_app_version 또는 saba-core 의존성) 그 버전을 알린다', () => {
        const app = updateRequiredText(t, { error_code: 'app_version_unsatisfied', related: ['0.1.10'] }, 'steamcmd');
        expect(app).toMatch(/^Update required/);
        expect(app).toContain('steamcmd');
        expect(app).toContain('0.1.10');
        const core = updateRequiredText(
            t,
            { error_code: 'component_version_unsatisfied', related: ['saba-core', '>=0.1.11'] },
            'steamcmd',
        );
        expect(core).toContain('0.1.11');
        expect(core).not.toContain('>=');
        // 다른 컴포넌트(gui 등) 요구는 이 앱 업데이트 안내가 아니다
        expect(
            updateRequiredText(t, { error_code: 'component_version_unsatisfied', related: ['gui', '>=1.0.0'] }, 'x'),
        ).toBeNull();
    });

    it('버전과 상관없는 실패는 null', () => {
        expect(updateRequiredText(t, { error_code: 'not_found' }, 'x')).toBeNull();
        expect(updateRequiredText(t, null)).toBeNull();
    });

    it('불러오지 못한 모듈 목록의 이유도 같은 문구를 쓴다', () => {
        const failure = {
            path: '/m/palworld',
            error: 'x',
            error_code: 'module_dependency_unsatisfied',
            missing_dependencies: ['saba-core >=0.1.10 (installed 0.1.9)'],
        };
        expect(failedModuleReason(failure, t)).toMatch(/^Update required/);
        expect(failedModuleReason({ path: '/m/ark', error_code: 'module_schema_unsupported', required: 3 }, t)).toMatch(
            /^Update required/,
        );
    });

    it('익스텐션을 켜거나 받을 때 더 새 앱이 필요하면 업데이트 안내를 띄운다', async () => {
        window.showToast = vi.fn();
        const refused = { success: false, error: 'raw', error_code: 'app_version_unsatisfied', related: ['0.1.10'] };
        window.api = {
            extensionList: vi.fn().mockResolvedValue({ extensions: [] }),
            extensionEnable: vi.fn().mockResolvedValue(refused),
            extensionInstall: vi.fn().mockResolvedValue(refused),
        };
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await act(async () => {
            await result.current.toggleExtension('steamcmd', true);
            await result.current.installExtension('steamcmd');
        });
        const errors = window.showToast.mock.calls.filter((c) => c[1] === 'error').map((c) => c[0]);
        expect(errors).toHaveLength(2);
        for (const text of errors) {
            expect(text).toMatch(/^Update required/);
            expect(text).toContain('0.1.10');
            expect(text).not.toContain('raw');
        }
    });
});
