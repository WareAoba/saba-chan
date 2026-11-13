/**
 * 탭에 들어갈 그림은 미리 받아 풀어 둔다 (사용자 11-13: 디스코드 탭을 열 때 그림이 늦게 뜨지 않게).
 * 숨겨 미리 그려 둔 탭(App KeptPage)에서 지연 로딩(lazy)이면 탭을 열 때에야 받기 시작하고,
 * 비동기 해독(async)이면 그림 없는 프레임이 먼저 보인다.
 */
import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Avatar } from '../components/discord/Avatar';
import { SabaArt } from '../components/ui/SabaArt';

describe('탭 그림', () => {
    it('디스코드 서버 아이콘은 숨은 탭에서도 바로 받는다', () => {
        const { container } = render(<Avatar name="g" url="https://cdn.discordapp.com/icons/1/a.png" />);
        expect(container.querySelector('img')?.getAttribute('loading')).not.toBe('lazy');
    });

    it('사바쨩 그림은 그림 없는 프레임 없이 그린다', () => {
        const { container } = render(<SabaArt kind="relay" />);
        expect(container.querySelector('img')?.getAttribute('decoding')).toBe('sync');
    });
});
