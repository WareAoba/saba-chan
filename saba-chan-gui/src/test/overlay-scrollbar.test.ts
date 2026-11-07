import { beforeAll, describe, expect, it } from 'vitest';
import { installOverlayScrollbars } from '../utils/overlayScrollbar';

// 스크롤바는 포인터를 올릴 때가 아니라 사람이 실제로 스크롤할 때만 뜬다 (사용자 11-07)
const box = document.createElement('div');
const thumb = () => document.querySelector('.ovs-y') as HTMLElement;

beforeAll(() => {
    box.style.overflowY = 'auto';
    Object.defineProperty(box, 'scrollHeight', { value: 1000 });
    Object.defineProperty(box, 'clientHeight', { value: 200 });
    Object.defineProperty(box, 'clientWidth', { value: 300 });
    box.getBoundingClientRect = () => ({ left: 0, top: 0, width: 300, height: 200, right: 300, bottom: 200 }) as DOMRect;
    document.body.appendChild(box);
    document.elementFromPoint = () => box;
    installOverlayScrollbars();
});

describe('오버레이 스크롤바', () => {
    it('포인터만 올리면 뜨지 않는다', () => {
        box.dispatchEvent(new Event('pointermove', { bubbles: true }));
        expect(thumb().classList.contains('show')).toBe(false);
    });

    it('코드가 스크롤하면 뜨지 않는다 (콘솔 자동 내림 등)', () => {
        box.dispatchEvent(new Event('scroll'));
        expect(thumb().classList.contains('show')).toBe(false);
    });

    it('휠로 스크롤하면 뜬다', () => {
        box.dispatchEvent(new Event('wheel', { bubbles: true }));
        box.dispatchEvent(new Event('scroll'));
        expect(thumb().classList.contains('show')).toBe(true);
    });
});

// 바깥 상자에 잘린 부분에는 손잡이를 그리지 않는다: 다른 요소 위로 뚫고 나오지 않게 (사용자 11-07)
describe('오버레이 스크롤바 자르기', () => {
    it('바깥 상자 밖으로 나간 부분을 잘라 낸다', () => {
        const outer = document.createElement('div');
        outer.style.overflowX = 'hidden';
        outer.style.overflowY = 'hidden';
        Object.defineProperty(outer, 'clientHeight', { value: 100 });
        Object.defineProperty(outer, 'clientWidth', { value: 300 });
        outer.getBoundingClientRect = () => ({ left: 0, top: 0, width: 300, height: 100, right: 300, bottom: 100 }) as DOMRect;
        document.body.appendChild(outer);
        outer.appendChild(box);
        box.dispatchEvent(new Event('wheel', { bubbles: true }));
        box.dispatchEvent(new Event('scroll'));
        // 손잡이 높이 40(200*200/1000), 위에서 0부터: 바깥 상자(높이 100) 안이라 자를 것이 없다
        expect(thumb().style.clipPath).toBe('');
        Object.defineProperty(box, 'scrollTop', { value: 800, configurable: true });
        box.dispatchEvent(new Event('wheel', { bubbles: true }));
        box.dispatchEvent(new Event('scroll'));
        // 맨 아래(160~200)는 바깥 상자 밖이라 통째로 잘려 숨는다
        expect(thumb().classList.contains('show')).toBe(false);
    });
});
