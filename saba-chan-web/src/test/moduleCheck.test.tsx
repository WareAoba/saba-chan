import { act, render, screen } from '@testing-library/react';
import { beforeAll, describe, expect, it, vi } from 'vitest';
import { ModulesTab } from '../components/ModulesTab';
import { initI18n } from '../i18n';
import { useData } from '../store';
import { mockApi } from './mocks';

const flush = () => act(async () => {});

// 문제 없는 모듈에 "검사 결과: 선언한 기능만 써요"를 늘 보이는 건 쓸모없다 (앱처럼 경고만)
describe('모듈 검사 결과', () => {
    beforeAll(() => initI18n('en'));

    it('선언하지 않은 기능을 쓰는 모듈만 누르지 않아도 경고를 보이고, 문제 없는 모듈은 아무것도 보이지 않는다', async () => {
        useData.setState({
            modules: [
                { name: 'clean', version: '1.0.0' },
                { name: 'risky', version: '1.0.0' },
            ] as never,
        });
        const api = new Proxy(
            {
                moduleManifest: vi.fn().mockResolvedValue({}),
                moduleAudit: vi.fn(async (name: string) => ({
                    undeclared: name === 'risky' ? [{ file: 'a.py', line: 1, permission: 'network' }] : [],
                })),
            },
            { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
        );
        render(<ModulesTab api={mockApi(api)} />);
        await flush();
        expect(screen.queryByText(/uses only declared/i)).toBeNull();
        expect(document.querySelectorAll('.hint-warn')).toHaveLength(1);
    });

    it('탭을 열 때 모듈 목록을 다시 읽어 그사이 생긴 업데이트를 보인다', async () => {
        useData.setState({ modules: [{ name: 'game', version: '1.0.0' }] as never });
        const api = new Proxy(
            {
                moduleManifest: vi.fn().mockResolvedValue({}),
                moduleList: vi.fn().mockResolvedValue({ modules: [{ name: 'game', version: '1.0.0', update_version: '1.1.0' }] }),
            },
            { get: (t, k) => t[k as keyof typeof t] ?? vi.fn().mockResolvedValue({}) },
        );
        render(<ModulesTab api={mockApi(api)} />);
        await flush();
        expect(useData.getState().modules[0].update_version).toBe('1.1.0');
    });
});
