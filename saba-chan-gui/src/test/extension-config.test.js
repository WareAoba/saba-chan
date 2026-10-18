import { act, fireEvent, render, screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ExtensionConfigForm } from '../components/ExtensionConfigForm';

const ext = {
    id: 'notify',
    enabled: true,
    config_fields: {
        webhook_url: { type: 'string', default: '' },
        retries: { type: 'number', default: 3 },
        verbose: { type: 'boolean', default: false },
    },
};

describe('익스텐션 전역 설정 (config_fields)', () => {
    beforeEach(() => {
        window.api = {};
    });

    it('config_fields가 없거나 채널이 없으면 그리지 않는다', () => {
        window.api.extensionConfigGet = vi.fn();
        const { container, rerender } = render(<ExtensionConfigForm ext={{ id: 'x', enabled: true }} />);
        expect(container).toBeEmptyDOMElement();
        window.api = {};
        rerender(<ExtensionConfigForm ext={ext} />);
        expect(container).toBeEmptyDOMElement();
    });

    it('저장된 값을 불러와 폼을 만들고, 바꾼 값을 타입에 맞게 저장한다', async () => {
        window.api.extensionConfigGet = vi.fn().mockResolvedValue({ config: { retries: 5 } });
        window.api.extensionConfigSet = vi.fn().mockResolvedValue({ config: {} });
        render(<ExtensionConfigForm ext={ext} />);
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Settings/i })));
        expect(window.api.extensionConfigGet).toHaveBeenCalledWith('notify');
        expect(screen.getByLabelText('retries')).toHaveValue(5);

        fireEvent.change(screen.getByLabelText('webhook_url'), { target: { value: 'https://x' } });
        fireEvent.change(screen.getByLabelText('retries'), { target: { value: '7' } });
        fireEvent.click(screen.getByLabelText('verbose'));
        await act(async () => fireEvent.click(screen.getByRole('button', { name: /Save/i })));
        expect(window.api.extensionConfigSet).toHaveBeenCalledWith('notify', {
            webhook_url: 'https://x',
            retries: 7,
            verbose: true,
        });
    });
});
