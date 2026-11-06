import { LANGUAGE_OPTIONS } from '@gui/utils/languages';
import clsx from 'clsx';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { setLanguage } from '../i18n';
import { Icon } from './icons';

/** "한국어 (Korean)" → "한국어": 메뉴에는 그 언어로 쓴 이름만 둔다 */
const selfName = (label: string) => label.replace(/\s*\(.*\)$/, '');

/** 버튼에 보이는 짧은 이름: pt-BR → PT, 중국어 둘은 갈라 보인다 (ZH-CN·ZH-TW) */
export const shortCode = (lang: string) => (lang.startsWith('zh') ? lang : lang.split('-')[0]).toUpperCase();

/** 머리·연결 화면의 언어 버튼. 고른 언어는 이 브라우저에만 기억한다 (PC 앱 설정은 그대로) */
export function LanguageMenu({ className }: { className?: string }) {
    const { t, i18n } = useTranslation();
    const [open, setOpen] = useState(false);
    const box = useRef<HTMLDivElement>(null);
    useEffect(() => {
        if (!open) return;
        const away = (e: Event) => {
            if (!box.current?.contains(e.target as Node)) setOpen(false);
        };
        const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false);
        document.addEventListener('pointerdown', away);
        document.addEventListener('keydown', esc);
        return () => {
            document.removeEventListener('pointerdown', away);
            document.removeEventListener('keydown', esc);
        };
    }, [open]);
    const pick = (value: string) => {
        setOpen(false);
        setLanguage(value);
    };
    return (
        <div className={clsx('lang-menu', className)} ref={box}>
            <button
                type="button"
                className="lang-btn"
                aria-label={t('more.language')}
                aria-haspopup="menu"
                aria-expanded={open}
                onClick={() => setOpen((v) => !v)}
            >
                <Icon name="globe" size={16} />
                <span className="lang-code">{shortCode(i18n.language)}</span>
            </button>
            {open && (
                <div className="lang-list" role="menu" aria-label={t('more.language')}>
                    {LANGUAGE_OPTIONS.map((o) => (
                        <button
                            type="button"
                            key={o.value}
                            role="menuitemradio"
                            aria-checked={o.value === i18n.language}
                            lang={o.value}
                            className={clsx('lang-item', o.value === i18n.language && 'on')}
                            onClick={() => pick(o.value)}
                        >
                            {selfName(o.label)}
                        </button>
                    ))}
                </div>
            )}
        </div>
    );
}
