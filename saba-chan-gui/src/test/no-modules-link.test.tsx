import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentType } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { AddInstanceModal as AddInstanceModalImpl } from '../components/Modals/AddInstanceModal';
import { ServersPage as ServersPageImpl } from '../components/servers/ServersPage';
import '../i18n';
import { useUIStore } from '../stores/useUIStore';

/** 시험에 필요한 props만 넘기려고 느슨하게 받는다 */
const ServersPage = ServersPageImpl as unknown as ComponentType<Record<string, unknown>>;
const AddInstanceModal = AddInstanceModalImpl as unknown as ComponentType<Record<string, unknown>>;

// 0.1.13 감사 #3: 서버도 모듈도 없는 첫 실행이 막다른 길이었다. 모듈을 받는 사바 스토리지(설정 > 저장 공간)로 가는 링크를 둔다.
describe('모듈이 하나도 없을 때', () => {
    beforeEach(() => useUIStore.getState()._resetForTest());

    it('빈 서버 화면에서 저장 공간 설정으로 간다', () => {
        render(<ServersPage servers={[]} modules={[]} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />);
        fireEvent.click(screen.getByRole('button', { name: /Storage/ }));
        expect(useUIStore.getState().settingsInitialView).toBe('storage');
    });

    it('모듈이 있으면 링크가 없다', () => {
        render(
            <ServersPage servers={[]} modules={[{ name: 'g' }]} onAddServer={vi.fn()} drag={{ cardRefs: { current: {} } }} />,
        );
        expect(screen.queryByRole('button', { name: /Storage/ })).toBeNull();
    });

    it('서버 추가 창의 비활성 새 서버 카드 아래에서 저장 공간 설정으로 간다', () => {
        const onClose = vi.fn();
        render(<AddInstanceModal isOpen onClose={onClose} extensions={[]} servers={[]} onAddServer={vi.fn()} />);
        fireEvent.click(screen.getByRole('button', { name: /Storage/ }));
        expect(useUIStore.getState().settingsInitialView).toBe('storage');
    });
});
