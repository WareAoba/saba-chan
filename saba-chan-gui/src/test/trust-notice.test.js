import { act, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';
import i18n from '../i18n';
import { blockedText, showTrustWarnings } from '../utils/trustNotice';

const t = i18n.getFixedT('en', 'gui');

describe('신뢰 경고 알림', () => {
    afterEach(() => {
        window.showToast = undefined;
        delete window.api.extensionEnable;
        delete window.api.extensionList;
    });

    it('warnings의 code별 문구를 경고 알림으로 띄우고, 없거나 모르는 code면 띄우지 않는다', () => {
        window.showToast = vi.fn();
        showTrustWarnings({ success: true, id: 'x' }, t); // #36 이전 데몬
        showTrustWarnings({ warnings: [{ code: 'something_new', module: 'm' }, null] }, t);
        expect(window.showToast).not.toHaveBeenCalled();

        showTrustWarnings(
            {
                module_trust: 'modified',
                warnings: [
                    { code: 'module_modified', module: 'minecraft' },
                    { code: 'module_unverified', module: 'palworld' },
                ],
            },
            t,
        );
        expect(window.showToast).toHaveBeenCalledTimes(2);
        expect(window.showToast.mock.calls[0][0]).toMatch(/module \(minecraft\) changed after it was installed/);
        expect(window.showToast.mock.calls[0][1]).toBe('warning');
        expect(window.showToast.mock.calls[1][0]).toMatch(/\(palworld\) that did not come from the official catalog/);
    });

    it('익스텐션을 켜면 경고를 띄우되 켜기는 그대로 성공한다', async () => {
        window.showToast = vi.fn();
        window.api.extensionList = vi.fn().mockResolvedValue({ extensions: [] });
        window.api.extensionEnable = vi.fn().mockResolvedValue({
            success: true,
            id: 'music',
            trust: 'unverified',
            warnings: [{ code: 'extension_unverified', extension: 'music' }],
        });
        const { result } = renderHook(() => useExtensions(), { wrapper: ExtensionProvider });
        let ok;
        await act(async () => {
            ok = await result.current.toggleExtension('music', true);
        });
        expect(ok).toBe(true);
        const warned = window.showToast.mock.calls.filter((c) => c[1] === 'warning');
        expect(warned).toHaveLength(1);
        expect(warned[0][0]).toMatch(/extension \(music\) that did not come from the official catalog/);
    });

    it('시작이 막힌 이유는 두 코드만 문구로 바꾸고, 나머지는 원래 흐름에 맡긴다', () => {
        expect(blockedText({ success: false, action_required: 'malware_detected', file: 'a.py' }, t)).toBe(
            'Antivirus flagged a file of this unverified component: a.py',
        );
        expect(blockedText({ action_required: 'launch_outside_instance', file: 'C:/x.exe' }, t)).toMatch(
            /outside the server folder: C:\/x\.exe/,
        );
        expect(blockedText({ action_required: 'extension_required' }, t)).toBeNull();
        expect(blockedText({ success: false, message: 'x' }, t)).toBeNull();
    });
});
