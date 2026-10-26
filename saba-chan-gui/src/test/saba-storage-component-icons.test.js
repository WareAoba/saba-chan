/**
 * 사바 스토리지 컴포넌트 목록: 컴포넌트마다 자기 그림 (사용자 10-26, A 세트 + 업데이터·GUI는 D 그림).
 * 코어와 인스톨러가 같은 서버 그림을 쓰지 않는다.
 */
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Icon } from '../components/Icon';
import { parseComponents } from '../components/Modals/SabaStorage';

const KEYS = ['updater', 'gui', 'saba-core', 'cli', 'installer'];

describe('컴포넌트 아이콘', () => {
    it('다섯 컴포넌트가 서로 다른 자기 아이콘을 쓴다', () => {
        const icons = parseComponents(KEYS.map((component) => ({ component }))).map((c) => c.icon);
        expect(icons).toEqual([
            'componentUpdater',
            'componentGui',
            'componentCore',
            'componentCli',
            'componentInstaller',
        ]);
    });

    it('아이콘이 실제로 그려진다', () => {
        for (const name of ['componentUpdater', 'componentGui', 'componentCore', 'componentCli', 'componentInstaller']) {
            const { container, unmount } = render(<Icon name={name} />);
            expect(container.querySelector('svg'), name).not.toBeNull();
            unmount();
        }
    });
});
