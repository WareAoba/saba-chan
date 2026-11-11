import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useTranslation } from 'react-i18next';
import { ExtensionProvider, useExtensions } from '../contexts/ExtensionContext';
import i18n from '../i18n';

const words: Record<string, string> = { en: 'Hello', ja: 'こんにちは' };

function Label() {
    const { t } = useTranslation();
    // 처음 번역은 slot 등록과 함께 다시 그려진다
    useExtensions();
    return <p>{t('ext_greeter:hello', { defaultValue: 'hello' })}</p>;
}

// 익스텐션 번역은 번들을 읽을 때의 언어와 en만 받았다. 언어를 바꾸면 앱을 다시 켤 때까지 영어로 남았다
describe('실행 중에 언어를 바꾸면', () => {
    beforeEach(async () => {
        await i18n.changeLanguage('en');
        window.showToast = vi.fn();
        window.api.extensionList = vi.fn(async () => ({
            extensions: [{ id: 'greeter', enabled: true, version: '1.0.0', gui: {} }],
        })) as never;
        window.api.extensionI18n = vi.fn(async (_id: string, lang: string) => (words[lang] ? { hello: words[lang] } : null)) as never;
    });
    afterEach(async () => {
        for (const k of ['extensionList', 'extensionI18n']) {
            delete (window.api as unknown as Record<string, unknown>)[k];
        }
        await i18n.changeLanguage('en');
    });

    it('켜진 익스텐션의 번역도 새 언어로 보인다', async () => {
        render(
            <ExtensionProvider>
                <Label />
            </ExtensionProvider>,
        );
        await waitFor(() => expect(screen.getByText('Hello')).toBeTruthy());
        await act(async () => {
            await i18n.changeLanguage('ja');
        });
        await waitFor(() => expect(screen.getByText('こんにちは')).toBeTruthy());
    });
});
