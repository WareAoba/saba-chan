// ═══════════════════════════════════════════════════════
// Saba-chan Updater GUI — 바닐라 JS 프론트엔드
// ═══════════════════════════════════════════════════════
// Tauri IPC로 백엔드와 통신합니다.

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

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
    $ringTitle.textContent = title || '업데이트중!';
    $ringSub.textContent = sub || '잠시만 기다려 주세요…!';    // 에러/완료 시 닫기 버튼 표시
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
        showRing('error', '업데이트 실패', state.error);
    } else if (state.components.length > 0 && state.components.every(c => !c.update_available)) {
        showRing('complete', '업데이트 완료!', '모든 컴포넌트가 최신 상태입니다');
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
        statusText = 'Not installed';
    } else if (comp.downloaded) {
        statusClass = 'ready';
        statusText = 'Ready to apply';
    } else if (comp.update_available) {
        statusClass = 'update';
        statusText = 'Update available';
    } else {
        statusClass = 'current';
        statusText = 'Up to date';
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
        btn.title = `Download ${comp.display_name}`;
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            btn.disabled = true;
            try {
                await invoke('download_component', { key: comp.key });
                showToast(`Downloaded: ${comp.display_name}`, 'success');
                const result = await invoke('get_status');
                updateState(result);
            } catch (err) {
                showToast(`Failed: ${err}`, 'error');
                btn.disabled = false;
            }
        });
        card.appendChild(btn);
    }

    // 개별 설치 버튼 (미설치인 경우)
    if (!comp.installed) {
        const btn = document.createElement('button');
        btn.className = 'component-action btn-danger';
        btn.textContent = 'Install';
        btn.addEventListener('click', async (e) => {
            e.stopPropagation();
            btn.disabled = true;
            showProgress(`Installing ${comp.display_name}...`, -1);
            try {
                await invoke('install_component', { key: comp.key });
                showToast(`Installed: ${comp.display_name}`, 'success');
                hideProgress();
                const result = await invoke('get_status');
                updateState(result);
            } catch (err) {
                showToast(`Install failed: ${err}`, 'error');
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
                showToast(`Unknown scenario: ${scenarioName}`, 'error');
        }
    } catch (e) {
        hideProgress();
        showRing('error', '❌ 시나리오 오류', String(e));
        showToast(`시나리오 실패: ${e}`, 'error');
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
    showRing('spinning', 'Self-Update 테스트', '업데이트 프로세스를 시뮬레이션합니다...');

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
        showToast(`재기동 실패: ${e}`, 'error');
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
                    <div class="update-complete-title">업데이트 완료!</div>
                    <div class="update-complete-list">${info.components.join(', ')}</div>
                </div>
                <button class="update-complete-close" onclick="this.parentElement.remove()">✕</button>
            `;
            document.body.prepend(banner);
            
            // 토스트도 표시
            showToast(`${info.components.length}개 컴포넌트 업데이트 완료`, 'success', 5000);
            
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
    showRing('spinning', '업데이트 적용 준비 중…', '잠시만 기다려 주세요');

    // 프로그레스 바 표시
    showProgress('매니페스트 로딩 중...', 0);

    // 진행 이벤트 리스닝 — 프로그레스 링 + 프로그레스 바 + 토스트 활용
    listen('apply:progress', (event) => {
        const { step, message, percent, applied } = event.payload;

        // 프로그레스 바
        if (percent >= 0) {
            showProgress(message, percent);
        }

        // 프로그레스 링 업데이트
        if (step === 'manifest') {
            showRing('spinning', '매니페스트 로딩', message);
        } else if (step === 'applying') {
            showRing('spinning', '업데이트 파일 적용 중…', message);
        } else if (step === 'complete') {
            showRing('complete', '업데이트 완료!', message);

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
                            <div class="component-version">적용 완료</div>
                        </div>
                        <span class="component-status-badge current">Updated</span>
                    `;
                    $componentList.appendChild(card);
                }
                showToast(`${applied.length}개 컴포넌트 업데이트 완료`, 'success', 5000);
            }

            // 재기동 안내
            if (mode.relaunch) {
                setTimeout(() => {
                    showRing('spinning', '업데이트 완료!', 'GUI를 재시작합니다…');
                    showProgress('GUI 재시작 중...', 100);
                }, 1500);
            }
        } else if (step === 'error') {
            showRing('error', '업데이트 실패', message);
            hideProgress();
            showToast(message, 'error', 8000);
        }
    });

    // apply 실행
    invoke('start_apply').catch(err => {
        showRing('error', '업데이트 실패', String(err));
        hideProgress();
        showToast(`적용 실패: ${err}`, 'error', 8000);
    });
}
