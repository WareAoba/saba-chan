// Electron GUI를 띄워 시나리오를 돌린다. tests/e2e/test_gui.py가 데몬을 띄운 뒤 실행한다.
// 사용: node run.mjs <시나리오> — 환경: SABA_* (시험 폴더), IPC_BASE, ELECTRON_START_URL, E2E_ARTIFACTS, E2E_ARGS(JSON)
// 결과: 마지막 줄에 JSON {ok, ...}. 실패하면 스크린샷과 화면 글자를 E2E_ARTIFACTS에 남긴다.
import fs from 'node:fs';
import path from 'node:path';
import { _electron as electron } from 'playwright-core';

const GUI_DIR = path.resolve(process.env.E2E_GUI_DIR);
const ART = process.env.E2E_ARTIFACTS;
const ARGS = JSON.parse(process.env.E2E_ARGS || '{}');
const scenario = process.argv[2];

function electronBinary() {
    const pathFile = path.join(GUI_DIR, 'node_modules', 'electron', 'path.txt');
    return path.join(GUI_DIR, 'node_modules', 'electron', 'dist', fs.readFileSync(pathFile, 'utf8').trim());
}

async function appWindow(app) {
    // 개발 모드는 DevTools 창도 연다. 앱 화면만 고른다
    for (let i = 0; i < 100; i++) {
        for (const w of app.windows()) {
            if (!w.url().startsWith('devtools://')) return w;
        }
        await new Promise((r) => setTimeout(r, 200));
    }
    throw new Error('app window not found');
}

async function snap(page, name) {
    if (!ART) return;
    fs.mkdirSync(ART, { recursive: true });
    await page.screenshot({ path: path.join(ART, `${name}.png`) }).catch(() => {});
    const text = await page.evaluate(() => document.body.innerText).catch(() => '');
    fs.writeFileSync(path.join(ART, `${name}.txt`), text);
}

const scenarios = {
    // 화면이 뜨고 데몬에 붙어 서버 목록이 보인다
    async attach(page) {
        await page.getByText(ARGS.serverName, { exact: false }).first().waitFor({ timeout: 60_000 });
        await snap(page, 'attach');
        return { text: await page.evaluate(() => document.body.innerText) };
    },
    // 서버 줄의 켜기 버튼 → 상세 화면 콘솔에 Done → 명령 입력 → 끄기. 문구는 locales/<lang>/gui.json에서 받는다(ARGS.l)
    async startConsoleStop(page) {
        const L = ARGS.l;
        await page.getByRole('button', { name: `${ARGS.serverName} ${L.start}` }).click({ timeout: 60_000 });
        await page.getByText(ARGS.serverName, { exact: true }).first().click();
        await page.getByText(/Done \(/).first().waitFor({ timeout: 60_000 });
        const input = page.getByPlaceholder(L.input_placeholder);
        await input.fill('say gui-e2e');
        await input.press('Enter');
        await page.getByText('[Server] gui-e2e').first().waitFor({ timeout: 30_000 });
        await snap(page, 'console');
        await page.getByRole('button', { name: L.stop, exact: true }).first().click();
        // 정지 확인 창의 "확인"
        await page.getByRole('button', { name: L.confirm, exact: true }).click({ timeout: 10_000 });
        await page.getByText(/Stopping the server/).first().waitFor({ timeout: 60_000 });
        await page.getByRole('button', { name: L.start, exact: true }).first().waitFor({ timeout: 60_000 });
        await snap(page, 'stopped');
        return {};
    },
    // 데몬이 꺼지면 "마지막으로 받은 상태" 알림 → 새 데몬(새 토큰)이 뜨면 알림이 걷히고 조작이 다시 된다.
    // 데몬을 끄고 켜는 일은 test_gui.py가 한다. 신호는 E2E_ARTIFACTS의 파일로 주고받는다.
    async reconnect(page) {
        const L = ARGS.l;
        const mark = (name) => fs.writeFileSync(path.join(ART, name), '');
        fs.mkdirSync(ART, { recursive: true });
        await page.getByText(ARGS.serverName, { exact: false }).first().waitFor({ timeout: 60_000 });
        mark('attached');
        await page.getByText(L.daemon_down_stale).first().waitFor({ timeout: 60_000 });
        await snap(page, 'daemon-down');
        mark('saw-down');
        await page.locator('.daemon-down-notice').first().waitFor({ state: 'detached', timeout: 90_000 });
        await page.getByRole('button', { name: `${ARGS.serverName} ${L.start}` }).click({ timeout: 30_000 });
        await page.getByText(ARGS.serverName, { exact: true }).first().click();
        await page.getByText(/Done \(/).first().waitFor({ timeout: 60_000 });
        await snap(page, 'reconnected');
        return {};
    },
    // 데몬이 안 뜨면 진행 화면에서 멈추고 [다시 시도]·[그대로 열기]를 보인다(메인으로 넘어가지 않는다).
    // 다시 시도가 실패하면 이유를 보이고, 그대로 열면 "연결되지 않았어요" 화면이 된다.
    async daemonFailed(page) {
        const L = ARGS.l;
        await page.getByText(L.failed_hint).first().waitFor({ timeout: 60_000 });
        await snap(page, 'daemon-failed');
        await page.getByRole('button', { name: L.retry, exact: true }).click();
        await page.getByRole('button', { name: L.retry, exact: true }).and(page.locator(':enabled')).waitFor({ timeout: 90_000 });
        await page.getByText(L.failed_hint).first().waitFor({ timeout: 5_000 });
        await snap(page, 'retry-failed');
        await page.getByRole('button', { name: L.continue, exact: true }).click();
        await page.getByText(L.daemon_down_title).first().waitFor({ timeout: 30_000 });
        await snap(page, 'opened-without-daemon');
        return {};
    },
    // 약관에 동의하지 않은 서버를 켜면 확인 창이 뜨고, 체크해야 [동의하고 켜기]가 눌리며, 누르면 켜진다
    async agreeAndStart(page) {
        const L = ARGS.l;
        await page.getByRole('button', { name: `${ARGS.serverName} ${L.start}` }).click({ timeout: 60_000 });
        await page.getByText(L.agreement_title).first().waitFor({ timeout: 30_000 });
        const go = page.getByRole('button', { name: L.accept_and_start, exact: true });
        if (await go.isEnabled()) throw new Error('체크하기 전에 [동의하고 켜기]가 눌림');
        await snap(page, 'agreement-dialog');
        await page.locator('.agreement-modal input[type=checkbox]').first().check();
        await go.click();
        await page.getByText(ARGS.serverName, { exact: true }).first().click();
        await page.getByText(/Done \(/).first().waitFor({ timeout: 60_000 });
        await snap(page, 'agreed-and-running');
        return {};
    },
    async explore(page) {
        await page.waitForTimeout(ARGS.waitMs ?? 8000);
        await snap(page, 'explore');
        const buttons = await page.getByRole('button').allInnerTexts();
        return { text: await page.evaluate(() => document.body.innerText), buttons };
    },
};

const app = await electron.launch({
    executablePath: electronBinary(),
    args: [GUI_DIR, '--no-sandbox'],
    cwd: GUI_DIR,
    env: process.env,
    timeout: 60_000,
});
let page;
let code = 0;
try {
    page = await appWindow(app);
    const result = await scenarios[scenario](page);
    console.log(JSON.stringify({ ok: true, ...result }));
} catch (e) {
    code = 1;
    if (page) await snap(page, `fail-${scenario}`);
    console.log(JSON.stringify({ ok: false, error: String(e?.stack || e) }));
} finally {
    await app.close().catch(() => {});
}
process.exit(code);
