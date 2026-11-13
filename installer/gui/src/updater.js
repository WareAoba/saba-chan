// ═══════════════════════════════════════════════════════
// Saba-chan Updater — 업데이트 적용 창 (설치기 --apply)
// ═══════════════════════════════════════════════════════
// 흐름: 그림(빛이 훑음) → 비스듬히 접히며 매트릭스(적용 중) → 성공(초/파 링, GUI 재시작)·실패(빨/보 링, 닫기)

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

// 문구는 locales/<lang>/installer.json의 `updater` (빌드 때 translations.js로 묶인다, build.rs)
const TRANSLATIONS = Object.fromEntries(Object.entries(window.SABA_TRANSLATIONS).map(([l, v]) => [l, v.updater]));

let currentLanguage = 'en';

function tr(key, vars = {}) {
    return SabaUI.translate(TRANSLATIONS, currentLanguage, key, vars);
}

// ─── DOM ────────────────────────────────────────────────

const $flow = document.getElementById('flow');
const $progressMsg = document.getElementById('progress-message');
const $progressPct = document.getElementById('progress-percent');
const $progressFill = document.getElementById('progress-fill');
const $ringTitle = document.getElementById('ring-title');
const $ringSub = document.getElementById('ring-sub');
const $failSummary = document.getElementById('fail-summary');
const $btnClose = document.getElementById('btn-close');

const ring = SabaUI.ring(document.getElementById('progress-ring'));
const matrix = SabaUI.matrix(document.getElementById('matrix'));
// 그림이 접히기 시작하면 매트릭스를 돌린다
const flow = SabaUI.flow($flow, { onFold: () => matrix.start() });

function applyStaticTranslations() {
    document.documentElement.lang = currentLanguage;
    SabaUI.setWordmark(currentLanguage);
    const title = tr('appTitle');
    document.title = title;
    // Tauri 윈도우 타이틀도 갱신 (setup에서 설정한 하드코딩 타이틀 덮어쓰기)
    try {
        getCurrentWindow().setTitle(title);
    } catch (_) {}
    const minBtn = document.getElementById('btn-minimize');
    if (minBtn) minBtn.title = tr('minimize');
    if ($btnClose) $btnClose.title = tr('close');
    document.getElementById('mode-chip').textContent = tr('modeUpdate');
    document.getElementById('btn-close-failed-text').textContent = tr('close');
    $ringTitle.textContent = tr('loadingTitle');
    $ringSub.textContent = tr('loadingSub');
    $progressMsg.textContent = tr('applyLoadingManifest');
}

async function initLocalization() {
    // Rust가 앱 설정 → OS 언어 → 영어 순으로 골라 지원 언어 태그로 준다
    try {
        currentLanguage = await invoke('get_preferred_language');
    } catch (_) {}
    applyStaticTranslations();
}

// ─── 타이틀바 버튼 ─────────────────────────────────────

(function initTitleBarButtons() {
    const appWindow = getCurrentWindow();
    document.getElementById('btn-minimize')?.addEventListener('click', () => appWindow.minimize());
    $btnClose?.addEventListener('click', () => appWindow.close());
    document.getElementById('btn-close-failed')?.addEventListener('click', () => appWindow.close());
})();

// ─── 진행 줄 ───────────────────────────────────────────

function showProgress(message, percent) {
    $progressMsg.textContent = message;
    const p = Math.max(0, Math.min(100, percent));
    $progressFill.style.width = `${p}%`;
    $progressPct.textContent = `${Math.round(p)}%`;
}

// ─── 결과 ──────────────────────────────────────────────
// 그림이 덮고 있는 동안 끝났으면 접히기 시작할 때 바꾼다. 무엇을 바꿨는지는 늘어놓지 않는다

function showSuccess(relaunch) {
    flow.afterReveal(() => {
        flow.go('done');
        matrix.stop();
        $flow.classList.remove('failed');
        ring.set('ok');
        $ringTitle.textContent = tr('ringComplete');
        $ringSub.textContent = '';
        $progressFill.classList.add('ok');
        showProgress(relaunch ? tr('applyRestartingGui') : tr('ringComplete'), 100);
        $btnClose.hidden = false;
    });
}

function showFailure(summary, titleKey = 'applyFailed') {
    flow.afterReveal(() => {
        flow.go('done');
        matrix.stop();
        $flow.classList.add('failed');
        ring.set('bad');
        $ringTitle.textContent = tr(titleKey);
        $ringSub.textContent = '';
        $failSummary.textContent = summary;
        $failSummary.hidden = false;
        // 실패 화면으로 다 넘어간 뒤에 링 영역을 밀어내며 띄운다
        flow.afterSettled(() => document.getElementById('fail-wrap').classList.add('in'));
        $btnClose.hidden = false;
    });
}

// ─── 초기 로드 ──────────────────────────────────────────

(async function init() {
    // 테마 적용 — settings.json에서 읽어 data-theme에 반영 (실패하면 auto: 시스템 설정)
    try {
        const theme = await invoke('get_theme');
        if (theme && ['light', 'dark', 'auto'].includes(theme)) SabaUI.setTheme(theme);
    } catch (_) {}

    // 윈도우 표시는 Rust setup에서 처리 — JS에서는 포커스만 비동기로 시도
    // (await appWindow.show()가 특정 환경에서 블로킹되는 Tauri v2 이슈 회피)
    const appWindow = getCurrentWindow();
    appWindow.show()
        .then(() => appWindow.setAlwaysOnTop(true))
        .then(() => appWindow.setFocus())
        .then(() => setTimeout(() => appWindow.setAlwaysOnTop(false).catch(() => {}), 800))
        .catch(() => {});

    await initLocalization();

    let mode = { relaunch: false };
    try {
        mode = await invoke('get_apply_mode');
    } catch (e) {
        console.log('Apply mode check:', e);
    }
    await enterApplyMode(mode);
})();

// ═══════════════════════════════════════════════════════
// Apply — 그림을 보이는 동안 적용을 시작하고, 진행은 아래 줄에
// ═══════════════════════════════════════════════════════

/** 손으로 되살리는 차례 (데몬이 넘긴 `Recovery` 모양: stop, exes[{from,to}], missingCore, installDir, data…) */
function recoverySteps(leadKey, r = {}) {
    const steps = [];
    if (r.stop) steps.push(tr('recoverStop'));
    if (r.exes?.length) steps.push(tr('recoverExes', { list: r.exes.map((e) => `${e.from} → ${e.to}`).join(', ') }));
    if (r.missingCore) steps.push(tr('recoverMissingCore', r));
    if (r.data) steps.push(tr('recoverData', r));
    steps.push(tr('recoverStart'));
    return `${tr(leadKey)} ${steps.map((step, i) => `${i + 1}) ${step}`).join(' ')}`;
}

async function enterApplyMode(mode) {
    const { listen } = window.__TAURI__.event;

    // 그림 위로 빛이 훑고, 머문 뒤 접힌다
    void $flow.offsetWidth;
    flow.reveal(100);
    showProgress(tr('applyLoadingManifest'), 0);

    // 등록이 끝난 뒤에 적용을 시작한다: 먼저 시작하면 첫 이벤트(빠른 실패 포함)를 놓친다
    await listen('apply:progress', (event) => {
        const { step, message, percent, applied, failed } = event.payload;

        if (step === 'manifest') {
            if (percent >= 0) showProgress(tr('applyLoadingManifest'), percent);
        } else if (step === 'applying') {
            if (percent >= 0) showProgress(tr('applyApplyingFiles'), percent);
        } else if (step === 'complete') {
            showSuccess(mode.relaunch);
        } else if (step === 'error') {
            // message는 진단용 원문(영어)이라 화면에는 번역한 요약만 보인다
            console.warn('[Apply]', message);
            showFailure(tr('applyFailedCount', { applied: applied ? applied.length : 0, failed: failed || 0 }));
        }
    });

    // 적용은 됐지만 앱을 다시 띄우지 못했다: 창을 닫지 않고 직접 실행하라고 알린다
    await listen('apply:relaunch-failed', () => showFailure(tr('relaunchFailedSub'), 'relaunchFailedTitle'));
    // 새 판이 실행되지 않아 이전 판으로 되돌렸지만 이전 판도 띄우지 못했다
    await listen('apply:rollback-relaunch-failed', () => showFailure(tr('rollbackRelaunchFailedSub'), 'relaunchFailedTitle'));
    // 되돌렸지만 설정을 되돌리지 못했다: 이전 판을 띄우지 않았다. 사본 자리를 알린다
    // 다 되돌리지 못했거나(`restore-failed`) 새 판을 끝내지 못해 그대로 뒀다(`left-in-place`): 창을 닫지 않고
    // 실제 상태에 맞는 차례(끝내기 → 실행 파일 → 데이터 → 실행)를 보인다. 인스턴스 폴더를 따로 둔 경우도 그 경로다
    await listen('apply:restore-failed', (event) => showFailure(recoverySteps('restoreFailedLead', event.payload), 'restoreFailedTitle'));
    await listen('apply:left-in-place', (event) => showFailure(recoverySteps('leftInPlaceLead', event.payload)));

    // 거부 사유는 진단용 원문이라 화면에는 번역한 요약만 보인다
    invoke('start_apply').catch(err => {
        console.warn('[Apply]', err);
        showFailure(tr('applyFailedCount', { applied: 0, failed: 1 }));
    });
}
