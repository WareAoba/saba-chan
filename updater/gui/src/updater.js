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
        statusUpToDate: 'Up to date',
        ringUpdating: 'Updating...',
        ringPleaseWait: 'Please wait...',
        ringComplete: 'Update complete!',
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
        applyFailedCount: '{{applied}} applied, {{failed}} failed or skipped',
        applyStartFailed: 'Apply failed: {{error}}',
    },
    ko: {
        appTitle: '사바쨩 업데이터',
        minimize: '최소화',
        close: '닫기',
        loadingTitle: '업데이트중!',
        loadingSub: '잠시만 기다려 주세요...!',
        statusUpToDate: '최신 상태',
        ringUpdating: '업데이트중!',
        ringPleaseWait: '잠시만 기다려 주세요...!',
        ringComplete: '업데이트 완료!',
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
        applyFailedCount: '{{applied}}개 적용, {{failed}}개 실패하거나 건너뜀',
        applyStartFailed: '적용 실패: {{error}}',
    },
    ja: {
        appTitle: 'Saba-chan アップデーター',
        minimize: '最小化',
        close: '閉じる',
        loadingTitle: '更新中...',
        loadingSub: 'しばらくお待ちください...',
        statusUpToDate: '最新',
        ringUpdating: '更新中...',
        ringPleaseWait: 'しばらくお待ちください...',
        ringComplete: '更新完了!',
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
        applyFailedCount: '{{applied}} 件適用、{{failed}} 件失敗またはスキップ',
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
            await enterApplyMode(applyMode);
            return;
        }
    } catch (e) {
        console.log('Apply mode check:', e);
    }

    // 업데이트 완료 후 재시작된 경우 알림 표시
    await checkAfterUpdate();
})();

// ═══════════════════════════════════════════════════════
// Apply Mode — --apply 실행 시 기존 GUI UI를 재활용
// ═══════════════════════════════════════════════════════

async function enterApplyMode(mode) {
    const { listen } = window.__TAURI__.event;

    // 프로그레스 링: 초기 "준비 중"
    showRing('spinning', tr('applyPreparing'), tr('applyPleaseWait'));

    // 프로그레스 바 표시
    showProgress(tr('applyLoadingManifest'), 0);

    // 진행 이벤트 리스닝 — 프로그레스 링 + 프로그레스 바 + 토스트 활용
    // 등록이 끝난 뒤에 적용을 시작한다: 먼저 시작하면 첫 이벤트(빠른 실패 포함)를 놓친다
    await listen('apply:progress', (event) => {
        const { step, message, percent, applied, failed } = event.payload;

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
            // message는 진단용 원문(영어)이라 화면에는 번역한 요약만 보인다
            console.warn('[Apply]', message);
            const summary = tr('applyFailedCount', { applied: applied ? applied.length : 0, failed: failed || 0 });
            showRing('error', tr('applyFailed'), summary);
            hideProgress();
            showToast(summary, 'error', 8000);
        }
    });

    // apply 실행
    invoke('start_apply').catch(err => {
        showRing('error', tr('applyFailed'), String(err));
        hideProgress();
        showToast(tr('applyStartFailed', { error: err }), 'error', 8000);
    });
}
