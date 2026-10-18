import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { ProtocolNotice } from '../components/ServerSettingsModal';

describe('모듈 프로토콜 안내', () => {
    it('모듈이 준 프로토콜 이름은 글자로만 그리고 마크업으로 해석하지 않는다', () => {
        const { container } = render(<ProtocolNotice protocol={'rest</strong><img src=x>'} />);
        expect(container.querySelector('img')).toBeNull();
        expect(container.querySelector('strong').textContent).toBe('REST</STRONG><IMG SRC=X>');
        expect(container.textContent).toBe('This module only supports the REST</STRONG><IMG SRC=X> protocol.');
    });
});
