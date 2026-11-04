/**
 * 정보 페이지 리뉴얼 (2026-11-05 시안 확정): 위에서부터 버전 → 사바쨩 자기소개 → 제작자.
 * 제작자 카드는 한글 닉네임 없이 GitHub 아이디와 소개만, 배경은 별밤·오로라.
 */
import fs from 'node:fs';
import path from 'node:path';
import { act, fireEvent, render } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import SettingsModal from '../components/Modals/SettingsModal';
import { startAurora } from '../components/Modals/aboutSky';

const LOCALES = path.resolve(__dirname, '../../../locales');
const LANGS = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

async function openAbout() {
    window.api.getAppVersion = vi.fn(() => Promise.resolve('0.1.12'));
    window.api.getComponentInfo = vi.fn(() => new Promise(() => {}));
    const view = render(<SettingsModal isOpen onClose={vi.fn()} />);
    await act(async () => fireEvent.click(view.container.querySelector('.settings-about-btn')));
    return view.container;
}

describe('정보 페이지 배치', () => {
    it('버전 카드가 맨 위, 자기소개가 가운데, 제작자 카드가 맨 아래에 온다', async () => {
        const c = await openAbout();
        const order = [...c.querySelectorAll('.about-scroll-area > *')].map((el) => el.className);
        expect(order.map((n) => n.split(' ').find((x) => /^about-(app|intro|dev)-card$/.test(x)))).toEqual([
            'about-app-card',
            'about-intro-card',
            'about-dev-card',
        ]);
    });

    it('컴포넌트 버전마다 아이콘이 붙는다', async () => {
        const c = await openAbout();
        const items = c.querySelectorAll('.about-component-item');
        expect(items).toHaveLength(4);
        for (const item of items) expect(item.querySelector('.about-component-icon svg')).not.toBeNull();
    });

    it('사바쨩 얼굴은 그라데이션 순환 링 안에 있다', async () => {
        const c = await openAbout();
        expect(c.querySelector('.about-app-ring .about-app-logo')).not.toBeNull();
    });

    it('제작자 카드는 한글 닉네임 없이 GitHub 아이디와 소개를 보인다', async () => {
        const c = await openAbout();
        const dev = c.querySelector('.about-dev-card');
        expect(dev.querySelector('.about-dev-nickname')).toBeNull();
        expect(dev.textContent).toContain('@WareAoba');
        expect(dev.textContent).toContain('AI tamer');
        expect(dev.querySelector('canvas.about-sky-stars')).not.toBeNull();
        expect(dev.querySelector('canvas.about-sky-aurora')).not.toBeNull();
    });

    it('자기소개는 상반신 폴라로이드 사진과 하루 그림 네 장을 담는다', async () => {
        const c = await openAbout();
        const intro = c.querySelector('.about-intro-card');
        expect(intro.querySelector('.about-polaroid img').getAttribute('src')).toBe('./saba/mermaid-upper.webp');
        expect(intro.querySelectorAll('.about-intro-day img')).toHaveLength(4);
    });
});

describe('정보 페이지 문구', () => {
    const flat = (o, p = '', out = {}) => {
        for (const [k, v] of Object.entries(o)) {
            if (v && typeof v === 'object') flat(v, `${p}${k}.`, out);
            else out[`${p}${k}`] = v;
        }
        return out;
    };
    const about = (lang) =>
        Object.keys(flat(JSON.parse(fs.readFileSync(path.join(LOCALES, lang, 'gui.json'), 'utf8'))))
            .filter((k) => k.startsWith('settings_modal.about_intro'))
            .sort();

    it('자기소개 문구는 지원 언어마다 ko 와 같은 키를 가진다', () => {
        const ko = about('ko');
        expect(ko.length).toBeGreaterThan(10);
        for (const lang of LANGS) expect([lang, about(lang)]).toEqual([lang, ko]);
    });

    it('쓰지 않게 된 한글 닉네임 키는 남기지 않는다', () => {
        for (const lang of LANGS) {
            const gui = JSON.parse(fs.readFileSync(path.join(LOCALES, lang, 'gui.json'), 'utf8'));
            expect([lang, 'dev_nickname' in gui.settings_modal]).toEqual([lang, false]);
        }
    });
});

describe('오로라', () => {
    it('WebGL 이 없으면 아무것도 그리지 않고 멈추개만 돌려준다', () => {
        const canvas = document.createElement('canvas');
        canvas.getContext = () => null;
        const stop = startAurora(canvas);
        expect(typeof stop).toBe('function');
        expect(() => stop()).not.toThrow();
    });
});
