// ═══════════════════════════════════════════════════════
// Saba-chan Installer — 미니 GUI 설치·제거 창
// ═══════════════════════════════════════════════════════
// 흐름: 시작 → (시트 올라옴) 설정 → [설치]: 그림이 펼쳐졌다 접히며 매트릭스(설치 중) → 완료·실패

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

// ═══════════════════════════════════════════════════════
// i18n
// ═══════════════════════════════════════════════════════

// 문구는 locales/<lang>/installer.json의 `installer` (빌드 때 translations.js로 묶인다, build.rs)
const T = Object.fromEntries(Object.entries(window.SABA_TRANSLATIONS).map(([l, v]) => [l, v.installer]));

let lang = 'en';

function tr(key, vars = {}) {
    return SabaUI.translate(T, lang, key, vars);
}

// Step → i18n key mappings for progress messages
const INSTALL_STEP_KEYS = {
    'prepare': 'progressPrepare',
    'elevate': 'progressElevate',
    'fetch': 'progressFetch',
    'download': 'progressDownload',
    'components-dir': 'progressComponentsDir',
    'extract': 'progressExtract',
    'modules-shared': 'progressModulesShared',
    'modules-download': 'progressModulesDownload',
    'modules-extract': 'progressModulesExtract',
    'extensions': 'progressExtensions',
    'runtime-python': 'progressRuntimePython',
    'config': 'progressConfig',
    'config-lang': 'progressConfigLang',
    'registry': 'progressRegistry',
    'shortcuts': 'progressShortcuts',
    'command': 'progressCommand',
};

const UNINSTALL_STEP_KEYS = {
    'detect': 'uninstallProgressDetect',
    'stop': 'uninstallProgressStop',
    'sandbox': 'uninstallProgressSandbox',
    'shortcuts': 'uninstallProgressShortcuts',
    'files': 'uninstallProgressFiles',
    'data': 'uninstallProgressData',
    'data-keep': 'uninstallProgressDataKeep',
    'temp': 'uninstallProgressTemp',
    'modules': 'uninstallProgressModules',
    'modules-keep': 'uninstallProgressModulesKeep',
    'registry': 'uninstallProgressRegistry',
    'self-delete': 'uninstallProgressSelfDelete',
};

/** Translate a progress step name, falling back to the raw message */
/** 제거 전 서버 종료 문제 한 줄 (installer core `StopWarning`의 `kind`) */
function uninstallWarning(w) {
    const vars = { server: w.server, detail: w.detail };
    switch (w.kind) {
        case 'servers_unlisted': return tr('uninstallWarnServersUnlisted', vars);
        case 'server_force_stopped': return tr('uninstallWarnServerForceStopped', vars);
        case 'daemon_unanswered': return tr('uninstallWarnDaemonUnanswered', vars);
        default: return String(w.kind || '');
    }
}

function trStep(stepMap, step, fallback) {
    const key = stepMap[step];
    if (!key) return fallback;
    const b = T[lang] || T.en;
    return b[key] || T.en[key] || fallback;
}

function applyTranslations() {
    document.documentElement.lang = lang;
    SabaUI.setWordmark(lang);
    const s = (id, key) => { const el = document.getElementById(id); if (el) el.textContent = tr(key); };
    s('welcome-text', moveFirst ? 'moveFirst' : setupMode ? 'welcomeSetup' : 'welcome');
    s('welcome-setup-sub', moveFirst ? 'moveFirstSub' : 'welcomeSetupSub');
    s('btn-next-text', moveFirst ? 'btnClose' : 'btnNext');
    s('mode-chip', uninstallMode ? 'modeUninstall' : setupMode ? 'modeSetup' : 'modeInstall');
    s('step-start', 'stepStart');
    s('step-settings', setupMode ? 'stepPick' : 'stepSettings');
    s('step-install', setupMode ? 'stepPrepare' : 'stepInstall');
    s('label-setup', 'labelSetup');
    s('setup-python', 'setupPython');
    s('setup-command', 'setupCommand');
    s('setup-modules', 'setupModules');
    s('hint-setup', 'hintSetup');
    s('step-done', 'stepDone');
    s('label-path', 'labelPath');
    s('label-components', 'labelComponents');
    s('hint-components', 'hintComponents');
    s('label-modules', 'labelModules');
    s('hint-modules', setupMode ? 'hintModulesSetup' : 'hintModules');
    s('label-channel', 'labelChannel');
    s('hint-channel', 'hintChannel');
    $channel?.querySelectorAll('option').forEach(o => { o.textContent = tr(CHANNEL_LABELS[o.value] || 'channelStable'); });
    s('label-options', 'labelOptions');
    s('label-desktop', 'labelDesktop');
    s('label-startmenu', 'labelStartMenu');
    s('privacy-notice', 'privacyNotice');
    s('btn-install-text', setupMode ? 'btnSetup' : 'btnInstall');
    s('btn-close-text', 'btnClose');
    s('btn-launch-text', 'btnLaunch');
    renderModules();
    s('btn-retry-text', 'btnRetry');
    s('uninstall-title', 'uninstallTitle');
    s('uninstall-desc', 'uninstallDesc');
    s('btn-cancel-text', 'btnCancel');
    s('btn-uninstall-text', 'btnUninstall');
    s('label-keep-instances', 'uninstallKeepInstances');
    s('hint-keep-instances', 'uninstallKeepInstancesHint');
    s('label-keep-settings', 'uninstallKeepSettings');
    s('hint-keep-settings', 'uninstallKeepSettingsHint');
}

// ═══════════════════════════════════════════════════════
// DOM refs
// ═══════════════════════════════════════════════════════

const $flowEl = document.getElementById('flow');
const $statusText = document.getElementById('status-text');
const $statusSub = document.getElementById('status-sub');
const $warnings = document.getElementById('complete-warnings');
const $warnWrap = document.getElementById('warn-wrap');
const $completeLog = document.getElementById('complete-log');
const $btnLaunch = document.getElementById('btn-launch');
const $btnRetry = document.getElementById('btn-retry');
const $msg = document.getElementById('progress-message');
const $pct = document.getElementById('progress-percent');
const $fill = document.getElementById('progress-fill');
const $installPath = document.getElementById('install-path');

const ring = SabaUI.ring(document.getElementById('glow-ring'));
const matrix = SabaUI.matrix(document.getElementById('matrix'));
// 그림이 접히기 시작하면 매트릭스를 돌린다
const flow = SabaUI.flow($flowEl, { onFold: () => matrix.start() });

let uninstallMode = false;
// 맥 앱을 처음 켤 때 준비 (`--setup`): 설치 위치·바로가기 없이 모듈만 고르고, 준비를 마치면 앱을 연다
let setupMode = false;
// 디스크 이미지·임시 자리에서 열림: 준비하지 않고 응용 프로그램 폴더로 옮기라고만 알린다
let moveFirst = false;

// ═══════════════════════════════════════════════════════
// 타이틀바
// ═══════════════════════════════════════════════════════

const appWindow = getCurrentWindow();
document.getElementById('btn-minimize')?.addEventListener('click', () => appWindow.minimize());
const $btnCloseWin = document.getElementById('btn-close');
$btnCloseWin?.addEventListener('click', () => appWindow.close());

/** 설치·제거가 도는 동안 닫기를 막는다 (중간에 끊기면 반쯤 설치·제거된 채 남는다). 창 쪽(Rust)도 닫기 요청을 막는다 */
function setWorking(on) {
    if ($btnCloseWin) $btnCloseWin.disabled = on;
}

function esc(s) { const d = document.createElement('div'); d.textContent = s; return d.innerHTML; }

// ═══════════════════════════════════════════════════════
// 상태 기계
// ═══════════════════════════════════════════════════════
// 단계는 #flow의 data-step 하나로 보인다 (common.css):
// start(시작) → sheet(설정 시트) → expand(그림 펼침, 진행 줄) → install(접히며 매트릭스) → done(완료·실패)
// 제거: uninstall(제거 시트) → install → done

let currentState = 'welcome';

/** 상단 바 단계 표시 (시작·설정·설치·완료) */
function markSteps(index) {
    document.querySelectorAll('#steps b').forEach((b, i) => {
        b.className = i < index ? 'done' : i === index ? 'cur' : '';
    });
}

function setProgress(percent, message) {
    if (message != null) $msg.textContent = message;
    $pct.textContent = `${percent}%`;
    $fill.style.width = `${percent}%`;
}

function resetProgress() {
    $fill.classList.remove('ok');
    setProgress(0, tr('preparing'));
}

function enterWelcome() {
    currentState = 'welcome';
    flow.reset('start');
    markSteps(0);
}

function enterSettings() {
    currentState = 'settings';
    document.getElementById('install-error').hidden = true;
    flow.reset('sheet');
    ring.set('idle');
    markSteps(1);
}

// 시트 바깥(뒤 화면)을 누르면 설정을 닫고 시작으로 (설치 중이 아닐 때만)
document.getElementById('sheet-overlay').addEventListener('click', () => {
    if (currentState === 'settings') enterWelcome();
});

function enterInstalling() {
    currentState = 'installing';
    $flowEl.classList.remove('failed');
    $warnings.hidden = true;
    $warnWrap.classList.remove('in');
    setWorking(true);
    resetProgress();
    $statusText.textContent = tr(setupMode ? 'setupRunning' : 'installing');
    $statusSub.textContent = tr(setupMode ? 'setupSub' : 'installSub');
    ring.set('spin');
    markSteps(2);
    // [설치]를 누른 때부터 진행 줄이 올라오고, 그림이 펼쳐졌다 접히며 매트릭스가 드러난다
    flow.reveal();
}

/** 완료·실패 화면: 매트릭스만 끄고 링 색을 바꾼다. 진행 줄 자리에 버튼 */
function showResult(ok, { title, sub, warningTitle, lines = [], logPath, launch = false, retry = false }) {
    setWorking(false);
    flow.afterReveal(() => {
        flow.go('done');
        matrix.stop();
        $flowEl.classList.toggle('failed', !ok);
        ring.set(ok ? 'ok' : 'bad');
        markSteps(3);
        $statusText.textContent = title;
        $statusSub.textContent = sub || '';
        // 띠 안: 아이콘 옆에 제목(있으면)과 줄들
        const text = document.createElement('div');
        if (warningTitle) {
            const head = document.createElement('strong');
            head.textContent = warningTitle;
            text.append(head);
        }
        if (lines.length) text.append(lines.join('\n'));
        $warnings.replaceChildren(text);
        $warnings.classList.toggle('stop', !ok);
        $warnings.hidden = !text.childNodes.length;
        // 경고는 완료 화면으로 다 넘어간 뒤에 링 영역을 밀어내며 띄운다
        $warnWrap.classList.remove('in');
        if (!$warnings.hidden) flow.afterSettled(() => $warnWrap.classList.add('in'));
        $completeLog.textContent = logPath ? tr('completeLog', { path: logPath }) : '';
        $btnLaunch.hidden = !launch;
        $btnRetry.hidden = !retry;
    });
}

function enterComplete(p) {
    currentState = 'complete';
    const warnings = (p && p.warnings) || [];
    showResult(true, {
        title: tr(setupMode ? 'setupCompleteTitle' : 'completeTitle'),
        sub: tr(setupMode ? 'setupCompleteSub' : 'completeSub'),
        warningTitle: warnings.length ? tr('completeWarnings') : '',
        lines: warnings,
        logPath: warnings.length ? p.log_path : '',
        launch: true,
    });
}

// 설치기가 알려 주는 실패 종류 → 번역 키 (원문은 아래 상자에 함께 보인다)
const ERROR_KEYS = {
    elevation_failed: 'errElevation',
    fetch_failed: 'fetchFailed',
    channel_older: 'errChannelOlder',
    installer_outdated: 'errOutdated',
    release_incomplete: 'errIncomplete',
    profile_failed: 'errProfile',
    download_failed: 'errDownload',
    extract_failed: 'errExtract',
    installed_elsewhere: 'errElsewhere',
    daemon_running: 'errDaemonRunning',
    components_dir_invalid: 'errComponentsDir',
    components_dir_failed: 'errComponentsDir',
    invalid_install_dir: 'errInstallDir',
};

// 모듈·익스텐션 위치를 거절한 이유 (`component_dirs::PathError` 코드) → 번역 키
const COMPONENT_DIR_KEYS = {
    not_absolute: 'compNotAbsolute',
    root_or_home: 'compRootOrHome',
    overlaps_current: 'compOverlap',
    overlaps_other: 'compOverlap',
    overlaps_install: 'compInsideInstall',
};

function enterError(p, titleKey = 'installFailed', canRetry = true) {
    currentState = 'error';
    const key = ERROR_KEYS[p.error_code];
    showResult(false, {
        title: tr(titleKey),
        sub: key ? tr(key) : p.message,
        lines: key && p.message ? [p.message] : [],
        logPath: p.log_path,
        retry: canRetry,
    });
}

// ═══════════════════════════════════════════════════════
// 이벤트: Welcome → Settings
// ═══════════════════════════════════════════════════════

document.getElementById('btn-next').addEventListener('click', () => moveFirst ? appWindow.close() : enterSettings());

// ═══════════════════════════════════════════════════════
// 이벤트: Settings 패널 내부
// ═══════════════════════════════════════════════════════

// 경로 찾기
document.getElementById('btn-browse')?.addEventListener('click', async () => {
    try {
        const r = await invoke('browse_folder');
        if (r) { $installPath.value = await invoke('set_install_path', { path: r }); }
    } catch (_) {}
});

// 모듈·익스텐션 위치: 설치기가 확인하고, 쓸 수 없으면 이유를 칸 아래에 보인다. 성공하면 true
const $componentsPath = document.getElementById('components-path');
const $componentsError = document.getElementById('components-error');
async function setComponentsDir(path) {
    try {
        $componentsPath.value = await invoke('set_components_dir', { path });
        $componentsError.hidden = true;
        return true;
    } catch (code) {
        $componentsError.textContent = tr(COMPONENT_DIR_KEYS[code] || 'compInvalid');
        $componentsError.hidden = false;
        return false;
    }
}
document.getElementById('btn-browse-components')?.addEventListener('click', async () => {
    try {
        const r = await invoke('browse_components_folder');
        if (r) await setComponentsDir(r);
    } catch (_) {}
});
$componentsPath?.addEventListener('change', () => setComponentsDir($componentsPath.value));

// 업데이트 채널: 고른 채널의 최신 판을 설치하고 이후 업데이트도 그 채널로 받는다. 채널마다 릴리즈가 달라 모듈 목록을 다시 받는다
const $channel = document.getElementById('update-channel');
const CHANNEL_LABELS = { stable: 'channelStable', rc: 'channelRc', beta: 'channelBeta' };
$channel?.addEventListener('change', async () => {
    try {
        await invoke('set_update_channel', { channel: $channel.value });
    } catch (_) {
        return;
    }
    await loadModules();
});

// 비어 있지 않은 폴더를 고르면 설치기가 그 아래 Saba-chan 폴더로 바꿔 돌려준다
$installPath?.addEventListener('change', async () => {
    $installPath.value = await invoke('set_install_path', { path: $installPath.value });
});

// 모듈
let selectedModules = new Set();

/** 앱 언어의 이름·설명 (카탈로그 번역). 없으면 기본값 */
function localized(map, fallback) {
    if (!map) return fallback;
    return map[lang] || map[lang.split('-')[0]] || fallback;
}

/** 이 OS·CPU에서 돌지 않는 이유 (설치기 core가 데몬과 같은 판정으로 단다) */
function unsupportedLabel(m) {
    if (m.unsupported_reason === 'arch') return tr('unsupportedArch');
    if (m.unsupported_reason === 'os') return tr('unsupportedOs');
    return '';
}

let availableModules = null;

async function loadModules() {
    try {
        availableModules = await invoke('get_available_modules');
        // 채널을 바꿔 다시 받았으면 그 채널에 없는 모듈은 고른 것에서 뺀다
        Array.from(selectedModules).filter(id => !availableModules.some(m => m.id === id)).forEach(id => selectedModules.delete(id));
        // 준비 모드는 이 기기에서 도는 게임을 처음부터 골라 둔다 (빼고 싶은 것만 끈다)
        if (setupMode) availableModules.filter(m => !m.unsupported_reason).forEach(m => selectedModules.add(m.id));
    } catch (_) {
        availableModules = [];
    }
    renderModules();
}

/** 모듈 목록을 그린다 (언어를 바꾸면 다시 그린다). 돌지 않는 모듈은 고를 수 없고 이유를 붙인다 */
function renderModules() {
    const $list = document.getElementById('module-list');
    if (!$list || availableModules === null) return;
    $list.innerHTML = '';
    if (!availableModules.length) {
        const note = document.createElement('div');
        note.className = 'module-desc';
        note.textContent = tr('modulesUnavailable');
        $list.appendChild(note);
        return;
    }
    // 돌릴 수 있는 것이 먼저
    const mods = [...availableModules].sort((a, b) => Number(!!a.unsupported_reason) - Number(!!b.unsupported_reason));
    for (const m of mods) {
        const why = unsupportedLabel(m);
        const name = localized(m.display_names, m.name);
        const card = document.createElement('div');
        card.className = 'module-card';
        card.dataset.id = m.id;
        // 의존 익스텐션 표시 (돌지 않는 모듈은 이유만)
        const extBadges = !why && (m.required_extensions || []).length > 0
            ? `<div class="module-extensions">${(m.required_extensions || []).map(e => `<span class="ext-badge">${esc(e)}</span>`).join(' ')}</div>`
            : '';
        card.innerHTML = `
            <div class="module-icon"></div>
            <div class="module-info">
                <div class="module-name">${esc(name)}</div>
                <div class="module-desc">${esc(localized(m.descriptions, m.description))}</div>
                ${why ? `<span class="module-why">${esc(why)}</span>` : extBadges}
            </div>
            <div class="module-check">
                <input type="checkbox" class="mod-chk" data-id="${esc(m.id)}" ${why ? 'disabled' : ''} />
                <span class="box" aria-hidden="true"><svg viewBox="0 0 16 16" focusable="false"><path d="M4 8.2 6.8 11 12 5"/></svg></span>
            </div>`;
        // 아이콘: img를 먼저 만들고 error 핸들러 연결 후 src 설정
        const iconContainer = card.querySelector('.module-icon');
        const iconImg = document.createElement('img');
        iconImg.alt = name;
        iconImg.addEventListener('error', () => {
            iconImg.remove();
            iconContainer.classList.add('icon-fallback');
            iconContainer.textContent = '🎮';
        });
        iconImg.src = m.icon;
        iconContainer.appendChild(iconImg);
        const cb = card.querySelector('.mod-chk');
        if (why) {
            card.classList.add('unsupported');
            selectedModules.delete(m.id);
        } else {
            cb.checked = selectedModules.has(m.id);
            card.classList.toggle('selected', cb.checked);
            card.addEventListener('click', (e) => {
                if (e.target.tagName === 'INPUT') return;
                cb.checked = !cb.checked;
                toggleMod(m.id, cb.checked);
            });
            cb.addEventListener('change', (e) => toggleMod(m.id, e.target.checked));
        }
        $list.appendChild(card);
    }
}

function toggleMod(id, on) {
    on ? selectedModules.add(id) : selectedModules.delete(id);
    document.querySelectorAll('.module-card').forEach(c => {
        c.classList.toggle('selected', selectedModules.has(c.dataset.id));
    });
}

const $installError = document.getElementById('install-error');
const $btnInstall = document.getElementById('btn-install');

/** 설치 폴더를 만들고 쓸 수 있는지 확인한다 (필요하면 UAC). 실패하면 시트에 남아 이유를 보인다 */
async function prepareInstall() {
    currentState = 'preparing';
    setWorking(true);
    $installError.hidden = true;
    $btnInstall.disabled = true;
    try {
        await invoke('prepare_install');
        return true;
    } catch (e) {
        currentState = 'settings';
        setWorking(false);
        const key = ERROR_KEYS[e?.error_code];
        $installError.textContent = key ? tr(key) : (e?.message ?? String(e));
        $installError.hidden = false;
        return false;
    } finally {
        $btnInstall.disabled = false;
    }
}

// 설치 버튼
document.getElementById('btn-install')?.addEventListener('click', async () => {
    if (currentState !== 'settings') return;
    if (setupMode) {
        await startSetup();
        return;
    }
    // 옵션 저장
    $installPath.value = await invoke('set_install_path', { path: $installPath.value });
    if (!(await setComponentsDir($componentsPath.value))) return;
    await invoke('set_language', { language: lang }); // 설치 직전 최종 동기화
    await invoke('set_shortcut_options', {
        desktop: document.getElementById('chk-desktop').checked,
        startMenu: document.getElementById('chk-startmenu').checked,
    });
    if ($channel?.value) await invoke('set_update_channel', { channel: $channel.value });
    await invoke('set_selected_modules', { modules: Array.from(selectedModules) });

    // 설치 폴더 준비: 관리자 권한(UAC)이 필요하면 여기서 끝낸 뒤에 그림과 설치를 시작한다
    if (!(await prepareInstall())) return;

    // 설치 모드 진입
    enterInstalling();

    // 릴리즈 체크
    $msg.textContent = tr('fetchingRelease');

    try {
        await invoke('fetch_latest_release');
    } catch (e) {
        enterError({ error_code: e?.error_code ?? 'fetch_failed', message: e?.message ?? String(e) });
        return;
    }

    try {
        await invoke('start_install');
    } catch (e) {
        // 이미 도는 설치가 있으면(두 번 누름) 그 진행 화면을 그대로 둔다
        if (String(e) === 'busy') return;
        enterError({ message: String(e) });
    }
});

/** 준비 모드: 설치 폴더는 이 앱 자리 그대로라 준비할 것이 없다. 고른 모듈과 언어만 넘기고 시작한다 */
async function startSetup() {
    await invoke('set_language', { language: lang });
    await invoke('set_selected_modules', { modules: Array.from(selectedModules) });
    enterInstalling();
    try {
        await invoke('start_setup');
    } catch (e) {
        if (String(e) === 'busy') return;
        enterError({ message: String(e) });
    }
}

// ═══════════════════════════════════════════════════════
// 설치 진행 이벤트
// ═══════════════════════════════════════════════════════

listen('install:progress', (ev) => {
    const p = ev.payload;

    if (p.step === 'error') {
        enterError(p);
        return;
    }

    if (p.step === 'complete') {
        $fill.classList.add('ok');
        setProgress(100, p.message);
        setTimeout(() => enterComplete(p), 600);
        return;
    }

    const stepMsg = trStep(INSTALL_STEP_KEYS, p.step, p.message);
    // 다운로드는 받은 크기를 함께 보인다
    if (p.download_received != null && p.download_total > 0) {
        const mbReceived = (p.download_received / 1048576).toFixed(1);
        const mbTotal = (p.download_total / 1048576).toFixed(1);
        const dlPct = Math.round(p.download_received / p.download_total * 100);
        setProgress(p.percent, `${stepMsg} (${mbReceived}/${mbTotal} MB — ${dlPct}%)`);
    } else if (p.download_received != null) {
        setProgress(p.percent, `${stepMsg} (${(p.download_received / 1048576).toFixed(1)} MB)`);
    } else {
        setProgress(p.percent, stepMsg);
    }
});

// ═══════════════════════════════════════════════════════
// 완료 상태 버튼
// ═══════════════════════════════════════════════════════

document.getElementById('btn-close-installer')?.addEventListener('click', () => appWindow.close());

$btnLaunch?.addEventListener('click', async () => {
    try {
        await invoke('launch_app');
    } catch (e) {
        // 조용히 닫지 않는다: 어떻게 실행하면 되는지 알려 준다
        $statusSub.textContent = tr('launchFailed');
        console.error('launch_app failed:', e);
        return;
    }
    appWindow.close();
});

// 실패 화면: 설정 시트로 돌아가 다시 설치
$btnRetry?.addEventListener('click', () => {
    $flowEl.classList.remove('failed');
    $warnings.hidden = true;
    $warnWrap.classList.remove('in');
    enterSettings();
});

// ═══════════════════════════════════════════════════════
// 언인스톨 모드
// ═══════════════════════════════════════════════════════

async function enterUninstallMode() {
    uninstallMode = true;
    currentState = 'uninstall';
    $flowEl.classList.add('no-art');
    document.getElementById('steps').hidden = true;
    document.getElementById('mode-chip').textContent = tr('modeUninstall');
    flow.reset('uninstall');

    try {
        const st = await invoke('get_installer_state');
        const info = document.getElementById('uninstall-info');
        if (info && st.install_path) {
            info.textContent = st.install_path;
            info.hidden = false;
        }
    } catch (_) {}
}

document.getElementById('btn-cancel')?.addEventListener('click', () => appWindow.close());

document.getElementById('btn-uninstall')?.addEventListener('click', async () => {
    if (currentState !== 'uninstall') return;
    currentState = 'uninstalling';
    setWorking(true);
    resetProgress();
    $statusText.textContent = tr('uninstalling');
    $statusSub.textContent = tr('uninstallSub');
    ring.set('spin');
    flow.go('install');
    matrix.start();

    const keepSettings = document.getElementById('chk-keep-settings')?.checked ?? true;
    const keepInstances = document.getElementById('chk-keep-instances')?.checked ?? true;
    try { await invoke('start_uninstall', { keepSettings, keepInstances }); } catch (e) {
        if (String(e) === 'busy') return;
        enterError({ message: String(e) }, 'uninstallFailed', false);
    }
});

listen('uninstall:progress', (ev) => {
    const p = ev.payload;

    if (p.step === 'error') {
        enterError(p, 'uninstallFailed', false);
        return;
    }

    if (p.step === 'complete') {
        currentState = 'complete';
        const left = p.left || [];
        // 정상 종료하지 못하고 강제로 끊은 서버는 지우지 못한 항목과 따로, 앞에 알린다
        const warnings = (p.warnings || []).map(uninstallWarning);
        setProgress(100, p.message);
        showResult(true, {
            title: tr(left.length ? 'uninstallCompletePartial' : 'uninstallComplete'),
            sub: tr(left.length ? 'uninstallCompletePartialSub' : 'uninstallCompleteSub'),
            lines: warnings.concat(left),
        });
        return;
    }

    setProgress(p.percent, trStep(UNINSTALL_STEP_KEYS, p.step, p.message));
});

// ═══════════════════════════════════════════════════════
// 테마 토글
// ═══════════════════════════════════════════════════════

function toggleTheme() {
    SabaUI.setTheme(SabaUI.effectiveTheme() === 'dark' ? 'light' : 'dark');
}

document.getElementById('btn-theme')?.addEventListener('click', toggleTheme);

// ═══════════════════════════════════════════════════════
// 커스텀 컨텍스트 메뉴
// ═══════════════════════════════════════════════════════

const $ctxMenu = document.getElementById('ctx-menu');
const $ctxOverlay = document.getElementById('ctx-overlay');

function closeContextMenu() {
    $ctxMenu.classList.remove('visible');
    $ctxOverlay.classList.remove('visible');
}

document.addEventListener('contextmenu', (e) => {
    // 입력 칸에서는 복사·붙여넣기 메뉴를 그대로 둔다
    if (e.target.closest('input')) return;
    e.preventDefault();
    $ctxMenu.style.top = `${Math.min(e.clientY, window.innerHeight - 100)}px`;
    $ctxMenu.style.left = `${Math.min(e.clientX, window.innerWidth - 170)}px`;
    $ctxMenu.classList.add('visible');
    $ctxOverlay.classList.add('visible');
});

$ctxOverlay.addEventListener('click', closeContextMenu);

document.getElementById('ctx-theme')?.addEventListener('click', () => {
    closeContextMenu();
    toggleTheme();
});

document.getElementById('ctx-lang')?.addEventListener('click', () => {
    closeContextMenu();
    showLangPopup();
});

// ═══════════════════════════════════════════════════════
// 언어 팝업
// ═══════════════════════════════════════════════════════

const $langPopup = document.getElementById('lang-popup');

function markActiveLang() {
    $langPopup.querySelectorAll('[data-lang]').forEach(el => {
        el.classList.toggle('active', el.dataset.lang === lang);
    });
}

/** 가운데에 띄운다 (오른쪽 클릭 메뉴에서) */
function showLangPopup() {
    $langPopup.classList.add('visible');
    const rect = $langPopup.getBoundingClientRect();
    $langPopup.style.top = `${(window.innerHeight - rect.height) / 2}px`;
    $langPopup.style.left = `${(window.innerWidth - rect.width) / 2}px`;
    markActiveLang();
}

function closeLangPopup() {
    $langPopup.classList.remove('visible');
}

document.addEventListener('click', (e) => {
    if ($langPopup.classList.contains('visible') && !$langPopup.contains(e.target) && !e.target.closest('#btn-lang')) {
        closeLangPopup();
    }
});

$langPopup.querySelectorAll('[data-lang]').forEach(el => {
    el.addEventListener('click', async () => {
        lang = el.dataset.lang;
        applyTranslations();
        try { await invoke('set_language', { language: lang }); } catch (_) {}
        closeLangPopup();
    });
});

// 언어 버튼 아래, 창 오른쪽 끝에 맞춰 띄운다
document.getElementById('btn-lang')?.addEventListener('click', (e) => {
    e.stopPropagation();
    if ($langPopup.classList.contains('visible')) { closeLangPopup(); return; }
    const rect = e.currentTarget.getBoundingClientRect();
    $langPopup.classList.add('visible');
    const popRect = $langPopup.getBoundingClientRect();
    $langPopup.style.left = `${Math.min(rect.left, window.innerWidth - popRect.width - 8)}px`;
    $langPopup.style.top = `${rect.bottom + 4}px`;
    markActiveLang();
});

// ═══════════════════════════════════════════════════════
// 초기화
// ═══════════════════════════════════════════════════════

(async function init() {
    // CSS가 적용된 후 윈도우를 표시하여 흰 화면 방지 (tauri.conf.json: visible=false)
    getCurrentWindow().show();

    // 테마 감지 (OS 설정 기본값)
    SabaUI.setTheme(SabaUI.systemTheme());

    // 언어: Rust가 앱 설정 → OS 언어 → 영어 순으로 골라 지원 언어 태그로 준다
    try {
        lang = await invoke('get_preferred_language');
    } catch (_) {}

    applyTranslations();

    // 감지된 언어를 Rust 백엔드에 동기화 (기본값 'en'이 아닌 실제 선택 언어를 설치에 반영)
    try { await invoke('set_language', { language: lang }); } catch (_) {}

    // 모드 확인
    try {
        const mode = await invoke('get_app_mode');
        if (mode.setup) {
            setupMode = true;
            moveFirst = !!mode.moveFirst;
            document.body.classList.add('setup');
            // 옮기라는 안내의 버튼은 닫기다 (다음 화살표 없이)
            if (moveFirst) document.querySelector('#btn-next svg')?.setAttribute('display', 'none');
            // 준비가 끝나면 앱을 연다: 설치기를 닫기만 하는 버튼은 두지 않는다
            document.getElementById('btn-close-installer').hidden = true;
            applyTranslations();
        }
        if (mode.uninstall) {
            // 처음 값은 인자에서 (`--remove-*`가 없으면 둘 다 남김)
            const check = (id, on) => { const el = document.getElementById(id); if (el) el.checked = on !== false; };
            check('chk-keep-settings', mode.keepSettings);
            check('chk-keep-instances', mode.keepInstances);
            enterUninstallMode();
            return;
        }
    } catch (_) {}

    enterWelcome();
    if (moveFirst) return;

    // 설치 경로 로드
    try {
        const st = await invoke('get_installer_state');
        if ($installPath) $installPath.value = st.install_path || '';
        if ($componentsPath) $componentsPath.value = st.components_dir || '';
        // 업데이트 채널: 지금 쓰는 채널(없으면 이 설치기의 채널)을 골라 두고, 모듈 목록도 그 채널에서 받는다
        if ($channel && !setupMode && st.channel) {
            $channel.value = st.channel;
            await invoke('set_update_channel', { channel: $channel.value });
        }
    } catch (_) {}

    // 모듈 로드
    await loadModules();
})();
