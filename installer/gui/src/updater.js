// ═══════════════════════════════════════════════════════
// Saba-chan Updater — 업데이트 적용 창 (설치기 --apply)
// ═══════════════════════════════════════════════════════
// 흐름: 그림(빛이 훑음) → 비스듬히 접히며 매트릭스(적용 중) → 성공(초/파 링, GUI 재시작)·실패(빨/보 링, 닫기)

const { invoke } = window.__TAURI__.core;
const { getCurrentWindow } = window.__TAURI__.window;

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
        rollbackRelaunchFailedSub: 'The update was rolled back to the previous version, but Saba-chan could not be restarted. Please start it yourself.',
        restoreFailedTitle: 'Could not finish the rollback',
        restoreFailedLead: 'Some of the rollback could not be completed. Before starting Saba-chan, follow these steps.',
        leftInPlaceLead: 'The new version did not start properly and could not be stopped. To undo the update, follow these steps.',
        recoverStop: 'Quit Saba-chan and end every saba-core process (Task Manager / Activity Monitor).',
        recoverExes: 'Restore the programs: delete each file on the right, then rename the file on the left to it: {{list}}',
        recoverMissingCore: 'No saba-core backup was found in {{installDir}}. Reinstall the previous version with its installer.',
        recoverData: 'Copy the contents of {{data}} into {{dataDir}} and the contents of {{instances}} into {{instancesDir}}, overwriting.',
        recoverStart: 'Then start Saba-chan.',
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
        rollbackRelaunchFailedSub: '새 버전이 제대로 시작되지 않아 이전 버전으로 되돌렸지만 다시 실행하지 못했어요. 사바쨩을 직접 실행해 주세요.',
        restoreFailedTitle: '되돌리기 실패',
        restoreFailedLead: '이전 버전으로 다 되돌리지 못했어요. 사바쨩을 실행하기 전에 이 차례로 되돌려 주세요.',
        leftInPlaceLead: '새 버전이 제대로 시작되지 않았는데 종료하지 못했어요. 업데이트를 되돌리려면 이 차례로 해 주세요.',
        recoverStop: '사바쨩을 끄고 saba-core 프로세스를 모두 종료해 주세요(작업 관리자 / 활성 상태 보기).',
        recoverExes: '프로그램 파일을 되돌려 주세요. 오른쪽 파일을 지운 뒤 왼쪽 파일의 이름을 그것으로 바꿔요: {{list}}',
        recoverMissingCore: '{{installDir}}에 saba-core 백업이 없어요. 이전 버전 설치 파일로 다시 설치해 주세요.',
        recoverData: '{{data}}의 내용을 {{dataDir}}에, {{instances}}의 내용을 {{instancesDir}}에 덮어써 주세요.',
        recoverStart: '그다음 사바쨩을 실행해 주세요.',
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
        rollbackRelaunchFailedSub: '前のバージョンに戻しましたが、再起動できませんでした。サバちゃんを手動で起動してください。',
        restoreFailedTitle: '元に戻せませんでした',
        restoreFailedLead: '前のバージョンに完全には戻せませんでした。サバちゃんを起動する前に、次の順で戻してください。',
        leftInPlaceLead: '新しいバージョンが正しく起動せず、終了もできませんでした。アップデートを元に戻すには、次の順で操作してください。',
        recoverStop: 'サバちゃんを終了し、saba-core のプロセスをすべて終了してください(タスク マネージャー / アクティビティモニタ)。',
        recoverExes: 'プログラムを戻してください。右側のファイルを削除してから、左側のファイルの名前をそれに変更します: {{list}}',
        recoverMissingCore: '{{installDir}} に saba-core のバックアップがありません。前のバージョンのインストーラーで再インストールしてください。',
        recoverData: '{{data}} の中身を {{dataDir}} に、{{instances}} の中身を {{instancesDir}} に上書きしてください。',
        recoverStart: 'その後、サバちゃんを起動してください。',
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
        rollbackRelaunchFailedSub: '已回退到之前的版本，但无法重新启动。请手动启动 Saba-chan。',
        restoreFailedTitle: '无法完成回退',
        restoreFailedLead: '未能完全回退到之前的版本。启动 Saba-chan 之前，请按以下步骤恢复。',
        leftInPlaceLead: '新版本未能正常启动，也无法停止。如需撤销更新，请按以下步骤操作。',
        recoverStop: '退出 Saba-chan，并结束所有 saba-core 进程（任务管理器 / 活动监视器）。',
        recoverExes: '恢复程序文件：先删除右侧的文件，再把左侧文件重命名为它：{{list}}',
        recoverMissingCore: '在 {{installDir}} 中找不到 saba-core 的备份。请用之前版本的安装程序重新安装。',
        recoverData: '将 {{data}} 中的内容覆盖到 {{dataDir}}，将 {{instances}} 中的内容覆盖到 {{instancesDir}}。',
        recoverStart: '然后启动 Saba-chan。',
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
        rollbackRelaunchFailedSub: '已還原至先前的版本，但無法重新啟動。請手動啟動 Saba-chan。',
        restoreFailedTitle: '無法完成還原',
        restoreFailedLead: '未能完全還原至先前的版本。啟動 Saba-chan 之前，請依下列步驟還原。',
        leftInPlaceLead: '新版本未能正常啟動，也無法停止。如需復原更新，請依下列步驟操作。',
        recoverStop: '結束 Saba-chan，並結束所有 saba-core 處理程序（工作管理員 / 活動監視器）。',
        recoverExes: '還原程式檔案：先刪除右側的檔案，再將左側檔案重新命名為它：{{list}}',
        recoverMissingCore: '在 {{installDir}} 中找不到 saba-core 的備份。請用先前版本的安裝程式重新安裝。',
        recoverData: '將 {{data}} 中的內容覆寫到 {{dataDir}}，將 {{instances}} 中的內容覆寫到 {{instancesDir}}。',
        recoverStart: '然後啟動 Saba-chan。',
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
        rollbackRelaunchFailedSub: 'Se volvió a la versión anterior, pero no se pudo reiniciar. Inicia Saba-chan manualmente.',
        restoreFailedTitle: 'No se pudo completar la reversión',
        restoreFailedLead: 'No se pudo volver por completo a la versión anterior. Antes de iniciar Saba-chan, sigue estos pasos.',
        leftInPlaceLead: 'La nueva versión no se inició correctamente y no se pudo detener. Para deshacer la actualización, sigue estos pasos.',
        recoverStop: 'Cierra Saba-chan y finaliza todos los procesos saba-core (Administrador de tareas / Monitor de Actividad).',
        recoverExes: 'Restaura los programas: borra cada archivo de la derecha y cambia el nombre del de la izquierda a ese nombre: {{list}}',
        recoverMissingCore: 'No se encontró una copia de saba-core en {{installDir}}. Reinstala la versión anterior con su instalador.',
        recoverData: 'Copia el contenido de {{data}} en {{dataDir}} y el de {{instances}} en {{instancesDir}}, sobrescribiendo.',
        recoverStart: 'Después, inicia Saba-chan.',
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
        rollbackRelaunchFailedSub: 'A versão anterior foi restaurada, mas não foi possível reiniciar. Abra o Saba-chan manualmente.',
        restoreFailedTitle: 'Não foi possível concluir a reversão',
        restoreFailedLead: 'Não foi possível voltar totalmente à versão anterior. Antes de abrir o Saba-chan, siga estes passos.',
        leftInPlaceLead: 'A nova versão não iniciou corretamente e não pôde ser encerrada. Para desfazer a atualização, siga estes passos.',
        recoverStop: 'Feche o Saba-chan e encerre todos os processos saba-core (Gerenciador de Tarefas / Monitor de Atividade).',
        recoverExes: 'Restaure os programas: apague cada arquivo da direita e renomeie o da esquerda para esse nome: {{list}}',
        recoverMissingCore: 'Nenhuma cópia do saba-core foi encontrada em {{installDir}}. Reinstale a versão anterior com o instalador dela.',
        recoverData: 'Copie o conteúdo de {{data}} para {{dataDir}} e o de {{instances}} para {{instancesDir}}, substituindo.',
        recoverStart: 'Depois, abra o Saba-chan.',
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
        rollbackRelaunchFailedSub: 'Восстановлена предыдущая версия, но перезапустить не удалось. Запустите Saba-chan вручную.',
        restoreFailedTitle: 'Не удалось завершить откат',
        restoreFailedLead: 'Не удалось полностью вернуть предыдущую версию. Перед запуском Saba-chan выполните следующие шаги.',
        leftInPlaceLead: 'Новая версия запустилась неправильно, и её не удалось остановить. Чтобы отменить обновление, выполните следующие шаги.',
        recoverStop: 'Закройте Saba-chan и завершите все процессы saba-core (Диспетчер задач / Мониторинг системы).',
        recoverExes: 'Верните программы: удалите каждый файл справа и переименуйте файл слева в это имя: {{list}}',
        recoverMissingCore: 'В {{installDir}} нет резервной копии saba-core. Переустановите предыдущую версию её установщиком.',
        recoverData: 'Скопируйте с заменой содержимое {{data}} в {{dataDir}}, а содержимое {{instances}} — в {{instancesDir}}.',
        recoverStart: 'Затем запустите Saba-chan.',
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
        rollbackRelaunchFailedSub: 'Die vorherige Version wurde wiederhergestellt, aber der Neustart ist fehlgeschlagen. Bitte starte Saba-chan selbst.',
        restoreFailedTitle: 'Zurücksetzen nicht abgeschlossen',
        restoreFailedLead: 'Die vorherige Version konnte nicht vollständig wiederhergestellt werden. Führe vor dem Start von Saba-chan diese Schritte aus.',
        leftInPlaceLead: 'Die neue Version ist nicht richtig gestartet und konnte nicht beendet werden. Um das Update rückgängig zu machen, führe diese Schritte aus.',
        recoverStop: 'Beende Saba-chan und alle saba-core-Prozesse (Task-Manager / Aktivitätsanzeige).',
        recoverExes: 'Stelle die Programme wieder her: Lösche jeweils die rechte Datei und benenne die linke in diesen Namen um: {{list}}',
        recoverMissingCore: 'In {{installDir}} wurde keine Sicherung von saba-core gefunden. Installiere die vorherige Version mit ihrem Installer neu.',
        recoverData: 'Kopiere den Inhalt von {{data}} nach {{dataDir}} und den Inhalt von {{instances}} nach {{instancesDir}} und überschreibe dabei.',
        recoverStart: 'Starte danach Saba-chan.',
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
        rollbackRelaunchFailedSub: 'La version précédente a été rétablie, mais le redémarrage a échoué. Lancez Saba-chan vous-même.',
        restoreFailedTitle: 'Retour arrière incomplet',
        restoreFailedLead: 'La version précédente n\'a pas pu être entièrement rétablie. Avant de lancer Saba-chan, suivez ces étapes.',
        leftInPlaceLead: 'La nouvelle version n\'a pas démarré correctement et n\'a pas pu être arrêtée. Pour annuler la mise à jour, suivez ces étapes.',
        recoverStop: 'Quittez Saba-chan et arrêtez tous les processus saba-core (Gestionnaire des tâches / Moniteur d\'activité).',
        recoverExes: 'Rétablissez les programmes : supprimez chaque fichier de droite puis renommez celui de gauche avec ce nom : {{list}}',
        recoverMissingCore: 'Aucune copie de saba-core n\'a été trouvée dans {{installDir}}. Réinstallez la version précédente avec son installateur.',
        recoverData: 'Copiez le contenu de {{data}} dans {{dataDir}} et celui de {{instances}} dans {{instancesDir}}, en remplaçant.',
        recoverStart: 'Lancez ensuite Saba-chan.',
    },
};

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
