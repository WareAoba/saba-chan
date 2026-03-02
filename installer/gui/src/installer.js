// ═══════════════════════════════════════════════════════
// Saba-chan Installer — Bottom-Sheet Wizard
// ═══════════════════════════════════════════════════════
// 흐름: Welcome → (시트 올라옴) Settings → (시트 내려감 + 링 회전) Installing → (초록) Complete

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

// ═══════════════════════════════════════════════════════
// i18n
// ═══════════════════════════════════════════════════════

const T = {
    en: {
        welcome: 'Welcome to Saba-chan Installer',
        btnNext: 'Next',
        labelPath: 'Install Location',
        labelModules: 'Game Modules',
        hintModules: 'You can add more later.',
        labelOptions: 'Options',
        labelDesktop: 'Desktop Shortcut',
        labelStartMenu: 'Start Menu Shortcut',
        btnInstall: 'Install',
        installing: 'Installing...',
        installSub: 'Please wait...',
        preparing: 'Preparing...',
        fetchingRelease: 'Checking for the latest release...',
        completeTitle: 'Installation Complete!',
        completeSub: 'Saba-chan is ready.',
        btnOpenFolder: 'Open Folder',
        btnFinish: 'Finish',
        installFailed: 'Installation failed',
        fetchFailed: 'Failed to fetch releases',
        uninstallTitle: 'Uninstall Saba-chan',
        uninstallDesc: 'This will completely remove Saba-chan, including all data, settings, and registry entries.',
        uninstallPath: 'Install location: {{path}}',
        btnCancel: 'Cancel',
        btnUninstall: 'Uninstall',
        uninstalling: 'Uninstalling...',
        uninstallSub: 'Please wait...',
        uninstallComplete: 'Uninstall complete!',
        uninstallCompleteSub: 'Saba-chan has been removed.',
        uninstallFailed: 'Uninstall failed',
        labelLanguage: 'Language',
    },
    ko: {
        welcome: '사바쨩 인스톨러에 오신 것을 환영합니다',
        btnNext: '다음',
        labelPath: '설치 경로',
        labelModules: '게임 모듈',
        hintModules: '나중에 추가할 수도 있어요.',
        labelOptions: '옵션',
        labelDesktop: '바탕화면 바로가기',
        labelStartMenu: '시작 메뉴 바로가기',
        btnInstall: '설치',
        installing: '설치 중!',
        installSub: '잠시만 기다려 주세요...!',
        preparing: '준비 중...',
        fetchingRelease: '최신 릴리즈 확인 중...',
        completeTitle: '설치 완료!',
        completeSub: '사바쨩이 준비되었습니다.',
        btnOpenFolder: '폴더 열기',
        btnFinish: '완료',
        installFailed: '설치 실패',
        fetchFailed: '릴리즈 정보를 가져올 수 없습니다',
        uninstallTitle: '사바쨩 제거',
        uninstallDesc: '사바쨩을 완전히 제거합니다. 모든 데이터, 설정, 레지스트리 항목이 삭제됩니다.',
        uninstallPath: '설치 위치: {{path}}',
        btnCancel: '취소',
        btnUninstall: '제거',
        uninstalling: '제거 중!',
        uninstallSub: '잠시만 기다려 주세요...!',
        uninstallComplete: '제거 완료!',
        uninstallCompleteSub: '사바쨩이 제거되었습니다.',
        uninstallFailed: '제거 실패',
    },
    ja: {
        welcome: 'Saba-chan インストーラーへようこそ',
        btnNext: '次へ',
        labelPath: 'インストール先',
        labelModules: 'ゲームモジュール',
        hintModules: '後から追加もできます。',
        labelOptions: 'オプション',
        labelDesktop: 'デスクトップショートカット',
        labelStartMenu: 'スタートメニューショートカット',
        btnInstall: 'インストール',
        installing: 'インストール中...',
        installSub: 'しばらくお待ちください...',
        preparing: '準備中...',
        fetchingRelease: '最新リリースを確認中...',
        completeTitle: 'インストール完了!',
        completeSub: 'Saba-chanの準備ができました。',
        btnOpenFolder: 'フォルダを開く',
        btnFinish: '完了',
        installFailed: 'インストール失敗',
        fetchFailed: 'リリース情報の取得に失敗',
        uninstallTitle: 'Saba-chan アンインストール',
        uninstallDesc: 'Saba-chanを完全に削除します。すべてのデータ、設定、レジストリが削除されます。',
        uninstallPath: 'インストール先: {{path}}',
        btnCancel: 'キャンセル',
        btnUninstall: 'アンインストール',
        uninstalling: 'アンインストール中...',
        uninstallSub: 'しばらくお待ちください...',
        uninstallComplete: 'アンインストール完了!',
        uninstallCompleteSub: 'Saba-chanは削除されました。',
        uninstallFailed: 'アンインストール失敗',
        labelLanguage: '言語',
    },
};

let lang = 'en';

function tr(key, vars = {}) {
    const b = T[lang] || T.en;
    const t = b[key] || T.en[key] || key;
    return t.replace(/\{\{(\w+)\}\}/g, (_, k) => String(vars[k] ?? ''));
}

function applyTranslations() {
    document.documentElement.lang = lang;
    const s = (id, key) => { const el = document.getElementById(id); if (el) el.textContent = tr(key); };
    s('status-text', 'welcome');
    s('btn-next-text', 'btnNext');
    s('label-path', 'labelPath');
    s('label-modules', 'labelModules');
    s('hint-modules', 'hintModules');
    s('label-options', 'labelOptions');
    s('label-desktop', 'labelDesktop');
    s('label-startmenu', 'labelStartMenu');
    s('btn-install-text', 'btnInstall');
    s('btn-open-folder-text', 'btnOpenFolder');
    s('btn-finish-text', 'btnFinish');
    s('uninstall-title', 'uninstallTitle');
    s('uninstall-desc', 'uninstallDesc');
    s('btn-cancel-text', 'btnCancel');
    s('btn-uninstall-text', 'btnUninstall');
}

// ═══════════════════════════════════════════════════════
// DOM refs
// ═══════════════════════════════════════════════════════

const $glowRing = document.getElementById('glow-ring');
const $logoArea = document.getElementById('logo-area');
const $statusText = document.getElementById('status-text');
const $statusSub = document.getElementById('status-sub');
const $mainStage = document.getElementById('main-stage');

const $sheet = document.getElementById('bottom-sheet');
const $overlay = document.getElementById('sheet-overlay');
const $uninstallSheet = document.getElementById('uninstall-sheet');

const $btnNext = document.getElementById('btn-next');
// toolbar-float is now in the title-bar, always visible
const $completeActions = document.getElementById('complete-actions');
const $progressBar = document.getElementById('progress-bar');

const $installPath = document.getElementById('install-path');

// ═══════════════════════════════════════════════════════
// 타이틀바
// ═══════════════════════════════════════════════════════

const appWindow = getCurrentWindow();
document.getElementById('btn-minimize')?.addEventListener('click', () => appWindow.minimize());
document.getElementById('btn-close')?.addEventListener('click', () => appWindow.close());

// ═══════════════════════════════════════════════════════
// Toast
// ═══════════════════════════════════════════════════════

const $toast = document.getElementById('toast-container');

function showToast(msg, type = 'info', dur = 3000) {
    const el = document.createElement('div');
    el.className = `toast toast-${type}`;
    const icons = {
        success: '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>',
        error:   '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/></svg>',
        info:    '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><line x1="12" y1="16" x2="12" y2="12"/><line x1="12" y1="8" x2="12.01" y2="8"/></svg>',
    };
    el.innerHTML = `<span class="toast-icon">${icons[type] || icons.info}</span><span class="toast-message">${esc(msg)}</span>`;
    el.addEventListener('click', () => { el.classList.add('toast-removing'); setTimeout(() => el.remove(), 250); });
    $toast.appendChild(el);
    if (dur > 0) setTimeout(() => { if (el.parentNode) { el.classList.add('toast-removing'); setTimeout(() => el.remove(), 250); } }, dur);
}

function esc(s) { const d = document.createElement('div'); d.textContent = s; return d.innerHTML; }

// ═══════════════════════════════════════════════════════
// 바텀 시트 제어
// ═══════════════════════════════════════════════════════

function openSheet(sheet) {
    $overlay.classList.add('visible');
    sheet.classList.add('open');
    $mainStage.classList.add('pushed-up');
    $btnNext.classList.add('hidden');
}

function closeSheet(sheet) {
    $overlay.classList.remove('visible');
    sheet.classList.remove('open');
    $mainStage.classList.remove('pushed-up');
}

// 오버레이 클릭 → 시트 닫기 (설치 중이 아닐 때만)
$overlay.addEventListener('click', () => {
    if (currentState === 'settings') {
        closeSheet($sheet);
        $btnNext.classList.remove('hidden');
        currentState = 'welcome';
    }
});

// ═══════════════════════════════════════════════════════
// 상태 기계
// ═══════════════════════════════════════════════════════
// welcome → settings (시트 올라옴) → installing (시트 내려감, 링 회전) → complete (초록)

let currentState = 'welcome';

function enterWelcome() {
    currentState = 'welcome';
    $glowRing.className = 'loading-logo-container idle';
    $logoArea.className = 'logo-area';
    $statusText.textContent = tr('welcome');
    $statusSub.textContent = '';
    $btnNext.classList.remove('hidden');
    $completeActions.style.display = 'none';
    $progressBar.style.display = 'none';
    closeSheet($sheet);
}

function enterSettings() {
    currentState = 'settings';
    openSheet($sheet);
}

function enterInstalling() {
    currentState = 'installing';
    closeSheet($sheet);
    $logoArea.className = 'logo-area state-installing';
    $glowRing.className = 'loading-logo-container spinning';
    $statusText.textContent = tr('installing');
    $statusSub.textContent = tr('installSub');
    $progressBar.style.display = '';
    document.getElementById('progress-message').textContent = tr('preparing');
    document.getElementById('progress-percent').textContent = '0%';
    document.getElementById('progress-fill').style.width = '0%';
    $completeActions.style.display = 'none';
}

function enterComplete(components) {
    currentState = 'complete';
    $glowRing.className = 'loading-logo-container complete';
    $logoArea.className = 'logo-area state-complete';
    $statusText.textContent = tr('completeTitle');
    $statusSub.textContent = tr('completeSub');
    $progressBar.style.display = 'none';
    $completeActions.style.display = '';
}

function enterError(msg) {
    currentState = 'error';
    $glowRing.className = 'loading-logo-container error';
    $logoArea.className = 'logo-area state-installing';
    $statusText.textContent = tr('installFailed');
    $statusSub.textContent = msg;
}

// ═══════════════════════════════════════════════════════
// 이벤트: Welcome → Settings
// ═══════════════════════════════════════════════════════

$btnNext.addEventListener('click', () => enterSettings());

// 언어 선택 (lang popup으로 대체됨)

// ═══════════════════════════════════════════════════════
// 이벤트: Settings 패널 내부
// ═══════════════════════════════════════════════════════

// 경로 찾기
document.getElementById('btn-browse')?.addEventListener('click', async () => {
    try {
        const r = await invoke('browse_folder');
        if (r) { $installPath.value = r; await invoke('set_install_path', { path: r }); }
    } catch (_) {}
});

$installPath?.addEventListener('change', async () => {
    await invoke('set_install_path', { path: $installPath.value });
});

// 모듈
let selectedModules = new Set();

async function loadModules() {
    const $list = document.getElementById('module-list');
    try {
        const mods = await invoke('get_available_modules');
        $list.innerHTML = '';
        for (const m of mods) {
            const card = document.createElement('div');
            card.className = 'module-card';
            card.dataset.id = m.id;
            card.innerHTML = `
                <div class="module-icon"><img src="${m.icon}" alt="${esc(m.name)}" /></div>
                <div class="module-info">
                    <div class="module-name">${esc(m.name)}</div>
                    <div class="module-desc">${esc(m.description)}</div>
                </div>
                <div class="module-check">
                    <input type="checkbox" class="mod-chk" data-id="${m.id}" />
                </div>`;
            card.addEventListener('click', (e) => {
                if (e.target.tagName === 'INPUT') return;
                const cb = card.querySelector('.mod-chk');
                cb.checked = !cb.checked;
                toggleMod(m.id, cb.checked);
            });
            card.querySelector('.mod-chk').addEventListener('change', (e) => toggleMod(m.id, e.target.checked));
            $list.appendChild(card);
        }
    } catch (_) {}
}

function toggleMod(id, on) {
    on ? selectedModules.add(id) : selectedModules.delete(id);
    document.querySelectorAll('.module-card').forEach(c => {
        c.classList.toggle('selected', selectedModules.has(c.dataset.id));
    });
}

// 설치 버튼
document.getElementById('btn-install')?.addEventListener('click', async () => {
    // 옵션 저장
    await invoke('set_install_path', { path: $installPath.value });
    await invoke('set_shortcut_options', {
        desktop: document.getElementById('chk-desktop').checked,
        startMenu: document.getElementById('chk-startmenu').checked,
    });
    await invoke('set_selected_modules', { modules: Array.from(selectedModules) });

    // 설치 모드 진입
    enterInstalling();

    // 릴리즈 체크
    $statusText.textContent = tr('fetchingRelease');
    $statusSub.textContent = '';

    try {
        await invoke('fetch_latest_release');
    } catch (e) {
        enterError(String(e));
        showToast(tr('fetchFailed'), 'error', 5000);
        return;
    }

    $statusText.textContent = tr('installing');
    $statusSub.textContent = tr('installSub');

    try {
        await invoke('start_install');
    } catch (e) {
        enterError(String(e));
        showToast(tr('installFailed'), 'error', 5000);
    }
});

// ═══════════════════════════════════════════════════════
// 설치 진행 이벤트
// ═══════════════════════════════════════════════════════

listen('install:progress', (ev) => {
    const p = ev.payload;
    const $msg = document.getElementById('progress-message');
    const $pct = document.getElementById('progress-percent');
    const $fill = document.getElementById('progress-fill');

    if (p.step === 'error') {
        enterError(p.message);
        showToast(p.message, 'error', 8000);
        return;
    }

    if (p.step === 'complete') {
        $fill.style.width = '100%';
        $fill.classList.add('complete-fill');
        $msg.textContent = p.message;
        $pct.textContent = '100%';
        setTimeout(() => enterComplete(p.installed_components), 600);
        return;
    }

    $msg.textContent = p.message;
    $pct.textContent = `${p.percent}%`;
    $fill.style.width = `${p.percent}%`;
});

// ═══════════════════════════════════════════════════════
// 완료 상태 버튼
// ═══════════════════════════════════════════════════════

document.getElementById('btn-open-folder')?.addEventListener('click', async () => {
    try {
        const st = await invoke('get_installer_state');
        if (st.install_path) await invoke('plugin:shell|open', { path: st.install_path });
    } catch (_) {}
});

document.getElementById('btn-finish')?.addEventListener('click', () => appWindow.close());

// ═══════════════════════════════════════════════════════
// 언인스톨 모드
// ═══════════════════════════════════════════════════════

async function enterUninstallMode() {
    $btnNext.classList.add('hidden');
    // toolbar is now in title-bar, always visible
    $statusText.textContent = tr('uninstallTitle');

    try {
        const st = await invoke('get_installer_state');
        const info = document.getElementById('uninstall-info');
        if (info && st.install_path) {
            info.innerHTML = `<p>${esc(tr('uninstallPath', { path: st.install_path }))}</p>`;
        }
    } catch (_) {}

    openSheet($uninstallSheet);
}

document.getElementById('btn-cancel')?.addEventListener('click', () => appWindow.close());

document.getElementById('btn-uninstall')?.addEventListener('click', async () => {
    closeSheet($uninstallSheet);
    $glowRing.className = 'loading-logo-container spinning';
    $logoArea.className = 'logo-area state-installing';
    $statusText.textContent = tr('uninstalling');
    $statusSub.textContent = tr('uninstallSub');
    $progressBar.style.display = '';
    currentState = 'uninstalling';

    try { await invoke('start_uninstall'); } catch (e) {
        enterError(String(e));
        showToast(tr('uninstallFailed'), 'error', 8000);
    }
});

listen('uninstall:progress', (ev) => {
    const p = ev.payload;
    const $msg = document.getElementById('progress-message');
    const $pct = document.getElementById('progress-percent');
    const $fill = document.getElementById('progress-fill');

    if (p.step === 'error') {
        enterError(p.message);
        showToast(p.message, 'error', 8000);
        return;
    }

    if (p.step === 'complete') {
        $glowRing.className = 'loading-logo-container complete';
        $logoArea.className = 'logo-area state-complete';
        $statusText.textContent = tr('uninstallComplete');
        $statusSub.textContent = tr('uninstallCompleteSub');
        $fill.style.width = '100%';
        $fill.classList.add('complete-fill');
        $msg.textContent = p.message;
        $pct.textContent = '100%';
        showToast(tr('uninstallComplete'), 'success');

        // 종료 버튼 표시
        $progressBar.style.display = 'none';
        $completeActions.style.display = '';
        document.getElementById('btn-open-folder').style.display = 'none';
        document.getElementById('btn-finish').textContent = tr('btnFinish');
        return;
    }

    $msg.textContent = p.message;
    $pct.textContent = `${p.percent}%`;
    $fill.style.width = `${p.percent}%`;
});

// ═══════════════════════════════════════════════════════
// 테마 토글
// ═══════════════════════════════════════════════════════

function getSystemTheme() {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
}

let currentTheme = 'auto'; // 'auto' | 'light' | 'dark'

function applyTheme(theme) {
    currentTheme = theme;
    document.body.setAttribute('data-theme', theme);
}

function toggleTheme() {
    const effective = currentTheme === 'auto' ? getSystemTheme() : currentTheme;
    applyTheme(effective === 'dark' ? 'light' : 'dark');
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

function showLangPopup(anchorEl) {
    // 중앙에 표시
    $langPopup.classList.add('visible');
    const rect = $langPopup.getBoundingClientRect();
    $langPopup.style.top = `${(window.innerHeight - rect.height) / 2}px`;
    $langPopup.style.left = `${(window.innerWidth - rect.width) / 2}px`;
    // 현재 활성 언어 표시
    $langPopup.querySelectorAll('.lang-popup-item').forEach(el => {
        el.classList.toggle('active', el.dataset.lang === lang);
    });
}

function closeLangPopup() {
    $langPopup.classList.remove('visible');
}

document.addEventListener('click', (e) => {
    if ($langPopup.classList.contains('visible') && !$langPopup.contains(e.target) && e.target.id !== 'btn-lang') {
        closeLangPopup();
    }
});

$langPopup.querySelectorAll('.lang-popup-item').forEach(el => {
    el.addEventListener('click', async () => {
        lang = el.dataset.lang;
        applyTranslations();
        if (currentState === 'welcome') $statusText.textContent = tr('welcome');
        // 랑 팝업 내 select 동기화 제거됨
        try { await invoke('set_language', { language: lang }); } catch (_) {}
        closeLangPopup();
    });
});

document.getElementById('btn-lang')?.addEventListener('click', (e) => {
    e.stopPropagation();
    if ($langPopup.classList.contains('visible')) { closeLangPopup(); return; }
    // 버튼 위에 팝업 표시
    const btn = e.currentTarget;
    const rect = btn.getBoundingClientRect();
    $langPopup.classList.add('visible');
    const popRect = $langPopup.getBoundingClientRect();
    $langPopup.style.left = `${rect.left}px`;
    $langPopup.style.top = `${rect.bottom + 4}px`;
    $langPopup.querySelectorAll('.lang-popup-item').forEach(el => {
        el.classList.toggle('active', el.dataset.lang === lang);
    });
});

// ═══════════════════════════════════════════════════════
// 초기화
// ═══════════════════════════════════════════════════════

(async function init() {
    // 테마 감지 (OS 설정 기본값)
    applyTheme(getSystemTheme());

    // 언어 감지
    try {
        const pref = await invoke('get_preferred_language');
        const supported = ['en','ko','ja','zh-CN','zh-TW','es','pt-BR','ru','de','fr'];
        const norm = pref.trim().replace('_', '-');
        const exact = supported.find(l => l.toLowerCase() === norm.toLowerCase());
        if (exact) lang = exact;
        else {
            const base = norm.toLowerCase().split('-')[0];
            if (base.startsWith('pt')) lang = 'pt-BR';
            else if (base.startsWith('zh')) lang = 'zh-CN';
            else lang = { ko:'ko', ja:'ja', es:'es', ru:'ru', de:'de', fr:'fr' }[base] || 'en';
        }
    } catch (_) {}

    applyTranslations();

    // 링 초기 상태
    $glowRing.className = 'loading-logo-container idle';

    // 모드 확인
    try {
        const mode = await invoke('get_app_mode');
        if (mode.uninstall) { enterUninstallMode(); return; }
    } catch (_) {}

    // 설치 경로 로드
    try {
        const st = await invoke('get_installer_state');
        if ($installPath) $installPath.value = st.install_path || '';
    } catch (_) {}

    // 모듈 로드
    await loadModules();
})();
