import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { Icon } from '../components/Icon';

describe('Icon 마스크 아이콘', () => {
    it('한 화면에 여러 개 그려도 마스크 id 가 겹치지 않고, 각자 제 마스크를 가리킨다', () => {
        const { container } = render(
            <>
                <Icon name="sabaFace" size="sm" />
                <Icon name="sabaFace" size="lg" />
            </>,
        );
        const masks = [...container.querySelectorAll('mask')].map((m) => m.id);
        // 아이콘마다 선(라이트)·면(다크) 마스크 두 개
        expect(masks).toHaveLength(4);
        expect(new Set(masks).size).toBe(4);
        for (const id of masks) {
            expect(id).toMatch(/^[a-zA-Z0-9_-]+$/);
            expect(container.querySelector(`path[mask="url(#${id})"]`)).not.toBeNull();
        }
    });
});
