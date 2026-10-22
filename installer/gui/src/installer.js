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
        modulesUnavailable: 'Could not load the game module list. You can add modules in the app after installing.',
        completeTitle: 'Installation Complete!',
        completeSub: 'Saba-chan is ready.',
        completeWarnings: 'Installed, but some steps had problems:',
        completeLog: 'Details: {{path}}',
        btnClose: 'Close',
        btnLaunch: 'Launch Saba-chan',
        btnRetry: 'Retry',
        errElevation: 'Couldn\'t get administrator rights to create the install folder. Pick another folder or allow the prompt.',
        errOutdated: 'This installer is too old for the current release. Download the latest installer from the website.',
        errIncomplete: 'This release is incomplete, so nothing was installed. Try again later.',
        errProfile: 'Couldn\'t switch the install type (desktop/no GUI). Close Saba-chan and try again.',
        errDownload: 'Download failed. Check your internet connection and try again.',
        errExtract: 'Couldn\'t write the files. Close Saba-chan and any running servers, then try again.',
        launchFailed: 'Couldn\'t start Saba-chan. Start it from the Start menu or desktop shortcut.',
        installFailed: 'Installation failed',
        fetchFailed: 'Failed to fetch releases',
        uninstallTitle: 'Uninstall Saba-chan',
        uninstallDesc: 'This removes Saba-chan along with its data: game server files, worlds (saves), backups, settings and registry entries. Running servers are stopped first.',
        uninstallKeepSettings: 'Keep game servers and settings',
        uninstallKeepSettingsHint: 'Keeps server files, worlds, backups, settings and the Discord connection so a reinstall picks them up. Modules and runtimes are removed and downloaded again.',
        uninstallPath: 'Install location: {{path}}',
        btnCancel: 'Cancel',
        btnUninstall: 'Uninstall',
        uninstalling: 'Uninstalling...',
        uninstallSub: 'Please wait...',
        uninstallComplete: 'Uninstall complete!',
        uninstallCompleteSub: 'Saba-chan has been removed.',
        uninstallCompletePartial: 'Uninstalled, but some files are left',
        uninstallCompletePartialSub: 'These could not be deleted. Close any program using them and delete them yourself:',
        uninstallFailed: 'Uninstall failed',
        labelLanguage: 'Language',
        // Install progress
        progressPrepare: 'Creating install directory...',
        progressElevate: 'Requesting administrator privileges...',
        progressFetch: 'Fetching release information...',
        progressDownload: 'Downloading...',
        progressExtract: 'Extracting...',
        progressModulesShared: 'Installing shared module utilities...',
        progressModulesDownload: 'Downloading game modules...',
        progressModulesExtract: 'Extracting game modules...',
        progressRuntimePython: 'Preparing Python runtime...',
        progressConfig: 'Setting up configuration...',
        progressConfigLang: 'Saving language settings...',
        progressRegistry: 'Registering application...',
        progressShortcuts: 'Creating shortcuts...',
        progressExtensions: 'Installing required extensions...',
        installerOutdated: 'Installer outdated',
        // Uninstall progress
        uninstallProgressDetect: 'Detecting install location...', 
        uninstallProgressStop: 'Stopping processes...',
        uninstallProgressShortcuts: 'Removing shortcuts...',
        uninstallProgressFiles: 'Removing installation files...',
        uninstallProgressData: 'Removing user data and runtime environments...',
        uninstallProgressDataKeep: 'Removing runtime environments (keeping settings)...',
        uninstallProgressTemp: 'Cleaning temporary files...',
        uninstallProgressModules: 'Cleaning modules directory...',
        uninstallProgressModulesKeep: 'Keeping module settings...',
        uninstallProgressRegistry: 'Removing registry entries...',
        uninstallProgressSelfDelete: 'Scheduling self-deletion...',
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
        modulesUnavailable: '게임 모듈 목록을 불러오지 못했습니다. 설치한 뒤 앱에서 모듈을 추가할 수 있습니다.',
        completeTitle: '설치 완료!',
        completeSub: '사바쨩을 쓸 준비가 됐습니다.',
        completeWarnings: '설치는 끝났지만 일부 단계에 문제가 있었습니다:',
        completeLog: '자세한 기록: {{path}}',
        btnClose: '닫기',
        btnLaunch: '사바쨩 실행',
        btnRetry: '다시 시도',
        errElevation: '설치 폴더를 만들 관리자 권한을 얻지 못했습니다. 다른 폴더를 고르거나 권한 요청을 허용해 주세요.',
        errOutdated: '이 설치기는 지금 릴리스보다 오래됐습니다. 웹사이트에서 최신 설치기를 받아 주세요.',
        errIncomplete: '이 릴리스에 필요한 구성요소가 빠져 있어 설치하지 않았습니다. 잠시 뒤 다시 시도해 주세요.',
        errProfile: '설치 구성(데스크톱/화면 없음)을 바꾸지 못했습니다. 사바쨩을 모두 닫고 다시 시도해 주세요.',
        errDownload: '내려받지 못했습니다. 인터넷 연결을 확인하고 다시 시도해 주세요.',
        errExtract: '파일을 쓰지 못했습니다. 사바쨩과 켜져 있는 서버를 끄고 다시 시도해 주세요.',
        launchFailed: '사바쨩을 실행하지 못했습니다. 시작 메뉴나 바탕화면 바로가기로 실행해 주세요.',
        installFailed: '설치 실패',
        fetchFailed: '릴리즈 정보를 가져오지 못했습니다',
        uninstallTitle: '사바쨩 제거',
        uninstallDesc: '사바쨩과 데이터를 함께 제거합니다. 게임 서버 파일, 월드(세이브), 백업, 설정, 레지스트리 항목이 삭제됩니다. 켜져 있는 서버는 먼저 정상 종료합니다.',
        uninstallKeepSettings: '게임 서버와 설정 남기기',
        uninstallKeepSettingsHint: '서버 파일, 월드, 백업, 설정, 디스코드 연결을 남겨 다시 설치하면 그대로 이어집니다. 모듈과 런타임은 지우고 다시 받습니다.',
        uninstallPath: '설치 위치: {{path}}',
        btnCancel: '취소',
        btnUninstall: '제거',
        uninstalling: '제거 중!',
        uninstallSub: '잠시만 기다려 주세요...!',
        uninstallComplete: '제거 완료!',
        uninstallCompleteSub: '사바쨩을 제거했습니다.',
        uninstallCompletePartial: '제거했지만 일부 파일이 남았습니다',
        uninstallCompletePartialSub: '아래 항목을 지우지 못했습니다. 사용 중인 프로그램을 닫고 직접 지워 주세요:',
        uninstallFailed: '제거 실패',
        labelLanguage: '언어',
        // Install progress
        progressPrepare: '설치 폴더를 만드는 중...',
        progressElevate: '관리자 권한 요청 중...',
        progressFetch: '릴리즈 정보 확인 중...',
        progressDownload: '다운로드 중...',
        progressExtract: '압축 해제 중...',
        progressModulesShared: '공용 모듈 유틸리티 설치 중...',
        progressModulesDownload: '게임 모듈 다운로드 중...',
        progressModulesExtract: '게임 모듈 압축 해제 중...',
        progressRuntimePython: 'Python 런타임 준비 중...',
        progressConfig: '설정을 준비하는 중...',
        progressConfigLang: '언어 설정 저장 중...',
        progressRegistry: '앱 등록 중...',
        progressShortcuts: '바로가기 생성 중...',
        progressExtensions: '필수 익스텐션 설치 중...',
        installerOutdated: '인스톨러가 오래됐습니다',
        // Uninstall progress
        uninstallProgressDetect: '설치 위치 확인 중...', 
        uninstallProgressStop: '프로세스 종료 중...',
        uninstallProgressShortcuts: '바로가기 제거 중...',
        uninstallProgressFiles: '설치 파일 제거 중...',
        uninstallProgressData: '사용자 데이터 및 런타임 환경 제거 중...',
        uninstallProgressDataKeep: '런타임 환경 제거 중 (설정 유지)...',
        uninstallProgressTemp: '임시 파일 정리 중...',
        uninstallProgressModules: '모듈 폴더 정리 중...',
        uninstallProgressModulesKeep: '모듈 설정 유지 중...',
        uninstallProgressRegistry: '레지스트리 항목 제거 중...',
        uninstallProgressSelfDelete: '자체 삭제 예약 중...',
    },
    ja: {
        welcome: 'サバちゃん インストーラーへようこそ',
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
        completeSub: 'サバちゃんの準備ができました。',
        completeWarnings: 'インストールは完了しましたが、一部の手順で問題がありました:',
        completeLog: '詳細: {{path}}',
        btnClose: '閉じる',
        btnLaunch: 'サバちゃんを起動',
        btnRetry: '再試行',
        errElevation: 'インストール先フォルダーを作成する管理者権限を取得できませんでした。別のフォルダーを選ぶか、権限の確認を許可してください。',
        errOutdated: 'このインストーラーは現在のリリースより古いです。ウェブサイトから最新のインストーラーを入手してください。',
        errIncomplete: 'このリリースには必要な構成要素が欠けているため、インストールしませんでした。しばらくしてから再試行してください。',
        errProfile: 'インストール構成（デスクトップ/ヘッドレス）を切り替えられませんでした。サバちゃんをすべて閉じてから再試行してください。',
        errDownload: 'ダウンロードできませんでした。インターネット接続を確認して、もう一度お試しください。',
        errExtract: 'ファイルを書き込めませんでした。サバちゃんと起動中のサーバーを終了してから、もう一度お試しください。',
        launchFailed: 'サバちゃんを起動できませんでした。スタートメニューかデスクトップのショートカットから起動してください。',
        installFailed: 'インストール失敗',
        fetchFailed: 'リリース情報の取得に失敗',
        uninstallTitle: 'サバちゃん アンインストール',
        uninstallDesc: 'サバちゃんとデータを削除します。ゲームサーバーのファイル、ワールド（セーブ）、バックアップ、設定、レジストリが削除されます。起動中のサーバーは先に正常終了します。',
        uninstallKeepSettings: 'ゲームサーバーと設定を残す',
        uninstallKeepSettingsHint: 'サーバーファイル、ワールド、バックアップ、設定、Discord 連携を残し、再インストール後もそのまま使えます。モジュールとランタイムは削除して再取得します。',
        uninstallPath: 'インストール先: {{path}}',
        btnCancel: 'キャンセル',
        btnUninstall: 'アンインストール',
        uninstalling: 'アンインストール中...',
        uninstallSub: 'しばらくお待ちください...',
        uninstallComplete: 'アンインストール完了!',
        uninstallCompleteSub: 'サバちゃんは削除されました。',
        uninstallCompletePartial: 'アンインストールしましたが、一部のファイルが残っています',
        uninstallCompletePartialSub: '次の項目を削除できませんでした。使用中のプログラムを閉じてから手動で削除してください:',
        uninstallFailed: 'アンインストール失敗',
        labelLanguage: '言語',
        // Install progress
        progressPrepare: 'インストールディレクトリを作成中...',
        progressElevate: '管理者権限を要求中...',
        progressFetch: 'リリース情報を取得中...',
        progressDownload: 'ダウンロード中...',
        progressExtract: '展開中...',
        progressModulesShared: '共有モジュールユーティリティをインストール中...',
        progressModulesDownload: 'ゲームモジュールをダウンロード中...',
        progressModulesExtract: 'ゲームモジュールを展開中...',
        progressRuntimePython: 'Pythonランタイムを準備中...',
        progressConfig: '設定を構成中...',
        progressConfigLang: '言語設定を保存中...',
        progressRegistry: 'アプリケーションを登録中...',
        progressShortcuts: 'ショートカットを作成中...',
        progressExtensions: '必要な拡張機能をインストール中...',
        installerOutdated: 'インストーラーが古いバージョンです',
        // Uninstall progress
        uninstallProgressDetect: 'インストール先を検出中...', 
        uninstallProgressStop: 'プロセスを停止中...',
        uninstallProgressShortcuts: 'ショートカットを削除中...',
        uninstallProgressFiles: 'インストールファイルを削除中...',
        uninstallProgressData: 'ユーザーデータとランタイム環境を削除中...',
        uninstallProgressDataKeep: 'ランタイム環境を削除中（設定は保持）...',
        uninstallProgressTemp: '一時ファイルをクリーンアップ中...',
        uninstallProgressModules: 'モジュールディレクトリをクリーンアップ中...',
        uninstallProgressModulesKeep: 'モジュール設定を保持中...',
        uninstallProgressRegistry: 'レジストリエントリを削除中...',
        uninstallProgressSelfDelete: '自己削除をスケジュール中...',
    },
    'zh-CN': {
        welcome: '欢迎使用 Saba-chan 安装程序',
        btnNext: '下一步',
        labelPath: '安装位置',
        labelModules: '游戏模块',
        hintModules: '之后也可以添加。',
        labelOptions: '选项',
        labelDesktop: '桌面快捷方式',
        labelStartMenu: '开始菜单快捷方式',
        btnInstall: '安装',
        installing: '正在安装...',
        installSub: '请稍候...',
        preparing: '准备中...',
        fetchingRelease: '正在检查最新版本...',
        completeTitle: '安装完成！',
        completeSub: 'Saba-chan 已准备就绪。',
        completeWarnings: '安装已完成，但部分步骤出现问题：',
        completeLog: '详细记录：{{path}}',
        btnClose: '关闭',
        btnLaunch: '启动 Saba-chan',
        btnRetry: '重试',
        errElevation: '无法获得创建安装文件夹所需的管理员权限。请选择其他文件夹，或允许权限请求。',
        errOutdated: '此安装程序比当前版本旧，请从网站下载最新的安装程序。',
        errIncomplete: '此版本缺少必需的组件，因此未进行安装。请稍后重试。',
        errProfile: '无法切换安装类型（桌面/无界面）。请关闭 Saba-chan 后重试。',
        errDownload: '下载失败。请检查网络连接后重试。',
        errExtract: '无法写入文件。请关闭 Saba-chan 和正在运行的服务器后重试。',
        launchFailed: '无法启动 Saba-chan。请从开始菜单或桌面快捷方式启动。',
        installFailed: '安装失败',
        fetchFailed: '无法获取版本信息',
        uninstallTitle: '卸载 Saba-chan',
        uninstallDesc: '将删除 Saba-chan 及其数据：游戏服务器文件、世界（存档）、备份、设置和注册表项。正在运行的服务器会先正常停止。',
        uninstallKeepSettings: '保留游戏服务器和设置',
        uninstallKeepSettingsHint: '保留服务器文件、世界、备份、设置和 Discord 连接，重新安装后可继续使用。模块和运行环境会删除并重新下载。',
        uninstallPath: '安装位置：{{path}}',
        btnCancel: '取消',
        btnUninstall: '卸载',
        uninstalling: '正在卸载...',
        uninstallSub: '请稍候...',
        uninstallComplete: '卸载完成！',
        uninstallCompleteSub: 'Saba-chan 已被移除。',
        uninstallCompletePartial: '已卸载，但仍有部分文件残留',
        uninstallCompletePartialSub: '以下项目无法删除。请关闭正在使用它们的程序后手动删除：',
        uninstallFailed: '卸载失败',
        labelLanguage: '语言',
        // Install progress
        progressPrepare: '正在创建安装文件夹...',
        progressElevate: '正在请求管理员权限...',
        progressFetch: '正在获取版本信息...',
        progressDownload: '正在下载...',
        progressExtract: '正在解压...',
        progressModulesShared: '正在安装共享模块工具...',
        progressModulesDownload: '正在下载游戏模块...',
        progressModulesExtract: '正在解压游戏模块...',
        progressRuntimePython: '正在准备 Python 运行时...',
        progressConfig: '正在准备设置...',
        progressConfigLang: '正在保存语言设置...',
        progressRegistry: '正在注册应用程序...',
        progressShortcuts: '正在创建快捷方式...',
        progressExtensions: '正在安装所需扩展...',
        installerOutdated: '安装程序版本过旧',
        // Uninstall progress
        uninstallProgressDetect: '正在检测安装位置...', 
        uninstallProgressStop: '正在停止进程...',
        uninstallProgressShortcuts: '正在删除快捷方式...',
        uninstallProgressFiles: '正在删除安装文件...',
        uninstallProgressData: '正在删除用户数据和运行时环境...',
        uninstallProgressDataKeep: '正在删除运行时环境（保留设置）...',
        uninstallProgressTemp: '正在清理临时文件...',
        uninstallProgressModules: '正在清理模块文件夹...',
        uninstallProgressModulesKeep: '正在保留模块设置...',
        uninstallProgressRegistry: '正在删除注册表项...',
        uninstallProgressSelfDelete: '正在计划自我删除...',
    },
    'zh-TW': {
        welcome: '歡迎使用 Saba-chan 安裝程式',
        btnNext: '下一步',
        labelPath: '安裝位置',
        labelModules: '遊戲模組',
        hintModules: '之後也可以新增。',
        labelOptions: '選項',
        labelDesktop: '桌面捷徑',
        labelStartMenu: '開始功能表捷徑',
        btnInstall: '安裝',
        installing: '正在安裝...',
        installSub: '請稍候...',
        preparing: '準備中...',
        fetchingRelease: '正在檢查最新版本...',
        completeTitle: '安裝完成！',
        completeSub: 'Saba-chan 已準備就緒。',
        completeWarnings: '安裝已完成，但部分步驟發生問題：',
        completeLog: '詳細記錄：{{path}}',
        btnClose: '關閉',
        btnLaunch: '啟動 Saba-chan',
        btnRetry: '重試',
        errElevation: '無法取得建立安裝資料夾所需的系統管理員權限。請選擇其他資料夾，或允許權限要求。',
        errOutdated: '此安裝程式比目前版本舊，請從網站下載最新的安裝程式。',
        errIncomplete: '此版本缺少必要的元件，因此未進行安裝。請稍後再試。',
        errProfile: '無法切換安裝類型（桌面/無介面）。請關閉 Saba-chan 後再試。',
        errDownload: '下載失敗。請檢查網路連線後重試。',
        errExtract: '無法寫入檔案。請關閉 Saba-chan 和執行中的伺服器後重試。',
        launchFailed: '無法啟動 Saba-chan。請從開始功能表或桌面捷徑啟動。',
        installFailed: '安裝失敗',
        fetchFailed: '無法取得版本資訊',
        uninstallTitle: '解除安裝 Saba-chan',
        uninstallDesc: '將移除 Saba-chan 及其資料：遊戲伺服器檔案、世界（存檔）、備份、設定和登錄檔項目。執行中的伺服器會先正常停止。',
        uninstallKeepSettings: '保留遊戲伺服器和設定',
        uninstallKeepSettingsHint: '保留伺服器檔案、世界、備份、設定和 Discord 連線，重新安裝後可繼續使用。模組和執行環境會刪除並重新下載。',
        uninstallPath: '安裝位置：{{path}}',
        btnCancel: '取消',
        btnUninstall: '解除安裝',
        uninstalling: '正在解除安裝...',
        uninstallSub: '請稍候...',
        uninstallComplete: '解除安裝完成！',
        uninstallCompleteSub: 'Saba-chan 已被移除。',
        uninstallCompletePartial: '已解除安裝，但仍有部分檔案殘留',
        uninstallCompletePartialSub: '無法刪除下列項目。請關閉正在使用它們的程式後手動刪除：',
        uninstallFailed: '解除安裝失敗',
        labelLanguage: '語言',
        // Install progress
        progressPrepare: '正在建立安裝資料夾...',
        progressElevate: '正在請求管理員權限...',
        progressFetch: '正在取得版本資訊...',
        progressDownload: '正在下載...',
        progressExtract: '正在解壓縮...',
        progressModulesShared: '正在安裝共用模組工具...',
        progressModulesDownload: '正在下載遊戲模組...',
        progressModulesExtract: '正在解壓縮遊戲模組...',
        progressRuntimePython: '正在準備 Python 執行環境...',
        progressConfig: '正在準備設定...',
        progressConfigLang: '正在儲存語言設定...',
        progressRegistry: '正在登錄應用程式...',
        progressShortcuts: '正在建立捷徑...',
        progressExtensions: '正在安裝所需擴充功能...',
        installerOutdated: '安裝程式版本過舊',
        // Uninstall progress
        uninstallProgressDetect: '正在偵測安裝位置...', 
        uninstallProgressStop: '正在停止處理程序...',
        uninstallProgressShortcuts: '正在刪除捷徑...',
        uninstallProgressFiles: '正在刪除安裝檔案...',
        uninstallProgressData: '正在刪除使用者資料和執行環境...',
        uninstallProgressDataKeep: '正在刪除執行環境（保留設定）...',
        uninstallProgressTemp: '正在清理暫存檔案...',
        uninstallProgressModules: '正在清理模組資料夾...',
        uninstallProgressModulesKeep: '正在保留模組設定...',
        uninstallProgressRegistry: '正在刪除登錄檔項目...',
        uninstallProgressSelfDelete: '正在排程自我刪除...',
    },
    es: {
        welcome: 'Bienvenido al instalador de Saba-chan',
        btnNext: 'Siguiente',
        labelPath: 'Ubicación de instalación',
        labelModules: 'Módulos de juego',
        hintModules: 'Puedes añadir más después.',
        labelOptions: 'Opciones',
        labelDesktop: 'Acceso directo en el escritorio',
        labelStartMenu: 'Acceso directo en el menú Inicio',
        btnInstall: 'Instalar',
        installing: 'Instalando...',
        installSub: 'Por favor, espera...',
        preparing: 'Preparando...',
        fetchingRelease: 'Comprobando la última versión...',
        completeTitle: '¡Instalación completada!',
        completeSub: 'Saba-chan está listo.',
        completeWarnings: 'Instalado, pero algunos pasos tuvieron problemas:',
        completeLog: 'Detalles: {{path}}',
        btnClose: 'Cerrar',
        btnLaunch: 'Iniciar Saba-chan',
        btnRetry: 'Reintentar',
        errElevation: 'No se obtuvieron permisos de administrador para crear la carpeta. Elige otra carpeta o acepta la solicitud.',
        errOutdated: 'Este instalador es más antiguo que la versión actual. Descarga el instalador más reciente desde el sitio web.',
        errIncomplete: 'A esta versión le faltan componentes necesarios, así que no se instaló nada. Inténtalo de nuevo más tarde.',
        errProfile: 'No se pudo cambiar el tipo de instalación (escritorio/sin interfaz). Cierra Saba-chan e inténtalo de nuevo.',
        errDownload: 'No se pudo descargar. Revisa tu conexión a internet y vuelve a intentarlo.',
        errExtract: 'No se pudieron escribir los archivos. Cierra Saba-chan y los servidores en marcha y vuelve a intentarlo.',
        launchFailed: 'No se pudo iniciar Saba-chan. Ábrelo desde el menú Inicio o el acceso directo del escritorio.',
        installFailed: 'Error en la instalación',
        fetchFailed: 'No se pudieron obtener las versiones',
        uninstallTitle: 'Desinstalar Saba-chan',
        uninstallDesc: 'Se eliminará Saba-chan junto con sus datos: archivos de servidores, mundos (partidas guardadas), copias de seguridad, ajustes y entradas del registro. Los servidores en marcha se detienen antes.',
        uninstallKeepSettings: 'Conservar servidores y ajustes',
        uninstallKeepSettingsHint: 'Conserva los archivos de servidor, mundos, copias de seguridad, ajustes y la conexión con Discord para una reinstalación. Los módulos y entornos se eliminan y se descargan de nuevo.',
        uninstallPath: 'Ubicación de instalación: {{path}}',
        btnCancel: 'Cancelar',
        btnUninstall: 'Desinstalar',
        uninstalling: 'Desinstalando...',
        uninstallSub: 'Por favor, espera...',
        uninstallComplete: '¡Desinstalación completada!',
        uninstallCompleteSub: 'Saba-chan ha sido eliminado.',
        uninstallCompletePartial: 'Desinstalado, pero quedan algunos archivos',
        uninstallCompletePartialSub: 'No se pudieron eliminar. Cierra los programas que los usan y elimínalos manualmente:',
        uninstallFailed: 'Error en la desinstalación',
        labelLanguage: 'Idioma',
        // Install progress
        progressPrepare: 'Creando directorio de instalación...',
        progressElevate: 'Solicitando privilegios de administrador...',
        progressFetch: 'Obteniendo información de la versión...',
        progressDownload: 'Descargando...',
        progressExtract: 'Extrayendo...',
        progressModulesShared: 'Instalando utilidades de módulos compartidos...',
        progressModulesDownload: 'Descargando módulos de juego...',
        progressModulesExtract: 'Extrayendo módulos de juego...',
        progressRuntimePython: 'Preparando entorno de Python...',
        progressConfig: 'Configurando ajustes...',
        progressConfigLang: 'Guardando configuración de idioma...',
        progressRegistry: 'Registrando la aplicación...',
        progressShortcuts: 'Creando accesos directos...',
        progressExtensions: 'Instalando extensiones necesarias...',
        installerOutdated: 'El instalador está desactualizado',
        // Uninstall progress
        uninstallProgressDetect: 'Detectando ubicación de instalación...', 
        uninstallProgressStop: 'Deteniendo procesos...',
        uninstallProgressShortcuts: 'Eliminando accesos directos...',
        uninstallProgressFiles: 'Eliminando archivos de instalación...',
        uninstallProgressData: 'Eliminando datos de usuario y entornos de ejecución...',
        uninstallProgressDataKeep: 'Eliminando entornos de ejecución (conservando ajustes)...',
        uninstallProgressTemp: 'Limpiando archivos temporales...',
        uninstallProgressModules: 'Limpiando directorio de módulos...',
        uninstallProgressModulesKeep: 'Conservando ajustes de módulos...',
        uninstallProgressRegistry: 'Eliminando entradas del registro...',
        uninstallProgressSelfDelete: 'Programando auto-eliminación...',
    },
    'pt-BR': {
        welcome: 'Bem-vindo ao instalador do Saba-chan',
        btnNext: 'Próximo',
        labelPath: 'Local de instalação',
        labelModules: 'Módulos de jogo',
        hintModules: 'Você pode adicionar mais depois.',
        labelOptions: 'Opções',
        labelDesktop: 'Atalho na área de trabalho',
        labelStartMenu: 'Atalho no menu Iniciar',
        btnInstall: 'Instalar',
        installing: 'Instalando...',
        installSub: 'Por favor, aguarde...',
        preparing: 'Preparando...',
        fetchingRelease: 'Verificando a última versão...',
        completeTitle: 'Instalação concluída!',
        completeSub: 'Saba-chan está pronto.',
        completeWarnings: 'Instalado, mas algumas etapas tiveram problemas:',
        completeLog: 'Detalhes: {{path}}',
        btnClose: 'Fechar',
        btnLaunch: 'Iniciar Saba-chan',
        btnRetry: 'Tentar de novo',
        errElevation: 'Não foi possível obter permissão de administrador para criar a pasta. Escolha outra pasta ou aceite o pedido.',
        errOutdated: 'Este instalador é mais antigo que a versão atual. Baixe o instalador mais recente no site.',
        errIncomplete: 'Esta versão está sem componentes necessários, então nada foi instalado. Tente novamente mais tarde.',
        errProfile: 'Não foi possível trocar o tipo de instalação (desktop/sem interface). Feche o Saba-chan e tente novamente.',
        errDownload: 'O download falhou. Verifique sua conexão com a internet e tente de novo.',
        errExtract: 'Não foi possível gravar os arquivos. Feche o Saba-chan e os servidores em execução e tente de novo.',
        launchFailed: 'Não foi possível iniciar o Saba-chan. Abra-o pelo menu Iniciar ou pelo atalho da área de trabalho.',
        installFailed: 'Falha na instalação',
        fetchFailed: 'Não foi possível obter as versões',
        uninstallTitle: 'Desinstalar Saba-chan',
        uninstallDesc: 'Isso remove o Saba-chan com seus dados: arquivos dos servidores, mundos (saves), backups, configurações e entradas do registro. Servidores em execução são parados antes.',
        uninstallKeepSettings: 'Manter servidores e configurações',
        uninstallKeepSettingsHint: 'Mantém arquivos dos servidores, mundos, backups, configurações e a conexão com o Discord para uma reinstalação. Módulos e ambientes são removidos e baixados de novo.',
        uninstallPath: 'Local de instalação: {{path}}',
        btnCancel: 'Cancelar',
        btnUninstall: 'Desinstalar',
        uninstalling: 'Desinstalando...',
        uninstallSub: 'Por favor, aguarde...',
        uninstallComplete: 'Desinstalação concluída!',
        uninstallCompleteSub: 'O Saba-chan foi removido.',
        uninstallCompletePartial: 'Desinstalado, mas alguns arquivos ficaram',
        uninstallCompletePartialSub: 'Não foi possível excluí-los. Feche os programas que os usam e exclua-os manualmente:',
        uninstallFailed: 'Falha na desinstalação',
        labelLanguage: 'Idioma',
        // Install progress
        progressPrepare: 'Criando pasta de instalação...',
        progressElevate: 'Solicitando privilégios de administrador...',
        progressFetch: 'Obtendo informações da versão...',
        progressDownload: 'Baixando...',
        progressExtract: 'Extraindo...',
        progressModulesShared: 'Instalando utilitários de módulos compartilhados...',
        progressModulesDownload: 'Baixando módulos de jogo...',
        progressModulesExtract: 'Extraindo módulos de jogo...',
        progressRuntimePython: 'Preparando ambiente Python...',
        progressConfig: 'Preparando as configurações...',
        progressConfigLang: 'Salvando configuração de idioma...',
        progressRegistry: 'Registrando o aplicativo...',
        progressShortcuts: 'Criando atalhos...',
        progressExtensions: 'Instalando extensões necessárias...',
        installerOutdated: 'O instalador está desatualizado',
        // Uninstall progress
        uninstallProgressDetect: 'Detectando local de instalação...', 
        uninstallProgressStop: 'Parando processos...',
        uninstallProgressShortcuts: 'Removendo atalhos...',
        uninstallProgressFiles: 'Removendo arquivos de instalação...',
        uninstallProgressData: 'Removendo dados do usuário e ambientes de execução...',
        uninstallProgressDataKeep: 'Removendo ambientes de execução (mantendo configurações)...',
        uninstallProgressTemp: 'Limpando arquivos temporários...',
        uninstallProgressModules: 'Limpando pasta de módulos...',
        uninstallProgressModulesKeep: 'Mantendo configurações de módulos...',
        uninstallProgressRegistry: 'Removendo entradas do registro...',
        uninstallProgressSelfDelete: 'Agendando auto-exclusão...',
    },
    ru: {
        welcome: 'Добро пожаловать в установщик Saba-chan',
        btnNext: 'Далее',
        labelPath: 'Путь установки',
        labelModules: 'Игровые модули',
        hintModules: 'Можно добавить позже.',
        labelOptions: 'Параметры',
        labelDesktop: 'Ярлык на рабочем столе',
        labelStartMenu: 'Ярлык в меню «Пуск»',
        btnInstall: 'Установить',
        installing: 'Установка...',
        installSub: 'Пожалуйста, подождите...',
        preparing: 'Подготовка...',
        fetchingRelease: 'Проверка последней версии...',
        completeTitle: 'Установка завершена!',
        completeSub: 'Saba-chan готов к работе.',
        completeWarnings: 'Установка завершена, но на некоторых шагах возникли проблемы:',
        completeLog: 'Подробности: {{path}}',
        btnClose: 'Закрыть',
        btnLaunch: 'Запустить Saba-chan',
        btnRetry: 'Повторить',
        errElevation: 'Не удалось получить права администратора для создания папки. Выберите другую папку или разрешите запрос.',
        errOutdated: 'Этот установщик старее текущей версии. Скачайте новый установщик с сайта.',
        errIncomplete: 'В этой версии не хватает нужных компонентов, поэтому ничего не установлено. Повторите попытку позже.',
        errProfile: 'Не удалось сменить тип установки (рабочий стол/без интерфейса). Закройте Saba-chan и повторите попытку.',
        errDownload: 'Не удалось скачать файлы. Проверьте подключение к интернету и попробуйте снова.',
        errExtract: 'Не удалось записать файлы. Закройте Saba-chan и запущенные серверы и попробуйте снова.',
        launchFailed: 'Не удалось запустить Saba-chan. Запустите его из меню «Пуск» или с ярлыка на рабочем столе.',
        installFailed: 'Ошибка установки',
        fetchFailed: 'Не удалось получить информацию о версиях',
        uninstallTitle: 'Удаление Saba-chan',
        uninstallDesc: 'Saba-chan будет удалён вместе с данными: файлами серверов, мирами (сохранениями), резервными копиями, настройками и записями реестра. Запущенные серверы сначала будут корректно остановлены.',
        uninstallKeepSettings: 'Сохранить серверы и настройки',
        uninstallKeepSettingsHint: 'Сохраняет файлы серверов, миры, резервные копии, настройки и подключение к Discord для переустановки. Модули и среды выполнения удаляются и загружаются заново.',
        uninstallPath: 'Путь установки: {{path}}',
        btnCancel: 'Отмена',
        btnUninstall: 'Удалить',
        uninstalling: 'Удаление...',
        uninstallSub: 'Пожалуйста, подождите...',
        uninstallComplete: 'Удаление завершено!',
        uninstallCompleteSub: 'Saba-chan был удалён.',
        uninstallCompletePartial: 'Удаление завершено, но некоторые файлы остались',
        uninstallCompletePartialSub: 'Их не удалось удалить. Закройте программы, которые их используют, и удалите их вручную:',
        uninstallFailed: 'Ошибка удаления',
        labelLanguage: 'Язык',
        // Install progress
        progressPrepare: 'Создание папки установки...',
        progressElevate: 'Запрос прав администратора...',
        progressFetch: 'Получение информации о версии...',
        progressDownload: 'Загрузка...',
        progressExtract: 'Извлечение...',
        progressModulesShared: 'Установка общих утилит модулей...',
        progressModulesDownload: 'Загрузка игровых модулей...',
        progressModulesExtract: 'Извлечение игровых модулей...',
        progressRuntimePython: 'Подготовка среды Python...',
        progressConfig: 'Подготовка конфигурации...',
        progressConfigLang: 'Сохранение языковых настроек...',
        progressRegistry: 'Регистрация приложения...',
        progressShortcuts: 'Создание ярлыков...',
        progressExtensions: 'Установка необходимых расширений...',
        installerOutdated: 'Установщик устарел',
        // Uninstall progress
        uninstallProgressDetect: 'Определение места установки...',
        uninstallProgressStop: 'Остановка процессов...',
        uninstallProgressShortcuts: 'Удаление ярлыков...',
        uninstallProgressFiles: 'Удаление файлов установки...',
        uninstallProgressData: 'Удаление пользовательских данных и сред выполнения...',
        uninstallProgressDataKeep: 'Удаление сред выполнения (сохранение настроек)...',
        uninstallProgressTemp: 'Очистка временных файлов...',
        uninstallProgressModules: 'Очистка папки модулей...',
        uninstallProgressModulesKeep: 'Сохранение настроек модулей...',
        uninstallProgressRegistry: 'Удаление записей реестра...',
        uninstallProgressSelfDelete: 'Планирование самоудаления...',
    },
    de: {
        welcome: 'Willkommen beim Saba-chan Installationsprogramm',
        btnNext: 'Weiter',
        labelPath: 'Installationsort',
        labelModules: 'Spielmodule',
        hintModules: 'Weitere können später hinzugefügt werden.',
        labelOptions: 'Optionen',
        labelDesktop: 'Desktop-Verknüpfung',
        labelStartMenu: 'Startmenü-Verknüpfung',
        btnInstall: 'Installieren',
        installing: 'Installation...',
        installSub: 'Bitte warten...',
        preparing: 'Vorbereitung...',
        fetchingRelease: 'Neueste Version wird geprüft...',
        completeTitle: 'Installation abgeschlossen!',
        completeSub: 'Saba-chan ist bereit.',
        completeWarnings: 'Installiert, aber bei einigen Schritten gab es Probleme:',
        completeLog: 'Details: {{path}}',
        btnClose: 'Schließen',
        btnLaunch: 'Saba-chan starten',
        btnRetry: 'Erneut versuchen',
        errElevation: 'Für den Installationsordner fehlten Administratorrechte. Wähle einen anderen Ordner oder erlaube die Abfrage.',
        errOutdated: 'Dieser Installer ist älter als die aktuelle Version. Lade den neuesten Installer von der Website.',
        errIncomplete: 'In dieser Version fehlen benötigte Komponenten, daher wurde nichts installiert. Versuche es später erneut.',
        errProfile: 'Der Installationstyp (Desktop/Headless) konnte nicht gewechselt werden. Schließe Saba-chan und versuche es erneut.',
        errDownload: 'Der Download ist fehlgeschlagen. Prüfe die Internetverbindung und versuche es erneut.',
        errExtract: 'Die Dateien konnten nicht geschrieben werden. Beende Saba-chan und laufende Server und versuche es erneut.',
        launchFailed: 'Saba-chan konnte nicht gestartet werden. Starte es über das Startmenü oder die Desktopverknüpfung.',
        installFailed: 'Installation fehlgeschlagen',
        fetchFailed: 'Versionsinformationen konnten nicht abgerufen werden',
        uninstallTitle: 'Saba-chan deinstallieren',
        uninstallDesc: 'Saba-chan wird mit seinen Daten entfernt: Serverdateien, Welten (Spielstände), Backups, Einstellungen und Registrierungseinträge. Laufende Server werden vorher ordnungsgemäß beendet.',
        uninstallKeepSettings: 'Server und Einstellungen behalten',
        uninstallKeepSettingsHint: 'Behält Serverdateien, Welten, Backups, Einstellungen und die Discord-Verbindung für eine Neuinstallation. Module und Laufzeitumgebungen werden entfernt und neu geladen.',
        uninstallPath: 'Installationsort: {{path}}',
        btnCancel: 'Abbrechen',
        btnUninstall: 'Deinstallieren',
        uninstalling: 'Deinstallation...',
        uninstallSub: 'Bitte warten...',
        uninstallComplete: 'Deinstallation abgeschlossen!',
        uninstallCompleteSub: 'Saba-chan wurde entfernt.',
        uninstallCompletePartial: 'Deinstalliert, aber einige Dateien sind übrig',
        uninstallCompletePartialSub: 'Diese konnten nicht gelöscht werden. Schließe Programme, die sie verwenden, und lösche sie manuell:',
        uninstallFailed: 'Deinstallation fehlgeschlagen',
        labelLanguage: 'Sprache',
        // Install progress
        progressPrepare: 'Installationsverzeichnis wird erstellt...',
        progressElevate: 'Administratorrechte werden angefordert...',
        progressFetch: 'Versionsinformationen werden abgerufen...',
        progressDownload: 'Herunterladen...',
        progressExtract: 'Entpacken...',
        progressModulesShared: 'Gemeinsame Modul-Dienstprogramme werden installiert...',
        progressModulesDownload: 'Spielmodule werden heruntergeladen...',
        progressModulesExtract: 'Spielmodule werden entpackt...',
        progressRuntimePython: 'Python-Laufzeitumgebung wird vorbereitet...',
        progressConfig: 'Konfiguration wird eingerichtet...',
        progressConfigLang: 'Spracheinstellungen werden gespeichert...',
        progressRegistry: 'Anwendung wird registriert...',
        progressShortcuts: 'Verknüpfungen werden erstellt...',
        progressExtensions: 'Erforderliche Erweiterungen werden installiert...',
        installerOutdated: 'Installer ist veraltet',
        // Uninstall progress
        uninstallProgressDetect: 'Installationsort wird erkannt...',
        uninstallProgressStop: 'Prozesse werden beendet...',
        uninstallProgressShortcuts: 'Verknüpfungen werden entfernt...',
        uninstallProgressFiles: 'Installationsdateien werden entfernt...',
        uninstallProgressData: 'Benutzerdaten und Laufzeitumgebungen werden entfernt...',
        uninstallProgressDataKeep: 'Laufzeitumgebungen werden entfernt (Einstellungen werden beibehalten)...',
        uninstallProgressTemp: 'Temporäre Dateien werden bereinigt...',
        uninstallProgressModules: 'Modulverzeichnis wird bereinigt...',
        uninstallProgressModulesKeep: 'Moduleinstellungen werden beibehalten...',
        uninstallProgressRegistry: 'Registrierungseinträge werden entfernt...',
        uninstallProgressSelfDelete: 'Selbstlöschung wird geplant...',
    },
    fr: {
        welcome: 'Bienvenue dans l\'installateur de Saba-chan',
        btnNext: 'Suivant',
        labelPath: 'Emplacement d\'installation',
        labelModules: 'Modules de jeu',
        hintModules: 'Vous pourrez en ajouter d\'autres plus tard.',
        labelOptions: 'Options',
        labelDesktop: 'Raccourci sur le bureau',
        labelStartMenu: 'Raccourci dans le menu Démarrer',
        btnInstall: 'Installer',
        installing: 'Installation en cours...',
        installSub: 'Veuillez patienter...',
        preparing: 'Préparation...',
        fetchingRelease: 'Vérification de la dernière version...',
        completeTitle: 'Installation terminée !',
        completeSub: 'Saba-chan est prêt.',
        completeWarnings: 'Installé, mais certaines étapes ont rencontré des problèmes :',
        completeLog: 'Détails : {{path}}',
        btnClose: 'Fermer',
        btnLaunch: 'Lancer Saba-chan',
        btnRetry: 'Réessayer',
        errElevation: 'Impossible d\'obtenir les droits administrateur pour créer le dossier. Choisissez un autre dossier ou acceptez la demande.',
        errOutdated: 'Ce programme d\'installation est plus ancien que la version actuelle. Téléchargez le plus récent sur le site.',
        errIncomplete: 'Il manque des composants nécessaires dans cette version, rien n\'a donc été installé. Réessayez plus tard.',
        errProfile: 'Impossible de changer le type d\'installation (bureau/sans interface). Fermez Saba-chan et réessayez.',
        errDownload: 'Le téléchargement a échoué. Vérifiez votre connexion internet et réessayez.',
        errExtract: 'Impossible d\'écrire les fichiers. Fermez Saba-chan et les serveurs en cours, puis réessayez.',
        launchFailed: 'Impossible de démarrer Saba-chan. Lancez-le depuis le menu Démarrer ou le raccourci du bureau.',
        installFailed: 'Échec de l\'installation',
        fetchFailed: 'Impossible de récupérer les versions',
        uninstallTitle: 'Désinstaller Saba-chan',
        uninstallDesc: 'Saba-chan sera supprimé avec ses données : fichiers des serveurs, mondes (sauvegardes), copies de sécurité, paramètres et entrées de registre. Les serveurs en cours sont d\'abord arrêtés proprement.',
        uninstallKeepSettings: 'Conserver les serveurs et les paramètres',
        uninstallKeepSettingsHint: 'Conserve les fichiers des serveurs, les mondes, les copies de sécurité, les paramètres et la connexion Discord pour une réinstallation. Les modules et environnements sont supprimés puis retéléchargés.',
        uninstallPath: 'Emplacement d\'installation : {{path}}',
        btnCancel: 'Annuler',
        btnUninstall: 'Désinstaller',
        uninstalling: 'Désinstallation...',
        uninstallSub: 'Veuillez patienter...',
        uninstallComplete: 'Désinstallation terminée !',
        uninstallCompleteSub: 'Saba-chan a été supprimé.',
        uninstallCompletePartial: 'Désinstallé, mais certains fichiers restent',
        uninstallCompletePartialSub: 'Ils n\'ont pas pu être supprimés. Fermez les programmes qui les utilisent et supprimez-les manuellement :',
        uninstallFailed: 'Échec de la désinstallation',
        labelLanguage: 'Langue',
        // Install progress
        progressPrepare: 'Création du répertoire d\'installation...',
        progressElevate: 'Demande de privilèges administrateur...',
        progressFetch: 'Récupération des informations de version...',
        progressDownload: 'Téléchargement...',
        progressExtract: 'Extraction...',
        progressModulesShared: 'Installation des utilitaires de modules partagés...',
        progressModulesDownload: 'Téléchargement des modules de jeu...',
        progressModulesExtract: 'Extraction des modules de jeu...',
        progressRuntimePython: 'Préparation de l\'environnement Python...',
        progressConfig: 'Configuration des paramètres...',
        progressConfigLang: 'Enregistrement des paramètres de langue...',
        progressRegistry: 'Enregistrement de l\'application...',
        progressShortcuts: 'Création des raccourcis...',
        progressExtensions: 'Installation des extensions requises...',
        installerOutdated: 'L\'installateur est obsolète',
        // Uninstall progress
        uninstallProgressDetect: 'Détection de l\'emplacement d\'installation...',
        uninstallProgressStop: 'Arrêt des processus...',
        uninstallProgressShortcuts: 'Suppression des raccourcis...',
        uninstallProgressFiles: 'Suppression des fichiers d\'installation...',
        uninstallProgressData: 'Suppression des données utilisateur et des environnements d\'exécution...',
        uninstallProgressDataKeep: 'Suppression des environnements d\'exécution (conservation des paramètres)...',
        uninstallProgressTemp: 'Nettoyage des fichiers temporaires...',
        uninstallProgressModules: 'Nettoyage du répertoire des modules...',
        uninstallProgressModulesKeep: 'Conservation des paramètres des modules...',
        uninstallProgressRegistry: 'Suppression des entrées de registre...',
        uninstallProgressSelfDelete: 'Planification de l\'auto-suppression...',
    },
};

let lang = 'en';

function tr(key, vars = {}) {
    const b = T[lang] || T.en;
    const t = b[key] || T.en[key] || key;
    return t.replace(/\{\{(\w+)\}\}/g, (_, k) => String(vars[k] ?? ''));
}

// Step → i18n key mappings for progress messages
const INSTALL_STEP_KEYS = {
    'prepare': 'progressPrepare',
    'elevate': 'progressElevate',
    'fetch': 'progressFetch',
    'download': 'progressDownload',
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
};

const UNINSTALL_STEP_KEYS = {
    'detect': 'uninstallProgressDetect',
    'stop': 'uninstallProgressStop',
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
function trStep(stepMap, step, fallback) {
    const key = stepMap[step];
    if (!key) return fallback;
    const b = T[lang] || T.en;
    return b[key] || T.en[key] || fallback;
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
    s('btn-close-text', 'btnClose');
    s('btn-launch-text', 'btnLaunch');
    s('btn-retry-text', 'btnRetry');
    s('uninstall-title', 'uninstallTitle');
    s('uninstall-desc', 'uninstallDesc');
    s('btn-cancel-text', 'btnCancel');
    s('btn-uninstall-text', 'btnUninstall');
    s('label-keep-settings', 'uninstallKeepSettings');
    s('hint-keep-settings', 'uninstallKeepSettingsHint');
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

function enterComplete(p) {
    currentState = 'complete';
    const $warn = document.getElementById('complete-warnings');
    const warnings = (p && p.warnings) || [];
    $warn.replaceChildren();
    if (warnings.length) {
        const head = document.createElement('strong');
        head.textContent = tr('completeWarnings');
        $warn.append(head, warnings.join('\n'));
        if (p.log_path) $warn.append('\n' + tr('completeLog').replace('{{path}}', p.log_path));
    }
    $warn.style.display = warnings.length ? '' : 'none';
    $glowRing.className = 'loading-logo-container complete';
    $logoArea.className = 'logo-area state-complete';
    $statusText.textContent = tr('completeTitle');
    $statusSub.textContent = tr('completeSub');
    $progressBar.style.display = 'none';
    $completeActions.style.display = '';
}

// 설치기가 알려 주는 실패 종류 → 번역 키 (원문은 아래 상자에 함께 보인다)
const ERROR_KEYS = {
    elevation_failed: 'errElevation',
    fetch_failed: 'fetchFailed',
    installer_outdated: 'errOutdated',
    release_incomplete: 'errIncomplete',
    profile_failed: 'errProfile',
    download_failed: 'errDownload',
    extract_failed: 'errExtract',
};

function enterError(p, titleKey = 'installFailed', canRetry = true) {
    currentState = 'error';
    $glowRing.className = 'loading-logo-container error';
    $logoArea.className = 'logo-area state-installing';
    $statusText.textContent = tr(titleKey);
    const key = ERROR_KEYS[p.error_code];
    $statusSub.textContent = key ? tr(key) : p.message;
    const $warn = document.getElementById('complete-warnings');
    $warn.replaceChildren();
    if (key && p.message) $warn.append(p.message);
    if (p.log_path) $warn.append('\n' + tr('completeLog').replace('{{path}}', p.log_path));
    $warn.style.display = $warn.childNodes.length ? '' : 'none';
    $progressBar.style.display = 'none';
    $completeActions.style.display = '';
    document.getElementById('btn-launch').style.display = 'none';
    document.getElementById('btn-retry').style.display = canRetry ? '' : 'none';
    document.getElementById('btn-close-text').textContent = tr('btnClose');
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
        if (r) { $installPath.value = await invoke('set_install_path', { path: r }); }
    } catch (_) {}
});

// 비어 있지 않은 폴더를 고르면 설치기가 그 아래 Saba-chan 폴더로 바꿔 돌려준다
$installPath?.addEventListener('change', async () => {
    $installPath.value = await invoke('set_install_path', { path: $installPath.value });
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
            // 의존 익스텐션 표시 (Fix 3)
            const extBadges = (m.required_extensions || []).length > 0
                ? `<div class="module-extensions">${(m.required_extensions || []).map(e => `<span class="ext-badge">${esc(e)}</span>`).join(' ')}</div>`
                : '';
            card.innerHTML = `
                <div class="module-icon"></div>
                <div class="module-info">
                    <div class="module-name">${esc(m.name)}</div>
                    <div class="module-desc">${esc(m.description)}</div>
                    ${extBadges}
                </div>
                <div class="module-check">
                    <input type="checkbox" class="mod-chk" data-id="${m.id}" />
                </div>`;
            // 아이콘: img를 먼저 만들고 error 핸들러 연결 후 src 설정
            const iconContainer = card.querySelector('.module-icon');
            const iconImg = document.createElement('img');
            iconImg.alt = m.name;
            iconImg.addEventListener('error', () => {
                iconImg.remove();
                iconContainer.classList.add('icon-fallback');
                iconContainer.textContent = '🎮';
            });
            iconImg.src = m.icon;
            iconContainer.appendChild(iconImg);
            card.addEventListener('click', (e) => {
                if (e.target.tagName === 'INPUT') return;
                const cb = card.querySelector('.mod-chk');
                cb.checked = !cb.checked;
                toggleMod(m.id, cb.checked);
            });
            card.querySelector('.mod-chk').addEventListener('change', (e) => toggleMod(m.id, e.target.checked));
            $list.appendChild(card);
        }
    } catch (_) {
        $list.innerHTML = '';
        const note = document.createElement('div');
        note.className = 'module-desc';
        note.textContent = tr('modulesUnavailable');
        $list.appendChild(note);
    }
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
    $installPath.value = await invoke('set_install_path', { path: $installPath.value });
    await invoke('set_language', { language: lang }); // 설치 직전 최종 동기화
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
        enterError({ error_code: 'fetch_failed', message: String(e) });
        return;
    }

    $statusText.textContent = tr('installing');
    $statusSub.textContent = tr('installSub');

    try {
        await invoke('start_install');
    } catch (e) {
        // 이미 도는 설치가 있으면(두 번 누름) 그 진행 화면을 그대로 둔다
        if (String(e) === 'busy') return;
        enterError({ message: String(e) });
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
        enterError(p);
        return;
    }

    if (p.step === 'complete') {
        $fill.style.width = '100%';
        $fill.classList.add('complete-fill');
        $msg.textContent = p.message;
        $pct.textContent = '100%';
        setTimeout(() => enterComplete(p), 600);
        return;
    }

    // Fix 4: 다운로드 바이트 단위 진행률 표시
    if (p.download_received != null && p.download_total != null && p.download_total > 0) {
        const mbReceived = (p.download_received / 1048576).toFixed(1);
        const mbTotal = (p.download_total / 1048576).toFixed(1);
        const dlPct = Math.round(p.download_received / p.download_total * 100);
        const stepMsg = trStep(INSTALL_STEP_KEYS, p.step, p.message);
        const fileName = p.download_file || '';
        $msg.textContent = `${stepMsg} (${mbReceived}/${mbTotal} MB — ${dlPct}%)`;
        $pct.textContent = `${p.percent}%`;
        $fill.style.width = `${p.percent}%`;
        return;
    } else if (p.download_received != null && (!p.download_total || p.download_total === 0)) {
        const mbReceived = (p.download_received / 1048576).toFixed(1);
        const stepMsg = trStep(INSTALL_STEP_KEYS, p.step, p.message);
        $msg.textContent = `${stepMsg} (${mbReceived} MB)`;
        $pct.textContent = `${p.percent}%`;
        $fill.style.width = `${p.percent}%`;
        return;
    }

    $msg.textContent = trStep(INSTALL_STEP_KEYS, p.step, p.message);
    $pct.textContent = `${p.percent}%`;
    $fill.style.width = `${p.percent}%`;
});

// ═══════════════════════════════════════════════════════
// 완료 상태 버튼
// ═══════════════════════════════════════════════════════

document.getElementById('btn-close-installer')?.addEventListener('click', () => appWindow.close());

document.getElementById('btn-launch')?.addEventListener('click', async () => {
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
document.getElementById('btn-retry')?.addEventListener('click', () => {
    $completeActions.style.display = 'none';
    document.getElementById('complete-warnings').style.display = 'none';
    document.getElementById('btn-launch').style.display = '';
    document.getElementById('btn-retry').style.display = 'none';
    enterSettings();
});

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

    const keepSettings = document.getElementById('chk-keep-settings')?.checked ?? false;
    try { await invoke('start_uninstall', { keepSettings }); } catch (e) {
        if (String(e) === 'busy') return;
        enterError({ message: String(e) }, 'uninstallFailed', false);
    }
});

listen('uninstall:progress', (ev) => {
    const p = ev.payload;
    const $msg = document.getElementById('progress-message');
    const $pct = document.getElementById('progress-percent');
    const $fill = document.getElementById('progress-fill');

    if (p.step === 'error') {
        enterError(p, 'uninstallFailed', false);
        return;
    }

    if (p.step === 'complete') {
        $glowRing.className = 'loading-logo-container complete';
        $logoArea.className = 'logo-area state-complete';
        const left = p.left || [];
        $statusText.textContent = tr(left.length ? 'uninstallCompletePartial' : 'uninstallComplete');
        $statusSub.textContent = tr(left.length ? 'uninstallCompletePartialSub' : 'uninstallCompleteSub');
        $fill.style.width = '100%';
        $fill.classList.add('complete-fill');
        $msg.textContent = left.length ? left.join('\n') : p.message;
        $msg.style.whiteSpace = left.length ? 'pre-line' : '';
        $msg.style.overflowWrap = left.length ? 'anywhere' : '';
        $pct.textContent = '100%';

        // 종료 버튼 표시
        $progressBar.style.display = 'none';
        $completeActions.style.display = '';
        document.getElementById('btn-launch').style.display = 'none';
        document.getElementById('btn-close-installer').textContent = tr('btnClose');
        return;
    }

    $msg.textContent = trStep(UNINSTALL_STEP_KEYS, p.step, p.message);
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
    // CSS가 적용된 후 윈도우를 표시하여 흰 화면 방지 (tauri.conf.json: visible=false)
    getCurrentWindow().show();

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

    // 감지된 언어를 Rust 백엔드에 동기화 (기본값 'en'이 아닌 실제 선택 언어를 설치에 반영)
    try { await invoke('set_language', { language: lang }); } catch (_) {}

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
