/**
 * 확장 GUI 하나의 오류가 다른 확장이나 앱 전체로 번지지 않는다.
 */
import { render, renderHook, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import ExtensionSlot from '../components/ExtensionSlot';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';

const ext = (id) => ({ id, enabled: true, version: '1.0.0', gui: { bundle: 'x.js' }, gui_allowed: true });
const Good = () => <span>good slot</span>;

describe('확장 슬롯 오류 격리', () => {
    beforeEach(() => {
        window.api.extensionGuiBundle = vi.fn().mockResolvedValue(null);
        window.api.extensionGuiStyles = vi.fn().mockResolvedValue(null);
        window.api.extensionI18n = vi.fn().mockResolvedValue(null);
        window.SabaExtBad = {
            registerSlots: () => {
                throw new Error('bad register');
            },
        };
        window.SabaExtGood = { registerSlots: () => ({ 'test.slot': Good }) };
        vi.spyOn(console, 'warn').mockImplementation(() => {});
        vi.spyOn(console, 'error').mockImplementation(() => {});
    });
    afterEach(() => {
        for (const k of ['extensionList', 'extensionGuiBundle', 'extensionGuiStyles', 'extensionI18n'])
            delete window.api[k];
        delete window.SabaExtBad;
        delete window.SabaExtGood;
        vi.restoreAllMocks();
    });

    it('registerSlots가 throw하는 확장이 있어도 다른 확장의 슬롯은 등록된다', async () => {
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [ext('bad'), ext('good')] });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        await waitFor(() => expect(result.current.slots['test.slot']).toHaveLength(1));
    });

    it('슬롯 컴포넌트 하나가 그리다 throw하면 그 자리만 비우고 나머지는 그린다', async () => {
        const Boom = () => {
            throw new Error('render boom');
        };
        window.SabaExtGood = { registerSlots: () => ({ 'test.slot': [Boom, Good] }) };
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [ext('good')] });
        render(
            <ExtensionProvider>
                <ExtensionSlot slotId="test.slot" />
            </ExtensionProvider>,
        );
        expect(await screen.findByText('good slot')).toBeTruthy();
    });
});
