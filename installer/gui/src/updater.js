// ═══════════════════════════════════════════════════════
// Saba-chan Updater — 업데이트 적용 창 (설치기 --apply)
// ═══════════════════════════════════════════════════════
// 흐름: 그림(빛이 훑음) → 비스듬히 접히며 매트릭스(적용 중) → 성공(초/파 링, GUI 재시작)·실패(빨/보 링, 닫기)

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

// SSOT: updater/src/constants.rs 와 동일 목록
// Tauri 웹뷰 환경이므로 CJS/ESM import 불가 → 인라인 선언 유지
const SUPPORTED_LANGUAGES = ['en', 'ko', 'ja', 'zh-CN', 'zh-TW', 'es', 'pt-BR', 'ru', 'de', 'fr'];

const TRANSLATIONS = {
    en: {
        modeUpdate: 'Update',
        appTitle: 'Saba-chan Updater',
        minimize: 'Minimize',
        close: 'Close',
        loadingTitle: 'Updating...',
        loadingSub: 'Please wait...',
        ringComplete: 'Update complete!',
        applyLoadingManifest: 'Loading update info...',
        applyApplyingFiles: 'Applying update files...',
        applyRestartingGui: 'Restarting GUI...',
        applyFailed: 'Update failed',
        applyFailedCount: '{{applied}} applied, {{failed}} failed or skipped',
        relaunchFailedTitle: 'Could not restart',
        relaunchFailedSub: 'The update is applied. Please start Saba-chan yourself.',
    },
    ko: {
        modeUpdate: '업데이트',
        appTitle: '사바쨩 업데이터',
        minimize: '최소화',
        close: '닫기',
        loadingTitle: '업데이트 중!',
        loadingSub: '잠시만 기다려 주세요...!',
        ringComplete: '업데이트 완료!',
        applyLoadingManifest: '업데이트 정보를 불러오는 중...',
        applyApplyingFiles: '업데이트 파일 적용 중…',
        applyRestartingGui: 'GUI를 재시작합니다…',
        applyFailed: '업데이트 실패',
        applyFailedCount: '{{applied}}개 적용, {{failed}}개 실패하거나 건너뜀',
        relaunchFailedTitle: '재시작 실패',
        relaunchFailedSub: '업데이트는 적용했어요. 사바쨩을 직접 실행해 주세요.',
    },
    ja: {
        modeUpdate: 'アップデート',
        appTitle: 'Saba-chan アップデーター',
        minimize: '最小化',
        close: '閉じる',
        loadingTitle: '更新中...',
        loadingSub: 'しばらくお待ちください...',
        ringComplete: '更新完了!',
        applyLoadingManifest: 'マニフェスト読み込み中...',
        applyApplyingFiles: '更新ファイルを適用中...',
        applyRestartingGui: 'GUIを再起動しています...',
        applyFailed: '更新失敗',
        applyFailedCount: '{{applied}} 件適用、{{failed}} 件失敗またはスキップ',
        relaunchFailedTitle: '再起動できませんでした',
        relaunchFailedSub: 'アップデートは適用されました。サバちゃんを手動で起動してください。',
    },
    'zh-CN': {
        modeUpdate: '更新',
        appTitle: 'Saba-chan 更新程序',
        minimize: '最小化',
        close: '关闭',
        loadingTitle: '更新中...',
        loadingSub: '请稍候...',
        ringComplete: '更新完成!',
        applyLoadingManifest: '正在加载更新信息...',
        applyApplyingFiles: '正在应用更新文件...',
        applyRestartingGui: '正在重启 GUI...',
        applyFailed: '更新失败',
        applyFailedCount: '已应用 {{applied}} 个,{{failed}} 个失败或已跳过',
        relaunchFailedTitle: '无法重新启动',
        relaunchFailedSub: '更新已应用。请手动启动 Saba-chan。',
    },
    'zh-TW': {
        modeUpdate: '更新',
        appTitle: 'Saba-chan 更新程式',
        minimize: '最小化',
        close: '關閉',
        loadingTitle: '更新中...',
        loadingSub: '請稍候...',
        ringComplete: '更新完成!',
        applyLoadingManifest: '正在載入更新資訊...',
        applyApplyingFiles: '正在套用更新檔案...',
        applyRestartingGui: '正在重新啟動 GUI...',
        applyFailed: '更新失敗',
        applyFailedCount: '已套用 {{applied}} 個,{{failed}} 個失敗或已略過',
        relaunchFailedTitle: '無法重新啟動',
        relaunchFailedSub: '更新已套用。請手動啟動 Saba-chan。',
    },
    es: {
        modeUpdate: 'Actualización',
        appTitle: 'Actualizador de Saba-chan',
        minimize: 'Minimizar',
        close: 'Cerrar',
        loadingTitle: 'Actualizando...',
        loadingSub: 'Espera un momento...',
        ringComplete: '¡Actualización completa!',
        applyLoadingManifest: 'Cargando la información de la actualización...',
        applyApplyingFiles: 'Aplicando los archivos de la actualización...',
        applyRestartingGui: 'Reiniciando la GUI...',
        applyFailed: 'La actualización falló',
        applyFailedCount: '{{applied}} aplicados, {{failed}} fallidos u omitidos',
        relaunchFailedTitle: 'No se pudo reiniciar',
        relaunchFailedSub: 'La actualización se aplicó. Inicia Saba-chan manualmente.',
    },
    'pt-BR': {
        modeUpdate: 'Atualização',
        appTitle: 'Atualizador do Saba-chan',
        minimize: 'Minimizar',
        close: 'Fechar',
        loadingTitle: 'Atualizando...',
        loadingSub: 'Aguarde um momento...',
        ringComplete: 'Atualização concluída!',
        applyLoadingManifest: 'Carregando as informações da atualização...',
        applyApplyingFiles: 'Aplicando os arquivos da atualização...',
        applyRestartingGui: 'Reiniciando a GUI...',
        applyFailed: 'Falha na atualização',
        applyFailedCount: '{{applied}} aplicados, {{failed}} com falha ou ignorados',
        relaunchFailedTitle: 'Não foi possível reiniciar',
        relaunchFailedSub: 'A atualização foi aplicada. Abra o Saba-chan manualmente.',
    },
    ru: {
        modeUpdate: 'Обновление',
        appTitle: 'Обновление Saba-chan',
        minimize: 'Свернуть',
        close: 'Закрыть',
        loadingTitle: 'Обновление...',
        loadingSub: 'Подождите немного...',
        ringComplete: 'Обновление завершено!',
        applyLoadingManifest: 'Загрузка сведений об обновлении...',
        applyApplyingFiles: 'Применение файлов обновления...',
        applyRestartingGui: 'Перезапуск GUI...',
        applyFailed: 'Не удалось обновить',
        applyFailedCount: 'Применено: {{applied}}, с ошибкой или пропущено: {{failed}}',
        relaunchFailedTitle: 'Не удалось перезапустить',
        relaunchFailedSub: 'Обновление применено. Запустите Saba-chan вручную.',
    },
    de: {
        modeUpdate: 'Update',
        appTitle: 'Saba-chan Updater',
        minimize: 'Minimieren',
        close: 'Schließen',
        loadingTitle: 'Wird aktualisiert...',
        loadingSub: 'Bitte einen Moment warten...',
        ringComplete: 'Update abgeschlossen!',
        applyLoadingManifest: 'Update-Informationen werden geladen...',
        applyApplyingFiles: 'Update-Dateien werden angewendet...',
        applyRestartingGui: 'GUI wird neu gestartet...',
        applyFailed: 'Update fehlgeschlagen',
        applyFailedCount: '{{applied}} angewendet, {{failed}} fehlgeschlagen oder übersprungen',
        relaunchFailedTitle: 'Neustart fehlgeschlagen',
        relaunchFailedSub: 'Das Update wurde angewendet. Bitte starte Saba-chan selbst.',
    },
    fr: {
        modeUpdate: 'Mise à jour',
        appTitle: 'Mise à jour de Saba-chan',
        minimize: 'Réduire',
        close: 'Fermer',
        loadingTitle: 'Mise à jour...',
        loadingSub: 'Merci de patienter...',
        ringComplete: 'Mise à jour terminée !',
        applyLoadingManifest: 'Chargement des informations de mise à jour...',
        applyApplyingFiles: 'Application des fichiers de mise à jour...',
        applyRestartingGui: 'Redémarrage de la GUI...',
        applyFailed: 'Échec de la mise à jour',
        applyFailedCount: '{{applied}} appliqués, {{failed}} en échec ou ignorés',
        relaunchFailedTitle: 'Redémarrage impossible',
        relaunchFailedSub: 'La mise à jour est appliquée. Lancez Saba-chan vous-même.',
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
    try {
        const preferredLanguage = await invoke('get_preferred_language');
        currentLanguage = normalizeLanguageTag(preferredLanguage);
    } catch (_) {
        currentLanguage = normalizeLanguageTag(navigator.language || 'en');
    }
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

    // 거부 사유는 진단용 원문이라 화면에는 번역한 요약만 보인다
    invoke('start_apply').catch(err => {
        console.warn('[Apply]', err);
        showFailure(tr('applyFailedCount', { applied: 0, failed: 1 }));
    });
}
