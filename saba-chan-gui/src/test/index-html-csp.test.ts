import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

// Tauri는 배포 빌드에서 index.html의 <style>에 nonce를 붙이고 CSP style-src에 그 nonce를 더한다.
// nonce가 있으면 브라우저는 'unsafe-inline'을 무시하므로, 나중에 넣는 <style>(익스텐션 스타일)이 막힌다
describe('index.html', () => {
    it('인라인 <style>을 두지 않는다 (익스텐션 스타일이 CSP에 막히지 않게)', () => {
        const html = readFileSync(join(__dirname, '..', '..', 'index.html'), 'utf8');
        expect(html).not.toMatch(/<style[\s>]/i);
    });
});
