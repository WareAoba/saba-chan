// ═══════════════════════════════════════════════════════
// Saba-chan Updater GUI — 바닐라 JS 프론트엔드
// ═══════════════════════════════════════════════════════
// Tauri IPC로 백엔드와 통신합니다.

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

// SSOT: shared/constants.js, updater/src/constants.rs 와 동일 목록
// Tauri 웹뷰 환경이므로 CJS/ESM import 불가 → 인라인 선언 유지
const SUPPORTED_LANGUAGES = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

const TRANSLATIONS = {
    en: {
        appTitle: 'Saba-chan Updater',
        minimize: 'Minimize',
        close: 'Close',
        loadingTitle: 'Updating...',
        loadingSub: 'Please wait...',
        statusNotInstalled: 'Not installed',
        statusReadyToApply: 'Ready to apply',
        statusUpdateAvailable: 'Update available',
        statusUpToDate: 'Up to date',
        downloadTooltip: 'Download {{name}}',
        downloaded: 'Downloaded: {{name}}',
        failed: 'Failed: {{error}}',
        install: 'Install',
        installing: 'Installing {{name}}...',
        installed: 'Installed: {{name}}',
        installFailed: 'Install failed: {{error}}',
        ringUpdating: 'Updating...',
        ringPleaseWait: 'Please wait...',
        ringFailed: 'Update failed',
        ringComplete: 'Update complete!',
        ringAllLatest: 'All components are up to date',
        unknownScenario: 'Unknown scenario: {{name}}',
        scenarioErrorTitle: '❌ Scenario error',
        scenarioFailed: 'Scenario failed: {{error}}',
        testModeTitle: 'Self-update test',
        testModeSub: 'Simulating update process...',
        relaunchFailed: 'Relaunch failed: {{error}}',
        bannerUpdated: 'Update complete!',
        bannerComponentsUpdated: '{{count}} components updated',
        applyPreparing: 'Preparing to apply update...',
        applyPleaseWait: 'Please wait',
        applyLoadingManifest: 'Loading manifest...',
        applyManifestLoading: 'Loading manifest',
        applyApplyingFiles: 'Applying update files...',
        applyApplied: 'Applied',
        applyUpdated: 'Updated',
        applyUpdatedCount: '{{count}} components updated',
        applyRestartingGui: 'Restarting GUI...',
        applyFailed: 'Update failed',
        applyStartFailed: 'Apply failed: {{error}}',
    },
    ko: {
        appTitle: '사바쨩 업데이터',
        minimize: '최소화',
        close: '닫기',
        loadingTitle: '업데이트중!',
        loadingSub: '잠시만 기다려 주세요...!',
        statusNotInstalled: '설치되지 않음',
        statusReadyToApply: '적용 준비 완료',
        statusUpdateAvailable: '업데이트 가능',
        statusUpToDate: '최신 상태',
        downloadTooltip: '{{name}} 다운로드',
        downloaded: '다운로드 완료: {{name}}',
        failed: '실패: {{error}}',
        install: '설치',
        installing: '{{name}} 설치 중...',
        installed: '설치 완료: {{name}}',
        installFailed: '설치 실패: {{error}}',
        ringUpdating: '업데이트중!',
        ringPleaseWait: '잠시만 기다려 주세요...!',
        ringFailed: '업데이트 실패',
        ringComplete: '업데이트 완료!',
        ringAllLatest: '모든 컴포넌트가 최신 상태입니다',
        unknownScenario: '알 수 없는 시나리오: {{name}}',
        scenarioErrorTitle: '❌ 시나리오 오류',
        scenarioFailed: '시나리오 실패: {{error}}',
        testModeTitle: 'Self-Update 테스트',
        testModeSub: '업데이트 프로세스를 시뮬레이션합니다...',
        relaunchFailed: '재기동 실패: {{error}}',
        bannerUpdated: '업데이트 완료!',
        bannerComponentsUpdated: '{{count}}개 컴포넌트 업데이트 완료',
        applyPreparing: '업데이트 적용 준비 중…',
        applyPleaseWait: '잠시만 기다려 주세요',
        applyLoadingManifest: '매니페스트 로딩 중...',
        applyManifestLoading: '매니페스트 로딩',
        applyApplyingFiles: '업데이트 파일 적용 중…',
        applyApplied: '적용 완료',
        applyUpdated: '업데이트됨',
        applyUpdatedCount: '{{count}}개 컴포넌트 업데이트 완료',
        applyRestartingGui: 'GUI를 재시작합니다…',
        applyFailed: '업데이트 실패',
        applyStartFailed: '적용 실패: {{error}}',
    },
    ja: {
        appTitle: 'Saba-chan アップデーター',
        minimize: '最小化',
        close: '閉じる',
        loadingTitle: '更新中...',
        loadingSub: 'しばらくお待ちください...',
        statusNotInstalled: '未インストール',
        statusReadyToApply: '適用準備完了',
        statusUpdateAvailable: '更新あり',
        statusUpToDate: '最新',
        downloadTooltip: '{{name}} をダウンロード',
        downloaded: 'ダウンロード完了: {{name}}',
        failed: '失敗: {{error}}',
        install: 'インストール',
        installing: '{{name}} をインストール中...',
        installed: 'インストール完了: {{name}}',
        installFailed: 'インストール失敗: {{error}}',
        ringUpdating: '更新中...',
        ringPleaseWait: 'しばらくお待ちください...',
        ringFailed: '更新失敗',
        ringComplete: '更新完了!',
        ringAllLatest: 'すべてのコンポーネントは最新です',
        unknownScenario: '不明なシナリオ: {{name}}',
        scenarioErrorTitle: '❌ シナリオエラー',
        scenarioFailed: 'シナリオ失敗: {{error}}',
        testModeTitle: 'セルフアップデートテスト',
        testModeSub: '更新プロセスをシミュレーションします...',
        relaunchFailed: '再起動失敗: {{error}}',
        bannerUpdated: '更新完了!',
        bannerComponentsUpdated: '{{count}} コンポーネント更新完了',
        applyPreparing: '更新適用を準備中...',
        applyPleaseWait: 'しばらくお待ちください',
        applyLoadingManifest: 'マニフェスト読み込み中...',
        applyManifestLoading: 'マニフェスト読み込み',
        applyApplyingFiles: '更新ファイルを適用中...',
        applyApplied: '適用完了',
        applyUpdated: '更新済み',
        applyUpdatedCount: '{{count}} コンポーネント更新完了',
        applyRestartingGui: 'GUIを再起動しています...',
        applyFailed: '更新失敗',
        applyStartFailed: '適用失敗: {{error}}',
    },
};

let currentLanguage = 'en';

function normalizeLanguageTag(input) {
    if (!input || typeof input !== 'string') return 'en';
    const canonical = input.trim().replace('_', '-');
    const exact = SUPPORTED_LANGUAGES.find((lang) => lang.toLowerCase() === canonical.toLowerCase());
    if (exact) return exact;

    const lower = canonical.toLowerCase();
    if (lower.startsWith('pt')) return 'pt-BR';
    if (lower.startsWith('zh-cn') || lower.startsWith('zh-hans')) return 'zh-CN';
    if (lower.startsWith('zh-tw') || lower.startsWith('zh-hant')) return 'zh-TW';

    const base = lower.split('-')[0];
    if (base === 'ko') return 'ko';
    if (base === 'ja') return 'ja';
    if (base === 'es') return 'es';
    if (base === 'ru') return 'ru';
    if (base === 'de') return 'de';
    if (base === 'fr') return 'fr';
    return 'en';
}

function tr(key, vars = {}) {
    const langBundle = TRANSLATIONS[currentLanguage] || TRANSLATIONS.en;
    const template = langBundle[key] || TRANSLATIONS.en[key] || key;
    return template.replace(/\{\{(\w+)\}\}/g, (_, name) => String(vars[name] ?? ''));
}

function applyStaticTranslations() {
    document.documentElement.lang = currentLanguage;
    const title = tr('appTitle');
    document.title = title;
    // Tauri 윈도우 타이틀도 갱신 (setup에서 설정한 하드코딩 타이틀 덮어쓰기)
    try {
        const appWindow = getCurrentWindow();
        appWindow.setTitle(title);
    } catch (_) {}
    const minBtn = document.getElementById('btn-minimize');
    if (minBtn) minBtn.title = tr('minimize');
    if ($btnClose) $btnClose.title = tr('close');
    if ($ringTitle) $ringTitle.textContent = tr('loadingTitle');
    if ($ringSub) $ringSub.textContent = tr('loadingSub');
    if ($progressMsg) $progressMsg.textContent = tr('applyLoadingManifest');
}

async function initLocalization() {
    try {
        const preferredLanguage = await invoke('get_preferred_language');
        currentLanguage = normalizeLanguageTag(preferredLanguage);
    } catch (_) {
        currentLanguage = normalizeLanguageTag(navigator.language || 'en');
    }
    applyStaticTranslations();
}

// ─── DOM ────────────────────────────────────────────────

const $componentList = document.getElementById('component-list');
const $progressBar   = document.getElementById('progress-bar');
const $progressMsg   = document.getElementById('progress-message');
const $progressPct   = document.getElementById('progress-percent');
const $progressFill  = document.getElementById('progress-fill');
const $toastContainer = document.getElementById('toast-container');

// Progress ring elements
const $ring          = document.getElementById('progress-ring');
const $ringTitle     = document.getElementById('ring-title');
const $ringSub       = document.getElementById('ring-sub');
const $loadingScreen = document.getElementById('loading-screen');

// ─── 타이틀바 버튼 ─────────────────────────────────────

const $btnClose = document.getElementById('btn-close');

(function initTitleBarButtons() {
    const appWindow = getCurrentWindow();
    document.getElementById('btn-minimize')?.addEventListener('click', () => {
        appWindow.minimize();
    });
    $btnClose?.addEventListener('click', () => {
        appWindow.close();
    });
})();

/** 업데이트 실패/완료 시만 닫기 버튼 표시 */
function setCloseButtonVisible(visible) {
    if ($btnClose) $btnClose.style.display = visible ? '' : 'none';
}

// ─── State ──────────────────────────────────────────────

let state = {
    components: [],
    checking: false,
    lastCheck: null,
    error: null,
};

// ─── UI 업데이트 ────────────────────────────────────────

function updateState(result) {
    state = {
        components: result.components || [],
        checking: result.checking,
        lastCheck: result.last_check,
        error: result.error,
    };
    renderComponents();
    updateRingFromState();
}

// ─── 프로그레스 링 (glow ring) ──────────────────────────────────────────────

/**
 * 프로그레스 링 표시
 * @param {'spinning'|'complete'|'error'} ringState
 * @param {string} title
 * @param {string} [sub]
 */
function showRing(ringState, title, sub) {
    $loadingScreen.style.display = '';
    // glow ring 상태 클래스
    $ring.className = `loading-logo-container ${ringState === 'spinning' ? '' : ringState}`;
    $ringTitle.textContent = title || tr('ringUpdating');
    $ringSub.textContent = sub || tr('ringPleaseWait');    // 에러/완료 시 닫기 버튼 표시
    setCloseButtonVisible(ringState === 'error' || ringState === 'complete');}

function hideRing() {
    $loadingScreen.style.display = 'none';
}

function setRingProgress(percent) {
    // glow ring은 determinate 링이 아니므로 프로그레스 바로 대체
    // (percent를 필요로 하는 곳에서는 showProgress를 사용)
}

/** state에서 링 상태 자동 결정 */
function updateRingFromState() {
    if (state.error) {
        showRing('error', tr('ringFailed'), state.error);
    } else if (state.components.length > 0 && state.components.every(c => !c.update_available)) {
        showRing('complete', tr('ringComplete'), tr('ringAllLatest'));
    }
    // 그 외에는 스피닝 상태 유지
}

function renderComponents() {
    $componentList.querySelectorAll('.component-card').forEach(el => el.remove());

    if (state.components.length === 0) {
        $componentList.style.display = 'none';
        return;
    }

    $componentList.style.display = '';
    for (const comp of state.components) {
        const card = createComponentCard(comp);
        $componentList.appendChild(card);
    }
}

function createComponentCard(comp) {
    const card = document.createElement('div');
    card.className = 'component-card';

    // 배지
    let badgeClass, badgeIcon;
    if (!comp.installed) {
        badgeClass = 'not-installed';
        badgeIcon = '✗';
    } else if (comp.downloaded) {
        badgeClass = 'downloaded';
        badgeIcon = '↓';
    } else if (comp.update_available) {
        badgeClass = 'update-available';
        badgeIcon = '⬆';
    } else {
        badgeClass = 'up-to-date';
        badgeIcon = '✓';
    }

    // 버전 텍스트
    let versionHtml = `v${comp.current_version}`;
    if (comp.latest_version && comp.update_available) {
        versionHtml += ` <span class="arrow">→</span> v${comp.latest_version}`;
    }

    // 상태 배지
    let statusClass, statusText;
    if (!comp.installed) {
        statusClass = 'missing';
        statusText = tr('statusNotInstalled');
    } else if (comp.downloaded) {
        statusClass = 'ready';
        statusText = tr('statusReadyToApply');
    } else if (comp.update_available) {
        statusClass = 'update';
        statusText = tr('statusUpdateAvailable');
    } else {
        statusClass = 'current';
        statusText = tr('statusUpToDate');
    }

    card.innerHTML = `
        <div class="component-badge ${badgeClass}">${badgeIcon}</div>
        <div class="component-info">
            <div class="component-name">${escapeHtml(comp.display_name)}</div>
            <div class="component-version">${versionHtml}</div>
        </div>
        <span class="component-status-badge ${statusClass}">${statusText}</span>
    `;

    // 개별 다운로드 버튼 (업데이트 있고 아직 다운로드 안 된 경우)
    if (comp.update_available && !comp.downloaded) {
        const btn = document.createElement('button');
        btn.className = 'component-action btn-primary';
        btn.textContent = '↓';
        btn.title = tr('downloadTooltip', { name: comp.display_name });
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            btn.disabled = true;
            try {
                await invoke('download_component', { key: comp.key });
                showToast(tr('downloaded', { name: comp.display_name }), 'success');
                const result = await invoke('get_status');
                updateState(result);
            } catch (err) {
                showToast(tr('failed', { error: err }), 'error');
                btn.disabled = false;
            }
        });
        card.appendChild(btn);
    }

    // 개별 설치 버튼 (미설치인 경우)
    if (!comp.installed) {
        const btn = document.createElement('button');
        btn.className = 'component-action btn-danger';
        btn.textContent = tr('install');
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            btn.disabled = true;
            showProgress(tr('installing', { name: comp.display_name }), -1);
            try {
                await invoke('install_component', { key: comp.key });
                showToast(tr('installed', { name: comp.display_name }), 'success');
                hideProgress();
                const result = await invoke('get_status');
                updateState(result);
            } catch (err) {
                showToast(tr('installFailed', { error: err }), 'error');
                hideProgress();
                btn.disabled = false;
            }
        });
        card.appendChild(btn);
    }

    return card;
}

// ─── 프로그레스 바 (기존 GUI 패턴) ──────────────────────

function showProgress(message, percent) {
    $progressBar.style.display = '';
    $progressMsg.textContent = message;
    if (percent < 0) {
        // indeterminate
        $progressFill.className = 'global-progress-fill indeterminate';
        $progressPct.textContent = '';
    } else if (percent >= 100) {
        $progressFill.className = 'global-progress-fill complete';
        $progressFill.style.width = '100%';
        $progressPct.textContent = '100%';
    } else {
        $progressFill.className = 'global-progress-fill';
        $progressFill.style.width = `${percent}%`;
        $progressPct.textContent = `${Math.round(percent)}%`;
    }
}

function hideProgress() {
    $progressBar.style.display = 'none';
    $progressFill.className = 'global-progress-fill';
    $progressFill.style.width = '0%';
}

// ─── Toast (기존 GUI 패턴) ──────────────────────────────

function showToast(message, type = 'info', duration = 3000) {
    const toast = document.createElement('div');
    toast.className = `toast toast-${type}`;

    const iconMap = {
        success: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"></path><polyline points="22 4 12 14.01 9 11.01"></polyline></svg>',
        error: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"></circle><line x1="15" y1="9" x2="9" y2="15"></line><line x1="9" y1="9" x2="15" y2="15"></line></svg>',
        warning: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="8" x2="12" y2="12"></line><line x1="12" y1="16" x2="12.01" y2="16"></line></svg>',
        info: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"></circle><line x1="12" y1="16" x2="12" y2="12"></line><line x1="12" y1="8" x2="12.01" y2="8"></line></svg>',
    };

    toast.innerHTML = `
        <span class="toast-icon">${iconMap[type] || iconMap.info}</span>
        <span class="toast-message">${escapeHtml(message)}</span>
    `;

    toast.addEventListener('click', () => removeToast(toast));
    $toastContainer.appendChild(toast);

    if (duration > 0) {
        setTimeout(() => removeToast(toast), duration);
    }
}

function removeToast(toast) {
    if (!toast.parentNode) return;
    toast.classList.add('toast-removing');
    setTimeout(() => toast.remove(), 300);
}

// ─── 유틸 ───────────────────────────────────────────────

function escapeHtml(str) {
    const div = document.createElement('div');
    div.textContent = str;
    return div.innerHTML;
}

// ─── 테스트 시나리오 모드 ───────────────────────────────
// 프론트엔드에서 개별 Tauri 커맨드를 단계별 호출하여
// 각 진행 상태를 프로그레스 링 + 프로그레스 바 + 토스트로 실시간 표시합니다.

async function runScenario(scenarioName) {

    try {
        switch (scenarioName) {
            case 'fetch':
                await scenarioFetch();
                break;
            case 'download_apply':
                await scenarioDownloadApply();
                break;
            case 'queue':
                await scenarioQueue();
                break;
            case 'error':
                await scenarioError();
                break;
            default:
                showToast(tr('unknownScenario', { name: scenarioName }), 'error');
        }
    } catch (e) {
        hideProgress();
        showRing('error', tr('scenarioErrorTitle'), String(e));
        showToast(tr('scenarioFailed', { error: e }), 'error');
    }


}

// ── 시나리오 1: 버전 페치 ───────────────────────────────
// Mock 서버에서 최신 릴리스 정보를 가져와 컴포넌트 목록을 표시합니다.
async function scenarioFetch() {
    // Step 1: 체크 시작
    showRing('spinning', '🔍 업데이트 확인 중...', 'Mock 서버에서 최신 릴리스 정보를 가져오는 중');
    showProgress('업데이트 확인 중...', -1);
    await sleep(500);

    // Step 2: check_updates 호출
    const result = await invoke('check_updates');
    updateState(result);

    // Step 3: 결과 표시
    hideProgress();
    const updates = (result.components || []).filter(c => c.update_available);
    const total = (result.components || []).length;

    if (updates.length > 0) {
        showRing('has-updates', `✅ 버전 페치 완료 — ${updates.length}개 업데이트 발견`,
            `전체 ${total}개 컴포넌트 중 ${updates.length}개 업데이트 가능`);
        showToast(`${updates.length}개 업데이트 발견 (전체 ${total}개)`, 'success');
    } else {
        showRing('complete', '✅ 버전 페치 완료 — 최신 상태', `전체 ${total}개 컴포넌트 확인됨`);
        showToast('모든 컴포넌트가 최신 버전입니다', 'info');
    }
}

// ── 시나리오 2: 다운로드 → 적용 ─────────────────────────
// check → download_all → apply_updates 전체 플로우를 단계별로 보여줍니다.
async function scenarioDownloadApply() {
    // Step 1/3: 업데이트 확인
    showRing('spinning', '⏳ [1/3] 업데이트 확인 중...', 'Mock 서버에서 릴리스 정보를 가져옵니다');
    showProgress('업데이트 확인 중...', 10);
    await sleep(400);

    const checkResult = await invoke('check_updates');
    updateState(checkResult);

    const updates = (checkResult.components || []).filter(c => c.update_available);
    if (updates.length === 0) {
        hideProgress();
        showRing('complete', '⚠️ 업데이트 없음', 'Mock 서버가 실행 중인지, 릴리스 데이터가 있는지 확인하세요');
        showToast('업데이트할 컴포넌트가 없습니다', 'warning');
        return;
    }

    showToast(`${updates.length}개 업데이트 발견, 다운로드를 시작합니다`, 'info');
    showProgress(`${updates.length}개 업데이트 발견`, 25);
    await sleep(600);

    // Step 2/3: 다운로드
    showRing('spinning', `⏳ [2/3] ${updates.length}개 컴포넌트 다운로드 중...`,
        updates.map(c => c.display_name).join(', '));
    showProgress('다운로드 중...', 40);
    await sleep(300);

    const downloaded = await invoke('download_all');
    showProgress(`다운로드 완료: ${downloaded.length}개`, 65);
    showToast(`다운로드 완료: ${downloaded.join(', ')}`, 'success');

    // 상태 갱신 — 카드에 "Ready to apply" 배지 반영
    const afterDl = await invoke('get_status');
    updateState(afterDl);
    await sleep(600);

    // Step 3/3: 적용
    showRing('spinning', '⏳ [3/3] 업데이트 파일 적용 중...', '다운로드된 파일을 설치 경로에 배포합니다');
    showProgress('파일 적용 중...', 80);
    await sleep(300);

    const applied = await invoke('apply_updates');
    showProgress('완료!', 100);

    // 최종 상태 갱신
    const afterApply = await invoke('get_status');
    updateState(afterApply);

    showRing('complete', '✅ 업데이트 완료', `다운로드 ${downloaded.length}개 → 적용 ${applied.length}개`);
    showToast(`업데이트 완료: ${applied.length}개 컴포넌트 적용됨`, 'success', 5000);

    // relaunch 설정이 있으면 saba-chan GUI 재기동 시도
    try {
        const testMode = await invoke('get_test_mode');
        if (testMode.relaunch_cmd) {
            await sleep(1500);
            showRing('spinning', '✅ 업데이트 완료', 'saba-chan GUI 재기동 중...');
            showProgress('GUI 재기동 중...', 100);
            await invoke('relaunch');
        }
    } catch (_) { /* relaunch 미설정 시 무시 */ }

    hideProgress();
}

// ── 시나리오 3: 큐 처리 (개별 다운로드) ─────────────────
// 다수 컴포넌트를 한 번에 download_all이 아닌,
// download_component를 하나씩 호출하며 큐 처리를 테스트합니다.
async function scenarioQueue() {
    // Step 1: 업데이트 확인
    showRing('spinning', '🔍 큐 처리 테스트 — 업데이트 확인 중...', 'Mock 서버에서 컴포넌트 목록을 가져옵니다');
    showProgress('업데이트 확인 중...', -1);
    await sleep(400);

    const checkResult = await invoke('check_updates');
    updateState(checkResult);

    const updates = (checkResult.components || []).filter(c => c.update_available);
    if (updates.length === 0) {
        hideProgress();
        showRing('complete', '⚠️ 업데이트 없음', 'Mock 서버가 실행 중인지 확인하세요');
        showToast('큐 테스트를 위한 업데이트가 없습니다', 'warning');
        return;
    }

    showToast(`${updates.length}개 컴포넌트를 개별 다운로드합니다`, 'info');
    await sleep(300);

    // Step 2: 각 컴포넌트 개별 다운로드
    const results = [];
    for (let i = 0; i < updates.length; i++) {
        const comp = updates[i];
        const progress = Math.round(((i) / updates.length) * 80) + 10;

        showRing('spinning', `⏳ 큐 [${i + 1}/${updates.length}] ${comp.display_name}`,
            `${comp.current_version} → ${comp.latest_version}`);
        showProgress(`다운로드 중: ${comp.display_name}`, progress);

        try {
            const msg = await invoke('download_component', { key: comp.key });
            results.push({ key: comp.key, name: comp.display_name, ok: true, msg });
            showToast(`✅ ${comp.display_name} 다운로드 완료`, 'success', 2000);
        } catch (err) {
            results.push({ key: comp.key, name: comp.display_name, ok: false, error: String(err) });
            showToast(`❌ ${comp.display_name} 실패: ${err}`, 'error', 4000);
        }

        // 상태 갱신 — 각 컴포넌트 다운로드 후 카드 반영
        const interim = await invoke('get_status');
        updateState(interim);
        await sleep(400);
    }

    // Step 3: 결과 요약
    const okCount = results.filter(r => r.ok).length;
    const failCount = results.filter(r => !r.ok).length;

    showProgress('큐 처리 완료!', 100);
    const ringState = failCount > 0 ? 'error' : 'complete';
    const ringTitle = failCount > 0
        ? `⚠️ 큐 처리 완료 (${failCount}개 실패)`
        : `✅ 큐 처리 완료 — ${okCount}개 성공`;
    const ringSub = results.map(r => `${r.name}: ${r.ok ? '✅' : '❌'}`).join(' | ');
    showRing(ringState, ringTitle, ringSub);

    showToast(`큐 처리 완료: 성공 ${okCount}, 실패 ${failCount}`, failCount > 0 ? 'warning' : 'success', 5000);

    await sleep(1000);
    hideProgress();
}

// ── 시나리오 4: 에러/예외처리 ───────────────────────────
// 일부러 잘못된 URL을 설정하여 API 호출 실패를 발생시키고,
// 에러가 정상적으로 catch 및 UI에 표시되는지 테스트합니다.
async function scenarioError() {
    // Step 1: 현재 api_base_url 저장
    let originalConfig;
    try {
        originalConfig = await invoke('get_config');
    } catch (_) {
        originalConfig = {};
    }
    const originalUrl = originalConfig.api_base_url || null;

    showRing('spinning', '🧪 에러 테스트 — 잘못된 URL 설정 중...',
        'API 엔드포인트를 존재하지 않는 서버로 변경합니다');
    showProgress('에러 시나리오 준비 중...', 20);
    await sleep(500);

    // Step 2: 잘못된 URL 설정
    await invoke('set_api_base_url', { url: 'http://127.0.0.1:1' });
    showToast('API URL → http://127.0.0.1:1 (존재하지 않는 서버)', 'warning', 3000);
    showProgress('잘못된 URL로 업데이트 확인 시도 중...', 50);
    await sleep(400);

    // Step 3: check_updates 호출 — 실패를 기대
    showRing('spinning', '🧪 에러 테스트 — 업데이트 확인 중...',
        '의도적 실패: 연결 불가능한 서버로 요청');

    let errorCaught = null;
    try {
        await invoke('check_updates');
        errorCaught = '(예상 외 성공 — 에러가 발생하지 않았습니다)';
    } catch (e) {
        errorCaught = String(e);
    }

    showProgress('에러 감지 완료', 80);
    await sleep(300);

    // Step 4: 원래 URL 복원
    showRing('spinning', '🧪 에러 테스트 — 설정 복원 중...',
        'API URL을 원래 값으로 되돌립니다');
    await invoke('set_api_base_url', { url: originalUrl });
    showToast('API URL 복원 완료', 'info', 2000);
    showProgress('설정 복원 완료', 100);
    await sleep(300);

    // Step 5: 결과 표시
    hideProgress();
    if (errorCaught && !errorCaught.includes('예상 외 성공')) {
        showRing('complete', '✅ 에러 테스트 통과', `에러 정상 감지: ${errorCaught}`);
        showToast(`에러가 정상적으로 감지되었습니다`, 'success', 5000);
    } else {
        showRing('error', '❌ 에러 테스트 실패', errorCaught || '에러가 발생하지 않았습니다');
        showToast('에러가 발생하지 않아 테스트 실패', 'error', 5000);
    }
}

// 기존 --test 모드 (self-update 시뮬레이션) 유지
async function runTestMode() {
    // 상태 표시
    showRing('spinning', tr('testModeTitle'), tr('testModeSub'));

    // 가짜 컴포넌트 카드 표시
    $componentList.querySelectorAll('.component-card').forEach(el => el.remove());
    const fakeComponents = [
        { display_name: 'saba-chan GUI', key: 'gui', current_version: '0.1.0', latest_version: '0.1.1' },
        { display_name: 'saba-chan CLI', key: 'cli', current_version: '0.1.0', latest_version: '0.1.1' },
    ];
    for (const comp of fakeComponents) {
        const card = createComponentCard({
            ...comp,
            installed: true,
            update_available: true,
            downloaded: false,
        });
        $componentList.appendChild(card);
        // 버튼 비활성화
        card.querySelectorAll('button').forEach(b => { b.disabled = true; });
    }

    const steps = [
        { msg: '업데이트 확인 중...', duration: 800 },
        { msg: 'saba-chan GUI v0.1.1 발견', duration: 600 },
        { msg: 'saba-chan CLI v0.1.1 발견', duration: 400 },
        { msg: '다운로드 중... (1/2)', duration: 1000 },
        { msg: '다운로드 중... (2/2)', duration: 1000 },
        { msg: '업데이트 적용 중...', duration: 800 },
        { msg: '완료! GUI를 재시작합니다...', duration: 1200 },
    ];

    let progress = 0;
    const stepIncrement = 100 / steps.length;

    for (const step of steps) {
        showProgress(step.msg, progress);
        await sleep(step.duration);
        progress += stepIncrement;

        // 다운로드 완료 후 카드 갱신
        if (step.msg.includes('2/2')) {
            $componentList.querySelectorAll('.component-card').forEach(el => el.remove());
            for (const comp of fakeComponents) {
                const card = createComponentCard({
                    ...comp,
                    installed: true,
                    update_available: true,
                    downloaded: true,
                });
                $componentList.appendChild(card);
                card.querySelectorAll('button').forEach(b => { b.disabled = true; });
            }
        }
    }

    showProgress('GUI 재기동 중...', 100);

    // 상태 업데이트
    showRing('complete', '업데이트 완료', 'saba-chan GUI를 재시작합니다...');

    await sleep(500);

    // saba-chan GUI 재기동 후 updater 종료
    try {
        await invoke('relaunch');
    } catch (e) {
        showToast(tr('relaunchFailed', { error: e }), 'error');
    }
}

function sleep(ms) {
    return new Promise(resolve => setTimeout(resolve, ms));
}

// ─── 업데이트 완료 후 알림 표시 ─────────────────────────

async function checkAfterUpdate() {
    try {
        const info = await invoke('check_after_update');
        if (info.updated && info.components.length > 0) {
            // 업데이트 완료 배너 표시
            const banner = document.createElement('div');
            banner.className = 'update-complete-banner';
            banner.innerHTML = `
                <div class="update-complete-icon">✓</div>
                <div class="update-complete-content">
                    <div class="update-complete-title">${escapeHtml(tr('bannerUpdated'))}</div>
                    <div class="update-complete-list">${info.components.join(', ')}</div>
                </div>
                <button class="update-complete-close" onclick="this.parentElement.remove()">✕</button>
            `;
            document.body.prepend(banner);
            
            // 토스트도 표시
            showToast(tr('bannerComponentsUpdated', { count: info.components.length }), 'success', 5000);
            
            // 5초 후 배너 자동 제거
            setTimeout(() => banner.remove(), 8000);
        } else if (info.message) {
            // 실패 메시지
            showToast(info.message, 'error', 5000);
        }
    } catch (e) {
        console.log('After update check:', e);
    }
}

// ─── 초기 로드 ──────────────────────────────────────────

(async function init() {
    // 테마 적용 — settings.json에서 읽어 CSS data-theme 속성에 반영
    try {
        const theme = await invoke('get_theme');
        if (theme && ['light', 'dark', 'auto'].includes(theme)) {
            document.body.setAttribute('data-theme', theme);
        }
    } catch (_) {
        // 테마 로드 실패 시 기본 auto (CSS media query가 처리)
    }

    // 윈도우 표시는 Rust setup에서 처리 — JS에서는 포커스만 비동기로 시도
    // (await appWindow.show()가 특정 환경에서 블로킹되는 Tauri v2 이슈 회피)
    const appWindow = getCurrentWindow();
    appWindow.show()
        .then(() => appWindow.setAlwaysOnTop(true))
        .then(() => appWindow.setFocus())
        .then(() => setTimeout(() => appWindow.setAlwaysOnTop(false).catch(() => {}), 800))
        .catch(() => {});

    await initLocalization();

    // 1. Apply 모드 확인 (--apply로 실행된 경우)
    try {
        const applyMode = await invoke('get_apply_mode');
        if (applyMode.enabled) {
            enterApplyMode(applyMode);
            return;
        }
    } catch (e) {
        console.log('Apply mode check:', e);
    }

    // 업데이트 완료 후 재시작된 경우 알림 표시
    await checkAfterUpdate();
    
    try {
        // 테스트 모드 확인
        const testMode = await invoke('get_test_mode');
        if (testMode.scenario) {
            // 시나리오 모드: 실제 Tauri 명령으로 E2E 실행
            runScenario(testMode.scenario);
            return;
        }
        if (testMode.enabled) {
            // 기존 self-update 시뮬레이션
            runTestMode();
            return;
        }
    } catch (e) {
        console.log('Test mode check:', e);
    }



    try {
        const result = await invoke('get_status');
        updateState(result);
    } catch (e) {
        // 첫 실행 시 상태가 비어 있을 수 있음
        console.log('Initial load:', e);
    }
})();

// ═══════════════════════════════════════════════════════
// Apply Mode — --apply 실행 시 기존 GUI UI를 재활용
// ═══════════════════════════════════════════════════════

function enterApplyMode(mode) {
    const { listen } = window.__TAURI__.event;

    // 프로그레스 링: 초기 "준비 중"
    showRing('spinning', tr('applyPreparing'), tr('applyPleaseWait'));

    // 프로그레스 바 표시
    showProgress(tr('applyLoadingManifest'), 0);

    // 진행 이벤트 리스닝 — 프로그레스 링 + 프로그레스 바 + 토스트 활용
    listen('apply:progress', (event) => {
        const { step, message, percent, applied } = event.payload;

        let localizedMessage = message;
        if (step === 'manifest') {
            localizedMessage = tr('applyLoadingManifest');
        } else if (step === 'applying') {
            localizedMessage = tr('applyApplyingFiles');
        } else if (step === 'complete') {
            localizedMessage = applied && applied.length > 0
                ? tr('applyUpdatedCount', { count: applied.length })
                : tr('statusUpToDate');
        }

        // 프로그레스 바
        if (percent >= 0) {
            showProgress(localizedMessage, percent);
        }

        // 프로그레스 링 업데이트
        if (step === 'manifest') {
            showRing('spinning', tr('applyManifestLoading'), localizedMessage);
        } else if (step === 'applying') {
            showRing('spinning', tr('applyApplyingFiles'), localizedMessage);
        } else if (step === 'complete') {
            showRing('complete', tr('ringComplete'), localizedMessage);

            // 적용된 컴포넌트를 카드로 표시
            $componentList.querySelectorAll('.component-card').forEach(el => el.remove());
            if (applied && applied.length > 0) {
                for (const name of applied) {
                    const card = document.createElement('div');
                    card.className = 'component-card';
                    card.innerHTML = `
                        <div class="component-badge up-to-date">✓</div>
                        <div class="component-info">
                            <div class="component-name">${escapeHtml(name)}</div>
                            <div class="component-version">${escapeHtml(tr('applyApplied'))}</div>
                        </div>
                        <span class="component-status-badge current">${escapeHtml(tr('applyUpdated'))}</span>
                    `;
                    $componentList.appendChild(card);
                }
                showToast(tr('applyUpdatedCount', { count: applied.length }), 'success', 5000);
            }

            // 재기동 안내
            if (mode.relaunch) {
                setTimeout(() => {
                    showRing('spinning', tr('ringComplete'), tr('applyRestartingGui'));
                    showProgress(tr('applyRestartingGui'), 100);
                }, 1500);
            }
        } else if (step === 'error') {
            showRing('error', tr('applyFailed'), message);
            hideProgress();
            showToast(message, 'error', 8000);
        }
    });

    // apply 실행
    invoke('start_apply').catch(err => {
        showRing('error', tr('applyFailed'), String(err));
        hideProgress();
        showToast(tr('applyStartFailed', { error: err }), 'error', 8000);
    });
}
