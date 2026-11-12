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

const T = {
    en: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: 'Setup',
        stepPick: 'Pick',
        stepPrepare: 'Prepare',
        welcomeSetup: 'Let\'s get Saba-chan ready',
        welcomeSetupSub: 'Saba-chan downloads game modules and sets up Python and the saba-chan command. This happens only once.',
        labelSetup: 'What Saba-chan prepares on this Mac',
        setupPython: 'Python runtime',
        setupCommand: 'The saba-chan command (for Terminal and AI agents)',
        setupModules: 'The game modules you pick and the extensions they need',
        hintSetup: 'Saba-chan stays inside the app in your Applications folder. There is no install location to choose.',
        hintModulesSetup: 'You can pick only the games that run on this Mac. You can get more in Saba Storage later.',
        btnSetup: 'Start',
        setupRunning: 'Preparing!',
        setupSub: 'This only happens once.',
        setupCompleteTitle: 'Saba-chan is ready',
        setupCompleteSub: 'You can now use the saba-chan command in a new Terminal window.',
        progressCommand: 'Registering the saba-chan command...',
        moveFirst: 'Move Saba-chan to Applications first',
        moveFirstSub: 'Saba-chan is running from the disk image or a temporary place. Drag it into the Applications folder, then open it from there.',
        unsupportedOs: 'Not for this OS',
        unsupportedArch: 'Not for this CPU',
        welcome: 'Welcome to Saba-chan Installer',
        modeInstall: 'Install',
        modeUninstall: 'Uninstall',
        stepStart: 'Start',
        stepSettings: 'Settings',
        stepInstall: 'Install',
        stepDone: 'Done',
        btnNext: 'Next',
        labelPath: 'Install Location',
        labelComponents: 'Modules and Extensions Location',
        hintComponents: 'Modules and extensions go in the modules and extensions folders here. Files already installed are moved.',
        compNotAbsolute: 'Choose a full folder path, such as D:\\Games\\Saba.',
        compRootOrHome: 'Choose a folder inside a drive or your user folder, not the drive itself or your user folder.',
        compOverlap: 'This folder overlaps the current modules or extensions folder. Choose another folder.',
        compInsideInstall: 'This folder is inside the install folder, or would put modules and extensions inside each other. Choose another folder.',
        compInvalid: 'Saba-chan can\'t use this folder for modules and extensions. Choose another folder.',
        errComponentsDir: 'Couldn\'t move the modules and extensions. Nothing was changed. Choose another folder or try again.',
        errInstallDir: 'This folder can\'t hold Saba-chan. Choose a folder that isn\'t a drive root, your home folder, or the settings folder.',
        labelModules: 'Game Modules',
        hintModules: 'You can add more later.',
        labelChannel: 'Update Channel',
        channelStable: 'Stable',
        channelRc: 'Preview (release candidate)',
        channelBeta: 'Beta (early, may be unstable)',
        hintChannel: 'Installs the latest release of this channel, and later updates come from it too. You can change it in the app settings after installing.',
        labelOptions: 'Options',
        labelDesktop: 'Desktop Shortcut',
        labelStartMenu: 'Start Menu Shortcut',
        privacyNotice: 'Saba-chan sends an anonymous ID and version numbers to the Saba-chan server to check for updates and to show the module and extension lists. You can turn off automatic update checks in the app settings. Discord features send data only after you connect them. Privacy policy: saba-chan.online/legal/privacy',
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
        errElsewhere: 'Saba-chan is already installed in another folder. Update or remove that install first (only one install per PC is supported).',
        errDaemonRunning: 'Saba-chan is running and the installer can\'t tell which install it belongs to. Quit Saba-chan first, then try again.',
        launchFailed: 'Couldn\'t start Saba-chan. Start it from the Start menu or desktop shortcut.',
        installFailed: 'Installation failed',
        fetchFailed: 'Failed to fetch releases',
        errChannelOlder: 'The latest release on this channel is older than this installer, so it can\'t be installed from this channel. Choose another channel.',
        uninstallTitle: 'Uninstall Saba-chan',
        uninstallDesc: 'This removes the Saba-chan program and every file it made: runtimes, modules, caches, logs, temporary files, shortcuts and registry entries. Running servers are stopped first. Choose below what to keep.',
        uninstallKeepInstances: 'Keep game servers',
        uninstallKeepInstancesHint: 'Keeps server files, worlds, backups and each server\'s settings so a reinstall picks them up.',
        uninstallKeepSettings: 'Keep settings',
        uninstallKeepSettingsHint: 'Keeps app settings and the Discord connection so a reinstall picks them up.',
        uninstallPath: 'Install location: {{path}}',
        btnCancel: 'Cancel',
        btnUninstall: 'Uninstall',
        uninstalling: 'Uninstalling...',
        uninstallSub: 'Please wait...',
        uninstallComplete: 'Uninstall complete!',
        uninstallCompleteSub: 'Saba-chan has been removed.',
        uninstallCompletePartial: 'Uninstalled, but some files are left',
        uninstallCompletePartialSub: 'These could not be deleted. Close any program using them and delete them yourself:',
        uninstallWarnServersUnlisted: 'Could not get the list of running servers, so they were force-stopped instead of shut down cleanly ({{detail}}).',
        uninstallWarnServerForceStopped: 'Server {{server}} did not shut down cleanly and was force-stopped ({{detail}}). Changes since its last save may be lost.',
        uninstallWarnDaemonUnanswered: 'The daemon did not answer, so running servers were force-stopped. Changes since their last save may be lost.',
        uninstallFailed: 'Uninstall failed',
        labelLanguage: 'Language',
        // Install progress
        progressPrepare: 'Creating install directory...',
        progressElevate: 'Requesting administrator privileges...',
        progressFetch: 'Fetching release information...',
        progressDownload: 'Downloading...',
        progressExtract: 'Extracting...',
        progressComponentsDir: 'Moving modules and extensions...',
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
        uninstallProgressSandbox: 'Removing isolation profiles and folder permissions...',
        uninstallProgressShortcuts: 'Removing shortcuts...',
        uninstallProgressFiles: 'Removing installation files...',
        uninstallProgressData: 'Removing user data and runtime environments...',
        uninstallProgressDataKeep: 'Removing runtime environments (keeping what you chose)...',
        uninstallProgressTemp: 'Cleaning temporary files...',
        uninstallProgressModules: 'Cleaning modules directory...',
        uninstallProgressModulesKeep: 'Keeping module settings...',
        uninstallProgressRegistry: 'Removing registry entries...',
        uninstallProgressSelfDelete: 'Scheduling self-deletion...',
    },
    ko: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: '준비',
        stepPick: '고르기',
        stepPrepare: '준비',
        welcomeSetup: '사바쨩을 처음 준비할게요',
        welcomeSetupSub: '게임 모듈을 받고, 파이썬과 saba-chan 명령어를 준비해요. 처음 한 번만 해요.',
        labelSetup: '이 Mac에서 준비할 것',
        setupPython: '파이썬 실행 환경',
        setupCommand: 'saba-chan 명령어 (터미널·AI 에이전트용)',
        setupModules: '고른 게임 모듈과 필요한 익스텐션',
        hintSetup: '사바쨩은 응용 프로그램 폴더의 앱 안에 그대로 있어요. 설치 위치는 고르지 않아요.',
        hintModulesSetup: '이 Mac에서 돌릴 수 있는 게임만 고를 수 있어요. 나중에 사바 스토리지에서 더 받을 수 있어요.',
        btnSetup: '준비 시작',
        setupRunning: '준비 중!',
        setupSub: '처음 한 번만 하는 준비예요.',
        setupCompleteTitle: '준비가 끝났어요',
        setupCompleteSub: '이제 새 터미널 창에서 saba-chan 명령을 쓸 수 있어요.',
        progressCommand: 'saba-chan 명령어 등록 중...',
        moveFirst: '먼저 응용 프로그램 폴더로 옮겨 주세요',
        moveFirstSub: '지금은 디스크 이미지나 임시 위치에서 열렸어요. 사바쨩을 응용 프로그램 폴더로 끌어 넣은 뒤 거기서 열어 주세요.',
        unsupportedOs: '이 OS 미지원',
        unsupportedArch: '이 CPU 미지원',
        welcome: '사바쨩 인스톨러에 오신 것을 환영합니다',
        modeInstall: '설치',
        modeUninstall: '제거',
        stepStart: '시작',
        stepSettings: '설정',
        stepInstall: '설치',
        stepDone: '완료',
        btnNext: '다음',
        labelPath: '설치 경로',
        labelComponents: '모듈·익스텐션 위치',
        hintComponents: '이 폴더 아래 modules, extensions 폴더에 모듈과 익스텐션을 둡니다. 이미 설치된 파일은 옮깁니다.',
        compNotAbsolute: 'D:\\Games\\Saba처럼 전체 경로를 골라 주세요.',
        compRootOrHome: '드라이브나 사용자 폴더 자체 말고, 그 안의 폴더를 골라 주세요.',
        compOverlap: '지금 모듈·익스텐션 폴더와 겹칩니다. 다른 폴더를 골라 주세요.',
        compInsideInstall: '설치 폴더 안이거나, 모듈 폴더와 익스텐션 폴더가 서로 겹치게 됩니다. 다른 폴더를 골라 주세요.',
        compInvalid: '이 폴더는 모듈·익스텐션 위치로 쓸 수 없습니다. 다른 폴더를 골라 주세요.',
        errComponentsDir: '모듈과 익스텐션을 옮기지 못했습니다. 바뀐 것은 없습니다. 다른 폴더를 고르거나 다시 시도해 주세요.',
        errInstallDir: '이 폴더에는 사바쨩을 설치할 수 없습니다. 드라이브 루트, 홈 폴더, 설정 폴더가 아닌 폴더를 고르세요.',
        labelModules: '게임 모듈',
        hintModules: '나중에 추가할 수도 있어요.',
        labelChannel: '업데이트 채널',
        channelStable: '정식',
        channelRc: '미리보기 (출시 후보)',
        channelBeta: '베타 (먼저 받기, 불안정할 수 있음)',
        hintChannel: '고른 채널의 최신 판을 설치하고, 이후 업데이트도 이 채널로 받아요. 설치한 뒤에 앱 설정에서 바꿀 수 있어요.',
        labelOptions: '옵션',
        labelDesktop: '바탕화면 바로가기',
        labelStartMenu: '시작 메뉴 바로가기',
        privacyNotice: '사바쨩은 업데이트를 확인하려고 익명 ID와 버전 정보를 사바쨩 서버로 보내요. 모듈·익스텐션 목록을 받을 때도 보내요. 자동 업데이트 확인은 앱 설정에서 끌 수 있어요. 디스코드 기능은 직접 연결한 뒤에만 정보를 보내요. 개인정보처리방침: saba-chan.online/legal/privacy',
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
        errElsewhere: '사바쨩이 이미 다른 폴더에 설치돼 있습니다. 그 설치본을 업데이트하거나 제거한 뒤 다시 시도해 주세요 (한 PC에는 하나만 설치할 수 있습니다).',
        errDaemonRunning: '어느 설치본의 것인지 알 수 없는 사바쨩이 실행 중입니다. 사바쨩을 먼저 종료한 뒤 다시 시도해 주세요.',
        launchFailed: '사바쨩을 실행하지 못했습니다. 시작 메뉴나 바탕화면 바로가기로 실행해 주세요.',
        installFailed: '설치 실패',
        fetchFailed: '릴리즈 정보를 가져오지 못했습니다',
        errChannelOlder: '고른 채널의 최신 판이 이 설치기보다 옛 판이라 이 채널로는 설치할 수 없어요. 다른 채널을 골라 주세요.',
        uninstallTitle: '사바쨩 제거',
        uninstallDesc: '사바쨩 프로그램과 사바쨩이 만든 파일을 모두 지웁니다. 런타임, 모듈, 캐시, 로그, 임시 파일, 바로가기, 레지스트리 항목이 삭제됩니다. 켜져 있는 서버는 먼저 정상 종료합니다. 남길 것은 아래에서 고르세요.',
        uninstallKeepInstances: '게임 서버 남기기',
        uninstallKeepInstancesHint: '서버 파일, 월드, 백업, 서버별 설정을 남겨 다시 설치하면 그대로 이어집니다.',
        uninstallKeepSettings: '설정 남기기',
        uninstallKeepSettingsHint: '앱 설정과 디스코드 연결을 남겨 다시 설치하면 그대로 이어집니다.',
        uninstallPath: '설치 위치: {{path}}',
        btnCancel: '취소',
        btnUninstall: '제거',
        uninstalling: '제거 중!',
        uninstallSub: '잠시만 기다려 주세요...!',
        uninstallComplete: '제거 완료!',
        uninstallCompleteSub: '사바쨩을 제거했습니다.',
        uninstallCompletePartial: '제거했지만 일부 파일이 남았습니다',
        uninstallCompletePartialSub: '아래 항목을 지우지 못했습니다. 사용 중인 프로그램을 닫고 직접 지워 주세요:',
        uninstallWarnServersUnlisted: '켜진 서버 목록을 받지 못해 서버를 정상 종료하지 못하고 강제로 껐습니다 ({{detail}}).',
        uninstallWarnServerForceStopped: '{{server}} 서버를 정상 종료하지 못해 강제로 껐습니다 ({{detail}}). 마지막 저장 이후 내용이 없을 수 있습니다.',
        uninstallWarnDaemonUnanswered: '데몬이 답하지 않아 켜져 있던 서버를 강제로 껐습니다. 마지막 저장 이후 내용이 없을 수 있습니다.',
        uninstallFailed: '제거 실패',
        labelLanguage: '언어',
        // Install progress
        progressPrepare: '설치 폴더를 만드는 중...',
        progressElevate: '관리자 권한 요청 중...',
        progressFetch: '릴리즈 정보 확인 중...',
        progressDownload: '다운로드 중...',
        progressExtract: '압축 해제 중...',
        progressComponentsDir: '모듈·익스텐션 옮기는 중...',
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
        uninstallProgressSandbox: '격리 실행 프로필과 폴더 권한 정리 중...',
        uninstallProgressShortcuts: '바로가기 제거 중...',
        uninstallProgressFiles: '설치 파일 제거 중...',
        uninstallProgressData: '사용자 데이터 및 런타임 환경 제거 중...',
        uninstallProgressDataKeep: '런타임 환경 제거 중 (고른 항목은 남김)...',
        uninstallProgressTemp: '임시 파일 정리 중...',
        uninstallProgressModules: '모듈 폴더 정리 중...',
        uninstallProgressModulesKeep: '모듈 설정 유지 중...',
        uninstallProgressRegistry: '레지스트리 항목 제거 중...',
        uninstallProgressSelfDelete: '자체 삭제 예약 중...',
    },
    ja: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: '準備',
        stepPick: '選ぶ',
        stepPrepare: '準備',
        welcomeSetup: 'サバちゃんの準備を始めます',
        welcomeSetupSub: 'ゲームモジュールをダウンロードし、Pythonとsaba-chanコマンドを準備します。最初の一回だけです。',
        labelSetup: 'このMacで準備するもの',
        setupPython: 'Python実行環境',
        setupCommand: 'saba-chanコマンド（ターミナル・AIエージェント用）',
        setupModules: '選んだゲームモジュールと必要な拡張機能',
        hintSetup: 'サバちゃんはアプリケーションフォルダのアプリの中にそのまま置かれます。インストール先は選びません。',
        hintModulesSetup: 'このMacで動くゲームだけ選べます。あとでサバストレージから追加できます。',
        btnSetup: '準備を始める',
        setupRunning: '準備中！',
        setupSub: '最初の一回だけの準備です。',
        setupCompleteTitle: '準備ができました',
        setupCompleteSub: '新しいターミナルウィンドウでsaba-chanコマンドが使えます。',
        progressCommand: 'saba-chanコマンドを登録中...',
        moveFirst: '先にアプリケーションフォルダへ移動してください',
        moveFirstSub: 'ディスクイメージか一時的な場所から開かれています。サバちゃんをアプリケーションフォルダへドラッグしてから、そこで開いてください。',
        unsupportedOs: 'このOSは非対応',
        unsupportedArch: 'このCPUは非対応',
        welcome: 'サバちゃん インストーラーへようこそ',
        modeInstall: 'インストール',
        modeUninstall: 'アンインストール',
        stepStart: '開始',
        stepSettings: '設定',
        stepInstall: 'インストール',
        stepDone: '完了',
        btnNext: '次へ',
        labelPath: 'インストール先',
        labelComponents: 'モジュール・拡張機能の場所',
        hintComponents: 'このフォルダーの modules と extensions フォルダーにモジュールと拡張機能を置きます。インストール済みのファイルは移動します。',
        compNotAbsolute: 'D:\\Games\\Saba のようにフルパスで選んでください。',
        compRootOrHome: 'ドライブやユーザーフォルダーそのものではなく、その中のフォルダーを選んでください。',
        compOverlap: '現在のモジュール・拡張機能フォルダーと重なっています。別のフォルダーを選んでください。',
        compInsideInstall: 'インストール先フォルダーの中か、モジュールと拡張機能のフォルダーが互いに重なります。別のフォルダーを選んでください。',
        compInvalid: 'このフォルダーはモジュール・拡張機能の場所に使えません。別のフォルダーを選んでください。',
        errComponentsDir: 'モジュールと拡張機能を移動できませんでした。何も変更していません。別のフォルダーを選ぶか、もう一度お試しください。',
        errInstallDir: 'このフォルダーにはサバちゃんをインストールできません。ドライブのルート、ホームフォルダー、設定フォルダー以外を選んでください。',
        labelModules: 'ゲームモジュール',
        hintModules: '後から追加もできます。',
        labelChannel: 'アップデートチャンネル',
        channelStable: '正式版',
        channelRc: 'プレビュー（リリース候補）',
        channelBeta: 'ベータ（先行版・不安定な場合あり）',
        hintChannel: '選んだチャンネルの最新版をインストールし、以後のアップデートもこのチャンネルから受け取ります。インストール後にアプリの設定で変更できます。',
        labelOptions: 'オプション',
        labelDesktop: 'デスクトップショートカット',
        labelStartMenu: 'スタートメニューショートカット',
        privacyNotice: 'Saba-chanはアップデートを確認するため、匿名IDとバージョン情報をSaba-chanのサーバーに送ります。モジュール・拡張機能の一覧を取得するときも送ります。自動アップデート確認はアプリの設定でオフにできます。Discord機能は連携した後にだけ情報を送ります。プライバシーポリシー: saba-chan.online/legal/privacy',
        btnInstall: 'インストール',
        installing: 'インストール中...',
        installSub: 'しばらくお待ちください...',
        preparing: '準備中...',
        fetchingRelease: '最新リリースを確認中...',
        modulesUnavailable: 'ゲームモジュールの一覧を読み込めませんでした。インストール後にアプリでモジュールを追加できます。',
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
        errElsewhere: 'サバちゃんは既に別のフォルダーにインストールされています。そちらを更新するか削除してから、もう一度お試しください（1台のPCにインストールできるのは1つだけです）。',
        errDaemonRunning: 'どのインストールのものか分からないサバちゃんが実行中です。サバちゃんを先に終了してから、もう一度お試しください。',
        launchFailed: 'サバちゃんを起動できませんでした。スタートメニューかデスクトップのショートカットから起動してください。',
        installFailed: 'インストール失敗',
        fetchFailed: 'リリース情報の取得に失敗',
        errChannelOlder: '選んだチャンネルの最新版がこのインストーラーより古いため、このチャンネルではインストールできません。別のチャンネルを選んでください。',
        uninstallTitle: 'サバちゃん アンインストール',
        uninstallDesc: 'サバちゃんのプログラムと、サバちゃんが作ったファイルをすべて削除します。ランタイム、モジュール、キャッシュ、ログ、一時ファイル、ショートカット、レジストリが削除されます。起動中のサーバーは先に正常終了します。残すものを下で選んでください。',
        uninstallKeepInstances: 'ゲームサーバーを残す',
        uninstallKeepInstancesHint: 'サーバーファイル、ワールド、バックアップ、サーバーごとの設定を残し、再インストール後もそのまま使えます。',
        uninstallKeepSettings: '設定を残す',
        uninstallKeepSettingsHint: 'アプリの設定と Discord 連携を残し、再インストール後もそのまま使えます。',
        uninstallPath: 'インストール先: {{path}}',
        btnCancel: 'キャンセル',
        btnUninstall: 'アンインストール',
        uninstalling: 'アンインストール中...',
        uninstallSub: 'しばらくお待ちください...',
        uninstallComplete: 'アンインストール完了!',
        uninstallCompleteSub: 'サバちゃんは削除されました。',
        uninstallCompletePartial: 'アンインストールしましたが、一部のファイルが残っています',
        uninstallCompletePartialSub: '次の項目を削除できませんでした。使用中のプログラムを閉じてから手動で削除してください:',
        uninstallWarnServersUnlisted: '起動中のサーバー一覧を取得できず、正常に終了できなかったため強制終了しました ({{detail}})。',
        uninstallWarnServerForceStopped: 'サーバー {{server}} を正常に終了できず、強制終了しました ({{detail}})。最後の保存以降の内容が失われている可能性があります。',
        uninstallWarnDaemonUnanswered: 'デーモンが応答しなかったため、起動中のサーバーを強制終了しました。最後の保存以降の内容が失われている可能性があります。',
        uninstallFailed: 'アンインストール失敗',
        labelLanguage: '言語',
        // Install progress
        progressPrepare: 'インストールディレクトリを作成中...',
        progressElevate: '管理者権限を要求中...',
        progressFetch: 'リリース情報を取得中...',
        progressDownload: 'ダウンロード中...',
        progressExtract: '展開中...',
        progressComponentsDir: 'モジュールと拡張機能を移動中...',
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
        uninstallProgressSandbox: '分離実行のプロファイルとフォルダー権限を削除中...',
        uninstallProgressShortcuts: 'ショートカットを削除中...',
        uninstallProgressFiles: 'インストールファイルを削除中...',
        uninstallProgressData: 'ユーザーデータとランタイム環境を削除中...',
        uninstallProgressDataKeep: 'ランタイム環境を削除中（選んだものは残します）...',
        uninstallProgressTemp: '一時ファイルをクリーンアップ中...',
        uninstallProgressModules: 'モジュールディレクトリをクリーンアップ中...',
        uninstallProgressModulesKeep: 'モジュール設定を保持中...',
        uninstallProgressRegistry: 'レジストリエントリを削除中...',
        uninstallProgressSelfDelete: '自己削除をスケジュール中...',
    },
    'zh-CN': {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: '准备',
        stepPick: '选择',
        stepPrepare: '准备',
        welcomeSetup: '现在来准备 Saba 酱',
        welcomeSetupSub: '下载游戏模块，并准备 Python 和 saba-chan 命令。只需一次。',
        labelSetup: '将在这台 Mac 上准备',
        setupPython: 'Python 运行环境',
        setupCommand: 'saba-chan 命令（用于终端和 AI 代理）',
        setupModules: '所选游戏模块及其所需扩展',
        hintSetup: 'Saba 酱保留在“应用程序”文件夹的应用中，无需选择安装位置。',
        hintModulesSetup: '只能选择可在这台 Mac 上运行的游戏。之后可在 Saba 存储中获取更多。',
        btnSetup: '开始准备',
        setupRunning: '准备中！',
        setupSub: '只在第一次进行。',
        setupCompleteTitle: '准备完成',
        setupCompleteSub: '现在可以在新的终端窗口中使用 saba-chan 命令。',
        progressCommand: '正在注册 saba-chan 命令...',
        moveFirst: '请先移到“应用程序”文件夹',
        moveFirstSub: '当前是从磁盘映像或临时位置打开的。请将 Saba 酱拖入“应用程序”文件夹，然后从那里打开。',
        unsupportedOs: '不支持此系统',
        unsupportedArch: '不支持此 CPU',
        welcome: '欢迎使用 Saba-chan 安装程序',
        modeInstall: '安装',
        modeUninstall: '卸载',
        stepStart: '开始',
        stepSettings: '设置',
        stepInstall: '安装',
        stepDone: '完成',
        btnNext: '下一步',
        labelPath: '安装位置',
        labelComponents: '模块和扩展位置',
        hintComponents: '模块和扩展将放在此文件夹下的 modules 和 extensions 文件夹中。已安装的文件会被移动。',
        compNotAbsolute: '请选择完整路径，例如 D:\\Games\\Saba。',
        compRootOrHome: '请选择驱动器或用户文件夹内的文件夹，而不是它们本身。',
        compOverlap: '该文件夹与当前的模块或扩展文件夹重叠。请选择其他文件夹。',
        compInsideInstall: '该文件夹位于安装文件夹内，或会使模块和扩展文件夹相互重叠。请选择其他文件夹。',
        compInvalid: '此文件夹不能用作模块和扩展位置。请选择其他文件夹。',
        errComponentsDir: '无法移动模块和扩展。没有做任何更改。请选择其他文件夹或重试。',
        errInstallDir: '无法安装到此文件夹。请选择驱动器根目录、主文件夹和设置文件夹以外的文件夹。',
        labelModules: '游戏模块',
        hintModules: '之后也可以添加。',
        labelChannel: '更新渠道',
        channelStable: '正式版',
        channelRc: '预览版（候选发布）',
        channelBeta: '测试版（抢先体验，可能不稳定）',
        hintChannel: '安装所选渠道的最新版本，之后的更新也从这个渠道接收。安装后可以在应用设置里更改。',
        labelOptions: '选项',
        labelDesktop: '桌面快捷方式',
        labelStartMenu: '开始菜单快捷方式',
        privacyNotice: 'Saba-chan 会向 Saba-chan 服务器发送匿名 ID 和版本信息以检查更新，获取模块和扩展列表时也会发送。可以在应用设置中关闭自动检查更新。Discord 功能仅在你连接后才会发送信息。隐私政策：saba-chan.online/legal/privacy',
        btnInstall: '安装',
        installing: '正在安装...',
        installSub: '请稍候...',
        preparing: '准备中...',
        fetchingRelease: '正在检查最新版本...',
        modulesUnavailable: '无法加载游戏模块列表。安装后可以在应用中添加模块。',
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
        errElsewhere: 'Saba-chan 已安装在其他文件夹中。请先更新或卸载该安装后重试（每台电脑只能安装一份）。',
        errDaemonRunning: '正在运行的 Saba-chan 无法确定属于哪个安装。请先退出 Saba-chan 后重试。',
        launchFailed: '无法启动 Saba-chan。请从开始菜单或桌面快捷方式启动。',
        installFailed: '安装失败',
        fetchFailed: '无法获取版本信息',
        errChannelOlder: '所选渠道的最新版本比此安装程序旧，无法通过这个渠道安装。请选择其他渠道。',
        uninstallTitle: '卸载 Saba-chan',
        uninstallDesc: '这会删除 Saba-chan 程序及其创建的所有文件：运行环境、模块、缓存、日志、临时文件、快捷方式和注册表项。正在运行的服务器会先正常关闭。请在下方选择要保留的内容。',
        uninstallKeepInstances: '保留游戏服务器',
        uninstallKeepInstancesHint: '保留服务器文件、世界、备份和各服务器的设置，重新安装后可继续使用。',
        uninstallKeepSettings: '保留设置',
        uninstallKeepSettingsHint: '保留应用设置和 Discord 连接，重新安装后可继续使用。',
        uninstallPath: '安装位置：{{path}}',
        btnCancel: '取消',
        btnUninstall: '卸载',
        uninstalling: '正在卸载...',
        uninstallSub: '请稍候...',
        uninstallComplete: '卸载完成！',
        uninstallCompleteSub: 'Saba-chan 已被移除。',
        uninstallCompletePartial: '已卸载，但仍有部分文件残留',
        uninstallCompletePartialSub: '以下项目无法删除。请关闭正在使用它们的程序后手动删除：',
        uninstallWarnServersUnlisted: '无法获取正在运行的服务器列表，未能正常关闭，已强制停止 ({{detail}})。',
        uninstallWarnServerForceStopped: '服务器 {{server}} 未能正常关闭，已强制停止 ({{detail}})。上次保存之后的内容可能丢失。',
        uninstallWarnDaemonUnanswered: '守护进程没有响应，已强制停止正在运行的服务器。上次保存之后的内容可能丢失。',
        uninstallFailed: '卸载失败',
        labelLanguage: '语言',
        // Install progress
        progressPrepare: '正在创建安装文件夹...',
        progressElevate: '正在请求管理员权限...',
        progressFetch: '正在获取版本信息...',
        progressDownload: '正在下载...',
        progressExtract: '正在解压...',
        progressComponentsDir: '正在移动模块和扩展...',
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
        uninstallProgressSandbox: '正在删除隔离运行配置和文件夹权限...',
        uninstallProgressShortcuts: '正在删除快捷方式...',
        uninstallProgressFiles: '正在删除安装文件...',
        uninstallProgressData: '正在删除用户数据和运行时环境...',
        uninstallProgressDataKeep: '正在删除运行环境（保留所选内容）...',
        uninstallProgressTemp: '正在清理临时文件...',
        uninstallProgressModules: '正在清理模块文件夹...',
        uninstallProgressModulesKeep: '正在保留模块设置...',
        uninstallProgressRegistry: '正在删除注册表项...',
        uninstallProgressSelfDelete: '正在计划自我删除...',
    },
    'zh-TW': {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: '準備',
        stepPick: '選擇',
        stepPrepare: '準備',
        welcomeSetup: '現在來準備 Saba 醬',
        welcomeSetupSub: '下載遊戲模組，並準備 Python 和 saba-chan 指令。只需一次。',
        labelSetup: '將在這台 Mac 上準備',
        setupPython: 'Python 執行環境',
        setupCommand: 'saba-chan 指令（用於終端機和 AI 代理）',
        setupModules: '所選遊戲模組及其所需擴充功能',
        hintSetup: 'Saba 醬保留在「應用程式」資料夾的 App 中，無需選擇安裝位置。',
        hintModulesSetup: '只能選擇可在這台 Mac 上執行的遊戲。之後可在 Saba 儲存空間取得更多。',
        btnSetup: '開始準備',
        setupRunning: '準備中！',
        setupSub: '只在第一次進行。',
        setupCompleteTitle: '準備完成',
        setupCompleteSub: '現在可以在新的終端機視窗中使用 saba-chan 指令。',
        progressCommand: '正在註冊 saba-chan 指令...',
        moveFirst: '請先移到「應用程式」資料夾',
        moveFirstSub: '目前是從磁碟映像或暫存位置開啟的。請將 Saba 醬拖入「應用程式」資料夾，再從那裡開啟。',
        unsupportedOs: '不支援此系統',
        unsupportedArch: '不支援此 CPU',
        welcome: '歡迎使用 Saba-chan 安裝程式',
        modeInstall: '安裝',
        modeUninstall: '解除安裝',
        stepStart: '開始',
        stepSettings: '設定',
        stepInstall: '安裝',
        stepDone: '完成',
        btnNext: '下一步',
        labelPath: '安裝位置',
        labelComponents: '模組與擴充功能位置',
        hintComponents: '模組與擴充功能會放在此資料夾下的 modules 和 extensions 資料夾。已安裝的檔案會被移動。',
        compNotAbsolute: '請選擇完整路徑，例如 D:\\Games\\Saba。',
        compRootOrHome: '請選擇磁碟機或使用者資料夾內的資料夾，而不是它們本身。',
        compOverlap: '此資料夾與目前的模組或擴充功能資料夾重疊。請選擇其他資料夾。',
        compInsideInstall: '此資料夾位於安裝資料夾內，或會使模組和擴充功能資料夾互相重疊。請選擇其他資料夾。',
        compInvalid: '此資料夾無法作為模組與擴充功能位置。請選擇其他資料夾。',
        errComponentsDir: '無法移動模組與擴充功能。沒有做任何變更。請選擇其他資料夾或再試一次。',
        errInstallDir: '無法安裝到此資料夾。請選擇磁碟機根目錄、主資料夾和設定資料夾以外的資料夾。',
        labelModules: '遊戲模組',
        hintModules: '之後也可以新增。',
        labelChannel: '更新頻道',
        channelStable: '正式版',
        channelRc: '預覽版（候選發布）',
        channelBeta: '測試版（搶先體驗，可能不穩定）',
        hintChannel: '安裝所選頻道的最新版本，之後的更新也從這個頻道接收。安裝後可以在應用程式設定裡變更。',
        labelOptions: '選項',
        labelDesktop: '桌面捷徑',
        labelStartMenu: '開始功能表捷徑',
        privacyNotice: 'Saba-chan 會向 Saba-chan 伺服器傳送匿名 ID 和版本資訊以檢查更新，取得模組和擴充功能清單時也會傳送。可以在應用程式設定中關閉自動檢查更新。Discord 功能僅在你連結後才會傳送資訊。隱私權政策：saba-chan.online/legal/privacy',
        btnInstall: '安裝',
        installing: '正在安裝...',
        installSub: '請稍候...',
        preparing: '準備中...',
        fetchingRelease: '正在檢查最新版本...',
        modulesUnavailable: '無法載入遊戲模組清單。安裝後可以在應用程式中新增模組。',
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
        errElsewhere: 'Saba-chan 已安裝在其他資料夾中。請先更新或移除該安裝後再試（每台電腦只能安裝一份）。',
        errDaemonRunning: '正在執行的 Saba-chan 無法確定屬於哪個安裝。請先結束 Saba-chan 後再試。',
        launchFailed: '無法啟動 Saba-chan。請從開始功能表或桌面捷徑啟動。',
        installFailed: '安裝失敗',
        fetchFailed: '無法取得版本資訊',
        errChannelOlder: '所選頻道的最新版本比此安裝程式舊，無法透過這個頻道安裝。請選擇其他頻道。',
        uninstallTitle: '解除安裝 Saba-chan',
        uninstallDesc: '這會刪除 Saba-chan 程式及其建立的所有檔案：執行環境、模組、快取、記錄、暫存檔、捷徑和登錄項目。執行中的伺服器會先正常關閉。請在下方選擇要保留的內容。',
        uninstallKeepInstances: '保留遊戲伺服器',
        uninstallKeepInstancesHint: '保留伺服器檔案、世界、備份和各伺服器的設定，重新安裝後可繼續使用。',
        uninstallKeepSettings: '保留設定',
        uninstallKeepSettingsHint: '保留應用程式設定和 Discord 連線，重新安裝後可繼續使用。',
        uninstallPath: '安裝位置：{{path}}',
        btnCancel: '取消',
        btnUninstall: '解除安裝',
        uninstalling: '正在解除安裝...',
        uninstallSub: '請稍候...',
        uninstallComplete: '解除安裝完成！',
        uninstallCompleteSub: 'Saba-chan 已被移除。',
        uninstallCompletePartial: '已解除安裝，但仍有部分檔案殘留',
        uninstallCompletePartialSub: '無法刪除下列項目。請關閉正在使用它們的程式後手動刪除：',
        uninstallWarnServersUnlisted: '無法取得執行中的伺服器清單，未能正常關閉，已強制停止 ({{detail}})。',
        uninstallWarnServerForceStopped: '伺服器 {{server}} 未能正常關閉，已強制停止 ({{detail}})。上次儲存之後的內容可能遺失。',
        uninstallWarnDaemonUnanswered: '常駐程式沒有回應，已強制停止執行中的伺服器。上次儲存之後的內容可能遺失。',
        uninstallFailed: '解除安裝失敗',
        labelLanguage: '語言',
        // Install progress
        progressPrepare: '正在建立安裝資料夾...',
        progressElevate: '正在請求管理員權限...',
        progressFetch: '正在取得版本資訊...',
        progressDownload: '正在下載...',
        progressExtract: '正在解壓縮...',
        progressComponentsDir: '正在移動模組和擴充功能...',
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
        uninstallProgressSandbox: '正在刪除隔離執行設定檔和資料夾權限...',
        uninstallProgressShortcuts: '正在刪除捷徑...',
        uninstallProgressFiles: '正在刪除安裝檔案...',
        uninstallProgressData: '正在刪除使用者資料和執行環境...',
        uninstallProgressDataKeep: '正在刪除執行環境（保留所選內容）...',
        uninstallProgressTemp: '正在清理暫存檔案...',
        uninstallProgressModules: '正在清理模組資料夾...',
        uninstallProgressModulesKeep: '正在保留模組設定...',
        uninstallProgressRegistry: '正在刪除登錄檔項目...',
        uninstallProgressSelfDelete: '正在排程自我刪除...',
    },
    es: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: 'Preparar',
        stepPick: 'Elegir',
        stepPrepare: 'Preparar',
        welcomeSetup: 'Vamos a preparar Saba-chan',
        welcomeSetupSub: 'Saba-chan descarga módulos de juego y prepara Python y el comando saba-chan. Solo se hace una vez.',
        labelSetup: 'Qué se prepara en este Mac',
        setupPython: 'Entorno de Python',
        setupCommand: 'El comando saba-chan (para Terminal y agentes de IA)',
        setupModules: 'Los módulos de juego que elijas y las extensiones que necesitan',
        hintSetup: 'Saba-chan se queda dentro de la app en tu carpeta Aplicaciones. No hay que elegir ubicación.',
        hintModulesSetup: 'Solo puedes elegir juegos que funcionan en este Mac. Puedes obtener más en Saba Storage después.',
        btnSetup: 'Empezar',
        setupRunning: '¡Preparando!',
        setupSub: 'Esto solo pasa una vez.',
        setupCompleteTitle: 'Saba-chan está listo',
        setupCompleteSub: 'Ya puedes usar el comando saba-chan en una nueva ventana de Terminal.',
        progressCommand: 'Registrando el comando saba-chan...',
        moveFirst: 'Primero mueve Saba-chan a Aplicaciones',
        moveFirstSub: 'Saba-chan se abrió desde la imagen de disco o un lugar temporal. Arrástralo a la carpeta Aplicaciones y ábrelo desde allí.',
        unsupportedOs: 'No para este sistema',
        unsupportedArch: 'No para esta CPU',
        welcome: 'Bienvenido al instalador de Saba-chan',
        modeInstall: 'Instalar',
        modeUninstall: 'Desinstalar',
        stepStart: 'Inicio',
        stepSettings: 'Ajustes',
        stepInstall: 'Instalar',
        stepDone: 'Listo',
        btnNext: 'Siguiente',
        labelPath: 'Ubicación de instalación',
        labelComponents: 'Ubicación de módulos y extensiones',
        hintComponents: 'Los módulos y extensiones van en las carpetas modules y extensions de aquí. Los archivos ya instalados se mueven.',
        compNotAbsolute: 'Elige una ruta completa, como D:\\Games\\Saba.',
        compRootOrHome: 'Elige una carpeta dentro de una unidad o de tu carpeta de usuario, no la unidad ni la carpeta de usuario.',
        compOverlap: 'Esta carpeta se solapa con la carpeta actual de módulos o extensiones. Elige otra.',
        compInsideInstall: 'Esta carpeta está dentro de la carpeta de instalación o haría que módulos y extensiones se solapen. Elige otra.',
        compInvalid: 'Saba-chan no puede usar esta carpeta para módulos y extensiones. Elige otra.',
        errComponentsDir: 'No se pudieron mover los módulos y extensiones. No se cambió nada. Elige otra carpeta o inténtalo de nuevo.',
        errInstallDir: 'No se puede instalar en esta carpeta. Elige una carpeta que no sea la raíz de una unidad, tu carpeta personal ni la carpeta de ajustes.',
        labelModules: 'Módulos de juego',
        hintModules: 'Puedes añadir más después.',
        labelChannel: 'Canal de actualizaciones',
        channelStable: 'Estable',
        channelRc: 'Vista previa (candidata)',
        channelBeta: 'Beta (anticipada, puede ser inestable)',
        hintChannel: 'Se instala la última versión de este canal y las actualizaciones posteriores también llegan por él. Puedes cambiarlo en los ajustes de la aplicación después de instalar.',
        labelOptions: 'Opciones',
        labelDesktop: 'Acceso directo en el escritorio',
        labelStartMenu: 'Acceso directo en el menú Inicio',
        privacyNotice: 'Saba-chan envía un ID anónimo y los números de versión al servidor de Saba-chan para buscar actualizaciones y mostrar las listas de módulos y extensiones. Puedes desactivar la búsqueda automática de actualizaciones en los ajustes de la app. Las funciones de Discord solo envían datos después de conectarlas. Política de privacidad: saba-chan.online/legal/privacy',
        btnInstall: 'Instalar',
        installing: 'Instalando...',
        installSub: 'Por favor, espera...',
        preparing: 'Preparando...',
        fetchingRelease: 'Comprobando la última versión...',
        modulesUnavailable: 'No se pudo cargar la lista de módulos de juego. Puedes añadir módulos en la app después de instalar.',
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
        errElsewhere: 'Saba-chan ya está instalado en otra carpeta. Actualiza o desinstala esa instalación primero (solo se admite una instalación por PC).',
        errDaemonRunning: 'Saba-chan está en ejecución y el instalador no sabe a qué instalación pertenece. Cierra Saba-chan primero y vuelve a intentarlo.',
        launchFailed: 'No se pudo iniciar Saba-chan. Ábrelo desde el menú Inicio o el acceso directo del escritorio.',
        installFailed: 'Error en la instalación',
        fetchFailed: 'No se pudieron obtener las versiones',
        errChannelOlder: 'La última versión de este canal es anterior a este instalador, así que no se puede instalar desde este canal. Elige otro canal.',
        uninstallTitle: 'Desinstalar Saba-chan',
        uninstallDesc: 'Esto elimina el programa Saba-chan y todos los archivos que creó: entornos, módulos, cachés, registros, archivos temporales, accesos directos y entradas del registro. Los servidores en marcha se detienen antes. Elija abajo qué conservar.',
        uninstallKeepInstances: 'Conservar los servidores',
        uninstallKeepInstancesHint: 'Conserva los archivos de servidor, mundos, copias de seguridad y los ajustes de cada servidor para una reinstalación.',
        uninstallKeepSettings: 'Conservar los ajustes',
        uninstallKeepSettingsHint: 'Conserva los ajustes de la aplicación y la conexión con Discord para una reinstalación.',
        uninstallPath: 'Ubicación de instalación: {{path}}',
        btnCancel: 'Cancelar',
        btnUninstall: 'Desinstalar',
        uninstalling: 'Desinstalando...',
        uninstallSub: 'Por favor, espera...',
        uninstallComplete: '¡Desinstalación completada!',
        uninstallCompleteSub: 'Saba-chan ha sido eliminado.',
        uninstallCompletePartial: 'Desinstalado, pero quedan algunos archivos',
        uninstallCompletePartialSub: 'No se pudieron eliminar. Cierra los programas que los usan y elimínalos manualmente:',
        uninstallWarnServersUnlisted: 'No se pudo obtener la lista de servidores en ejecución, así que se detuvieron a la fuerza ({{detail}}).',
        uninstallWarnServerForceStopped: 'El servidor {{server}} no se cerró correctamente y se detuvo a la fuerza ({{detail}}). Puede perderse lo hecho desde el último guardado.',
        uninstallWarnDaemonUnanswered: 'El daemon no respondió, así que los servidores en ejecución se detuvieron a la fuerza. Puede perderse lo hecho desde el último guardado.',
        uninstallFailed: 'Error en la desinstalación',
        labelLanguage: 'Idioma',
        // Install progress
        progressPrepare: 'Creando directorio de instalación...',
        progressElevate: 'Solicitando privilegios de administrador...',
        progressFetch: 'Obteniendo información de la versión...',
        progressDownload: 'Descargando...',
        progressExtract: 'Extrayendo...',
        progressComponentsDir: 'Moviendo módulos y extensiones...',
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
        uninstallProgressSandbox: 'Eliminando perfiles de aislamiento y permisos de carpetas...',
        uninstallProgressShortcuts: 'Eliminando accesos directos...',
        uninstallProgressFiles: 'Eliminando archivos de instalación...',
        uninstallProgressData: 'Eliminando datos de usuario y entornos de ejecución...',
        uninstallProgressDataKeep: 'Eliminando entornos (se conserva lo elegido)...',
        uninstallProgressTemp: 'Limpiando archivos temporales...',
        uninstallProgressModules: 'Limpiando directorio de módulos...',
        uninstallProgressModulesKeep: 'Conservando ajustes de módulos...',
        uninstallProgressRegistry: 'Eliminando entradas del registro...',
        uninstallProgressSelfDelete: 'Programando auto-eliminación...',
    },
    'pt-BR': {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: 'Preparar',
        stepPick: 'Escolher',
        stepPrepare: 'Preparar',
        welcomeSetup: 'Vamos preparar a Saba-chan',
        welcomeSetupSub: 'A Saba-chan baixa módulos de jogo e prepara o Python e o comando saba-chan. Isso acontece só uma vez.',
        labelSetup: 'O que é preparado neste Mac',
        setupPython: 'Ambiente Python',
        setupCommand: 'O comando saba-chan (para o Terminal e agentes de IA)',
        setupModules: 'Os módulos de jogo escolhidos e as extensões de que precisam',
        hintSetup: 'A Saba-chan fica dentro do app na pasta Aplicativos. Não é preciso escolher local.',
        hintModulesSetup: 'Você só pode escolher jogos que rodam neste Mac. Depois pode obter mais no Saba Storage.',
        btnSetup: 'Começar',
        setupRunning: 'Preparando!',
        setupSub: 'Isso só acontece uma vez.',
        setupCompleteTitle: 'A Saba-chan está pronta',
        setupCompleteSub: 'Agora você pode usar o comando saba-chan em uma nova janela do Terminal.',
        progressCommand: 'Registrando o comando saba-chan...',
        moveFirst: 'Primeiro mova a Saba-chan para Aplicativos',
        moveFirstSub: 'A Saba-chan foi aberta da imagem de disco ou de um local temporário. Arraste-a para a pasta Aplicativos e abra-a de lá.',
        unsupportedOs: 'Não para este sistema',
        unsupportedArch: 'Não para esta CPU',
        welcome: 'Bem-vindo ao instalador do Saba-chan',
        modeInstall: 'Instalar',
        modeUninstall: 'Desinstalar',
        stepStart: 'Início',
        stepSettings: 'Configurações',
        stepInstall: 'Instalar',
        stepDone: 'Concluído',
        btnNext: 'Próximo',
        labelPath: 'Local de instalação',
        labelComponents: 'Local de módulos e extensões',
        hintComponents: 'Módulos e extensões ficam nas pastas modules e extensions daqui. Arquivos já instalados são movidos.',
        compNotAbsolute: 'Escolha um caminho completo, como D:\\Games\\Saba.',
        compRootOrHome: 'Escolha uma pasta dentro de uma unidade ou da sua pasta de usuário, não a própria unidade ou pasta de usuário.',
        compOverlap: 'Esta pasta se sobrepõe à pasta atual de módulos ou extensões. Escolha outra.',
        compInsideInstall: 'Esta pasta fica dentro da pasta de instalação ou faria módulos e extensões se sobreporem. Escolha outra.',
        compInvalid: 'O Saba-chan não pode usar esta pasta para módulos e extensões. Escolha outra.',
        errComponentsDir: 'Não foi possível mover os módulos e extensões. Nada foi alterado. Escolha outra pasta ou tente de novo.',
        errInstallDir: 'Não é possível instalar nesta pasta. Escolha uma pasta que não seja a raiz de uma unidade, a sua pasta pessoal nem a pasta de configurações.',
        labelModules: 'Módulos de jogo',
        hintModules: 'Você pode adicionar mais depois.',
        labelChannel: 'Canal de atualizações',
        channelStable: 'Estável',
        channelRc: 'Prévia (candidata)',
        channelBeta: 'Beta (antecipada, pode ser instável)',
        hintChannel: 'Instala a versão mais recente deste canal, e as próximas atualizações também vêm dele. Você pode mudar nas configurações do app depois de instalar.',
        labelOptions: 'Opções',
        labelDesktop: 'Atalho na área de trabalho',
        labelStartMenu: 'Atalho no menu Iniciar',
        privacyNotice: 'O Saba-chan envia um ID anônimo e os números de versão ao servidor do Saba-chan para verificar atualizações e mostrar as listas de módulos e extensões. Você pode desativar a verificação automática de atualizações nas configurações do app. Os recursos do Discord só enviam dados depois que você os conecta. Política de privacidade: saba-chan.online/legal/privacy',
        btnInstall: 'Instalar',
        installing: 'Instalando...',
        installSub: 'Por favor, aguarde...',
        preparing: 'Preparando...',
        fetchingRelease: 'Verificando a última versão...',
        modulesUnavailable: 'Não foi possível carregar a lista de módulos de jogo. Você pode adicionar módulos no app depois de instalar.',
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
        errElsewhere: 'O Saba-chan já está instalado em outra pasta. Atualize ou remova essa instalação primeiro (só é possível uma instalação por PC).',
        errDaemonRunning: 'O Saba-chan está em execução e o instalador não sabe a qual instalação ele pertence. Feche o Saba-chan primeiro e tente de novo.',
        launchFailed: 'Não foi possível iniciar o Saba-chan. Abra-o pelo menu Iniciar ou pelo atalho da área de trabalho.',
        installFailed: 'Falha na instalação',
        fetchFailed: 'Não foi possível obter as versões',
        errChannelOlder: 'A versão mais recente deste canal é anterior a este instalador, então não dá para instalar por este canal. Escolha outro canal.',
        uninstallTitle: 'Desinstalar Saba-chan',
        uninstallDesc: 'Isto remove o programa Saba-chan e todos os arquivos que ele criou: ambientes, módulos, caches, logs, arquivos temporários, atalhos e entradas do registro. Os servidores em execução são parados antes. Escolha abaixo o que manter.',
        uninstallKeepInstances: 'Manter os servidores',
        uninstallKeepInstancesHint: 'Mantém arquivos dos servidores, mundos, backups e as configurações de cada servidor para uma reinstalação.',
        uninstallKeepSettings: 'Manter as configurações',
        uninstallKeepSettingsHint: 'Mantém as configurações do aplicativo e a conexão com o Discord para uma reinstalação.',
        uninstallPath: 'Local de instalação: {{path}}',
        btnCancel: 'Cancelar',
        btnUninstall: 'Desinstalar',
        uninstalling: 'Desinstalando...',
        uninstallSub: 'Por favor, aguarde...',
        uninstallComplete: 'Desinstalação concluída!',
        uninstallCompleteSub: 'O Saba-chan foi removido.',
        uninstallCompletePartial: 'Desinstalado, mas alguns arquivos ficaram',
        uninstallCompletePartialSub: 'Não foi possível excluí-los. Feche os programas que os usam e exclua-os manualmente:',
        uninstallWarnServersUnlisted: 'Não foi possível obter a lista de servidores em execução, então eles foram forçados a parar ({{detail}}).',
        uninstallWarnServerForceStopped: 'O servidor {{server}} não fechou corretamente e foi forçado a parar ({{detail}}). O que mudou desde o último salvamento pode ter se perdido.',
        uninstallWarnDaemonUnanswered: 'O daemon não respondeu, então os servidores em execução foram forçados a parar. O que mudou desde o último salvamento pode ter se perdido.',
        uninstallFailed: 'Falha na desinstalação',
        labelLanguage: 'Idioma',
        // Install progress
        progressPrepare: 'Criando pasta de instalação...',
        progressElevate: 'Solicitando privilégios de administrador...',
        progressFetch: 'Obtendo informações da versão...',
        progressDownload: 'Baixando...',
        progressExtract: 'Extraindo...',
        progressComponentsDir: 'Movendo módulos e extensões...',
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
        uninstallProgressSandbox: 'Removendo perfis de isolamento e permissões de pastas...',
        uninstallProgressShortcuts: 'Removendo atalhos...',
        uninstallProgressFiles: 'Removendo arquivos de instalação...',
        uninstallProgressData: 'Removendo dados do usuário e ambientes de execução...',
        uninstallProgressDataKeep: 'Removendo ambientes (mantendo o que você escolheu)...',
        uninstallProgressTemp: 'Limpando arquivos temporários...',
        uninstallProgressModules: 'Limpando pasta de módulos...',
        uninstallProgressModulesKeep: 'Mantendo configurações de módulos...',
        uninstallProgressRegistry: 'Removendo entradas do registro...',
        uninstallProgressSelfDelete: 'Agendando auto-exclusão...',
    },
    ru: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: 'Подготовка',
        stepPick: 'Выбор',
        stepPrepare: 'Подготовка',
        welcomeSetup: 'Подготовим Saba-chan',
        welcomeSetupSub: 'Saba-chan загрузит игровые модули и подготовит Python и команду saba-chan. Это нужно только один раз.',
        labelSetup: 'Что будет подготовлено на этом Mac',
        setupPython: 'Среда Python',
        setupCommand: 'Команда saba-chan (для Терминала и ИИ-агентов)',
        setupModules: 'Выбранные игровые модули и нужные им расширения',
        hintSetup: 'Saba-chan остаётся внутри приложения в папке «Программы». Выбирать место установки не нужно.',
        hintModulesSetup: 'Можно выбрать только игры, которые работают на этом Mac. Остальные можно получить позже в Saba Storage.',
        btnSetup: 'Начать',
        setupRunning: 'Подготовка!',
        setupSub: 'Это нужно только один раз.',
        setupCompleteTitle: 'Saba-chan готова',
        setupCompleteSub: 'Теперь команду saba-chan можно использовать в новом окне Терминала.',
        progressCommand: 'Регистрация команды saba-chan...',
        moveFirst: 'Сначала переместите Saba-chan в «Программы»',
        moveFirstSub: 'Saba-chan открыта из образа диска или временного места. Перетащите её в папку «Программы» и откройте оттуда.',
        unsupportedOs: 'Не для этой ОС',
        unsupportedArch: 'Не для этого CPU',
        welcome: 'Добро пожаловать в установщик Saba-chan',
        modeInstall: 'Установка',
        modeUninstall: 'Удаление',
        stepStart: 'Начало',
        stepSettings: 'Настройки',
        stepInstall: 'Установка',
        stepDone: 'Готово',
        btnNext: 'Далее',
        labelPath: 'Путь установки',
        labelComponents: 'Расположение модулей и расширений',
        hintComponents: 'Модули и расширения будут в папках modules и extensions внутри этой папки. Уже установленные файлы будут перенесены.',
        compNotAbsolute: 'Укажите полный путь, например D:\\Games\\Saba.',
        compRootOrHome: 'Выберите папку внутри диска или папки пользователя, а не сам диск или папку пользователя.',
        compOverlap: 'Эта папка пересекается с текущей папкой модулей или расширений. Выберите другую.',
        compInsideInstall: 'Эта папка внутри папки установки или папки модулей и расширений окажутся друг в друге. Выберите другую.',
        compInvalid: 'Saba-chan не может использовать эту папку для модулей и расширений. Выберите другую.',
        errComponentsDir: 'Не удалось перенести модули и расширения. Ничего не изменено. Выберите другую папку или повторите попытку.',
        errInstallDir: 'В эту папку нельзя установить программу. Выберите папку, которая не является корнем диска, домашней папкой или папкой настроек.',
        labelModules: 'Игровые модули',
        hintModules: 'Можно добавить позже.',
        labelChannel: 'Канал обновлений',
        channelStable: 'Стабильный',
        channelRc: 'Предварительный (кандидат в релиз)',
        channelBeta: 'Бета (раньше всех, может быть нестабильной)',
        hintChannel: 'Устанавливается последняя версия выбранного канала, и дальнейшие обновления тоже приходят из него. После установки канал можно изменить в настройках приложения.',
        labelOptions: 'Параметры',
        labelDesktop: 'Ярлык на рабочем столе',
        labelStartMenu: 'Ярлык в меню «Пуск»',
        privacyNotice: 'Saba-chan отправляет анонимный ID и номера версий на сервер Saba-chan, чтобы проверять обновления и показывать списки модулей и расширений. Автоматическую проверку обновлений можно отключить в настройках приложения. Функции Discord отправляют данные только после подключения. Политика конфиденциальности: saba-chan.online/legal/privacy',
        btnInstall: 'Установить',
        installing: 'Установка...',
        installSub: 'Пожалуйста, подождите...',
        preparing: 'Подготовка...',
        fetchingRelease: 'Проверка последней версии...',
        modulesUnavailable: 'Не удалось загрузить список игровых модулей. Модули можно добавить в приложении после установки.',
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
        errElsewhere: 'Saba-chan уже установлен в другой папке. Сначала обновите или удалите ту установку (на одном ПК поддерживается только одна).',
        errDaemonRunning: 'Запущен Saba-chan, и установщик не может определить, к какой установке он относится. Сначала закройте Saba-chan и попробуйте снова.',
        launchFailed: 'Не удалось запустить Saba-chan. Запустите его из меню «Пуск» или с ярлыка на рабочем столе.',
        installFailed: 'Ошибка установки',
        fetchFailed: 'Не удалось получить информацию о версиях',
        errChannelOlder: 'Последняя версия этого канала старше этого установщика, поэтому установить её из этого канала нельзя. Выберите другой канал.',
        uninstallTitle: 'Удаление Saba-chan',
        uninstallDesc: 'Будут удалены программа Saba-chan и все созданные ею файлы: среды выполнения, модули, кэши, журналы, временные файлы, ярлыки и записи реестра. Запущенные серверы сначала останавливаются. Ниже выберите, что сохранить.',
        uninstallKeepInstances: 'Сохранить серверы',
        uninstallKeepInstancesHint: 'Сохраняет файлы серверов, миры, резервные копии и настройки каждого сервера для переустановки.',
        uninstallKeepSettings: 'Сохранить настройки',
        uninstallKeepSettingsHint: 'Сохраняет настройки приложения и подключение к Discord для переустановки.',
        uninstallPath: 'Путь установки: {{path}}',
        btnCancel: 'Отмена',
        btnUninstall: 'Удалить',
        uninstalling: 'Удаление...',
        uninstallSub: 'Пожалуйста, подождите...',
        uninstallComplete: 'Удаление завершено!',
        uninstallCompleteSub: 'Saba-chan был удалён.',
        uninstallCompletePartial: 'Удаление завершено, но некоторые файлы остались',
        uninstallCompletePartialSub: 'Их не удалось удалить. Закройте программы, которые их используют, и удалите их вручную:',
        uninstallWarnServersUnlisted: 'Не удалось получить список запущенных серверов, поэтому они были остановлены принудительно ({{detail}}).',
        uninstallWarnServerForceStopped: 'Сервер {{server}} не завершился корректно и был остановлен принудительно ({{detail}}). Изменения после последнего сохранения могли потеряться.',
        uninstallWarnDaemonUnanswered: 'Демон не ответил, поэтому запущенные серверы были остановлены принудительно. Изменения после последнего сохранения могли потеряться.',
        uninstallFailed: 'Ошибка удаления',
        labelLanguage: 'Язык',
        // Install progress
        progressPrepare: 'Создание папки установки...',
        progressElevate: 'Запрос прав администратора...',
        progressFetch: 'Получение информации о версии...',
        progressDownload: 'Загрузка...',
        progressExtract: 'Извлечение...',
        progressComponentsDir: 'Перемещение модулей и расширений...',
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
        uninstallProgressSandbox: 'Удаление профилей изоляции и прав на папки...',
        uninstallProgressShortcuts: 'Удаление ярлыков...',
        uninstallProgressFiles: 'Удаление файлов установки...',
        uninstallProgressData: 'Удаление пользовательских данных и сред выполнения...',
        uninstallProgressDataKeep: 'Удаление сред выполнения (выбранное сохраняется)...',
        uninstallProgressTemp: 'Очистка временных файлов...',
        uninstallProgressModules: 'Очистка папки модулей...',
        uninstallProgressModulesKeep: 'Сохранение настроек модулей...',
        uninstallProgressRegistry: 'Удаление записей реестра...',
        uninstallProgressSelfDelete: 'Планирование самоудаления...',
    },
    de: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: 'Einrichten',
        stepPick: 'Auswahl',
        stepPrepare: 'Einrichten',
        welcomeSetup: 'Saba-chan wird eingerichtet',
        welcomeSetupSub: 'Saba-chan lädt Spielmodule und richtet Python und den Befehl saba-chan ein. Das passiert nur einmal.',
        labelSetup: 'Was auf diesem Mac eingerichtet wird',
        setupPython: 'Python-Laufzeit',
        setupCommand: 'Der Befehl saba-chan (für Terminal und KI-Agenten)',
        setupModules: 'Die gewählten Spielmodule und die Erweiterungen, die sie brauchen',
        hintSetup: 'Saba-chan bleibt in der App im Ordner „Programme“. Ein Installationsort wird nicht gewählt.',
        hintModulesSetup: 'Nur Spiele, die auf diesem Mac laufen, sind wählbar. Mehr gibt es später im Saba Storage.',
        btnSetup: 'Starten',
        setupRunning: 'Wird eingerichtet!',
        setupSub: 'Das passiert nur einmal.',
        setupCompleteTitle: 'Saba-chan ist bereit',
        setupCompleteSub: 'Der Befehl saba-chan funktioniert jetzt in einem neuen Terminal-Fenster.',
        progressCommand: 'Befehl saba-chan wird registriert...',
        moveFirst: 'Bitte zuerst nach „Programme“ verschieben',
        moveFirstSub: 'Saba-chan wurde vom Disk-Image oder einem temporären Ort geöffnet. Zieh die App in den Ordner „Programme“ und öffne sie dort.',
        unsupportedOs: 'Nicht für dieses System',
        unsupportedArch: 'Nicht für diese CPU',
        welcome: 'Willkommen beim Saba-chan Installationsprogramm',
        modeInstall: 'Installieren',
        modeUninstall: 'Deinstallieren',
        stepStart: 'Start',
        stepSettings: 'Einstellungen',
        stepInstall: 'Installieren',
        stepDone: 'Fertig',
        btnNext: 'Weiter',
        labelPath: 'Installationsort',
        labelComponents: 'Speicherort für Module und Erweiterungen',
        hintComponents: 'Module und Erweiterungen kommen hier in die Ordner modules und extensions. Bereits installierte Dateien werden verschoben.',
        compNotAbsolute: 'Wähle einen vollständigen Pfad, z. B. D:\\Games\\Saba.',
        compRootOrHome: 'Wähle einen Ordner innerhalb eines Laufwerks oder deines Benutzerordners, nicht das Laufwerk oder den Benutzerordner selbst.',
        compOverlap: 'Dieser Ordner überschneidet sich mit dem aktuellen Modul- oder Erweiterungsordner. Wähle einen anderen.',
        compInsideInstall: 'Dieser Ordner liegt im Installationsordner oder würde Modul- und Erweiterungsordner ineinander legen. Wähle einen anderen.',
        compInvalid: 'Saba-chan kann diesen Ordner nicht für Module und Erweiterungen verwenden. Wähle einen anderen.',
        errComponentsDir: 'Module und Erweiterungen konnten nicht verschoben werden. Es wurde nichts geändert. Wähle einen anderen Ordner oder versuche es erneut.',
        errInstallDir: 'In diesen Ordner kann nicht installiert werden. Wähle einen Ordner, der kein Laufwerksstamm, kein Benutzerordner und nicht der Einstellungsordner ist.',
        labelModules: 'Spielmodule',
        hintModules: 'Weitere können später hinzugefügt werden.',
        labelChannel: 'Update-Kanal',
        channelStable: 'Stabil',
        channelRc: 'Vorschau (Release Candidate)',
        channelBeta: 'Beta (früh, evtl. instabil)',
        hintChannel: 'Installiert die neueste Version dieses Kanals; spätere Updates kommen ebenfalls von dort. Nach der Installation lässt er sich in den App-Einstellungen ändern.',
        labelOptions: 'Optionen',
        labelDesktop: 'Desktop-Verknüpfung',
        labelStartMenu: 'Startmenü-Verknüpfung',
        privacyNotice: 'Saba-chan sendet eine anonyme ID und Versionsnummern an den Saba-chan-Server, um nach Updates zu suchen und die Modul- und Erweiterungslisten anzuzeigen. Die automatische Update-Suche kannst du in den App-Einstellungen ausschalten. Discord-Funktionen senden erst Daten, wenn du sie verbindest. Datenschutzerklärung: saba-chan.online/legal/privacy',
        btnInstall: 'Installieren',
        installing: 'Installation...',
        installSub: 'Bitte warten...',
        preparing: 'Vorbereitung...',
        fetchingRelease: 'Neueste Version wird geprüft...',
        modulesUnavailable: 'Die Liste der Spielmodule konnte nicht geladen werden. Du kannst Module nach der Installation in der App hinzufügen.',
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
        errElsewhere: 'Saba-chan ist bereits in einem anderen Ordner installiert. Aktualisiere oder entferne zuerst diese Installation (pro PC wird nur eine unterstützt).',
        errDaemonRunning: 'Saba-chan läuft, und der Installer kann nicht erkennen, zu welcher Installation es gehört. Beende Saba-chan zuerst und versuche es erneut.',
        launchFailed: 'Saba-chan konnte nicht gestartet werden. Starte es über das Startmenü oder die Desktopverknüpfung.',
        installFailed: 'Installation fehlgeschlagen',
        fetchFailed: 'Versionsinformationen konnten nicht abgerufen werden',
        errChannelOlder: 'Die neueste Version dieses Kanals ist älter als dieses Installationsprogramm und lässt sich daher nicht über diesen Kanal installieren. Wähle einen anderen Kanal.',
        uninstallTitle: 'Saba-chan deinstallieren',
        uninstallDesc: 'Entfernt das Programm Saba-chan und alle Dateien, die es angelegt hat: Laufzeitumgebungen, Module, Caches, Protokolle, temporäre Dateien, Verknüpfungen und Registrierungseinträge. Laufende Server werden zuerst beendet. Wählen Sie unten, was erhalten bleibt.',
        uninstallKeepInstances: 'Server behalten',
        uninstallKeepInstancesHint: 'Behält Serverdateien, Welten, Backups und die Einstellungen jedes Servers für eine Neuinstallation.',
        uninstallKeepSettings: 'Einstellungen behalten',
        uninstallKeepSettingsHint: 'Behält die App-Einstellungen und die Discord-Verbindung für eine Neuinstallation.',
        uninstallPath: 'Installationsort: {{path}}',
        btnCancel: 'Abbrechen',
        btnUninstall: 'Deinstallieren',
        uninstalling: 'Deinstallation...',
        uninstallSub: 'Bitte warten...',
        uninstallComplete: 'Deinstallation abgeschlossen!',
        uninstallCompleteSub: 'Saba-chan wurde entfernt.',
        uninstallCompletePartial: 'Deinstalliert, aber einige Dateien sind übrig',
        uninstallCompletePartialSub: 'Diese konnten nicht gelöscht werden. Schließe Programme, die sie verwenden, und lösche sie manuell:',
        uninstallWarnServersUnlisted: 'Die Liste der laufenden Server konnte nicht abgerufen werden, daher wurden sie zwangsweise beendet ({{detail}}).',
        uninstallWarnServerForceStopped: 'Server {{server}} wurde nicht sauber beendet und zwangsweise gestoppt ({{detail}}). Änderungen seit dem letzten Speichern können verloren sein.',
        uninstallWarnDaemonUnanswered: 'Der Daemon hat nicht geantwortet, daher wurden laufende Server zwangsweise gestoppt. Änderungen seit dem letzten Speichern können verloren sein.',
        uninstallFailed: 'Deinstallation fehlgeschlagen',
        labelLanguage: 'Sprache',
        // Install progress
        progressPrepare: 'Installationsverzeichnis wird erstellt...',
        progressElevate: 'Administratorrechte werden angefordert...',
        progressFetch: 'Versionsinformationen werden abgerufen...',
        progressDownload: 'Herunterladen...',
        progressExtract: 'Entpacken...',
        progressComponentsDir: 'Module und Erweiterungen werden verschoben...',
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
        uninstallProgressSandbox: 'Isolationsprofile und Ordnerrechte werden entfernt...',
        uninstallProgressShortcuts: 'Verknüpfungen werden entfernt...',
        uninstallProgressFiles: 'Installationsdateien werden entfernt...',
        uninstallProgressData: 'Benutzerdaten und Laufzeitumgebungen werden entfernt...',
        uninstallProgressDataKeep: 'Laufzeitumgebungen werden entfernt (Ausgewähltes bleibt)...',
        uninstallProgressTemp: 'Temporäre Dateien werden bereinigt...',
        uninstallProgressModules: 'Modulverzeichnis wird bereinigt...',
        uninstallProgressModulesKeep: 'Moduleinstellungen werden beibehalten...',
        uninstallProgressRegistry: 'Registrierungseinträge werden entfernt...',
        uninstallProgressSelfDelete: 'Selbstlöschung wird geplant...',
    },
    fr: {
        // 준비 모드 (맥 앱을 처음 켤 때)
        modeSetup: 'Préparation',
        stepPick: 'Choix',
        stepPrepare: 'Préparation',
        welcomeSetup: 'Préparons Saba-chan',
        welcomeSetupSub: 'Saba-chan télécharge les modules de jeu et prépare Python et la commande saba-chan. Cela ne se fait qu’une fois.',
        labelSetup: 'Ce qui est préparé sur ce Mac',
        setupPython: 'Environnement Python',
        setupCommand: 'La commande saba-chan (pour le Terminal et les agents IA)',
        setupModules: 'Les modules de jeu choisis et les extensions dont ils ont besoin',
        hintSetup: 'Saba-chan reste dans l’app du dossier Applications. Aucun emplacement à choisir.',
        hintModulesSetup: 'Seuls les jeux qui tournent sur ce Mac peuvent être choisis. D’autres sont disponibles plus tard dans Saba Storage.',
        btnSetup: 'Commencer',
        setupRunning: 'Préparation !',
        setupSub: 'Cela ne se fait qu’une fois.',
        setupCompleteTitle: 'Saba-chan est prête',
        setupCompleteSub: 'La commande saba-chan fonctionne maintenant dans une nouvelle fenêtre du Terminal.',
        progressCommand: 'Enregistrement de la commande saba-chan...',
        moveFirst: 'Déplacez d’abord Saba-chan dans Applications',
        moveFirstSub: 'Saba-chan a été ouverte depuis l’image disque ou un emplacement temporaire. Glissez-la dans le dossier Applications, puis ouvrez-la depuis là.',
        unsupportedOs: 'Pas pour ce système',
        unsupportedArch: 'Pas pour ce CPU',
        welcome: 'Bienvenue dans l\'installateur de Saba-chan',
        modeInstall: 'Installer',
        modeUninstall: 'Désinstaller',
        stepStart: 'Début',
        stepSettings: 'Paramètres',
        stepInstall: 'Installer',
        stepDone: 'Terminé',
        btnNext: 'Suivant',
        labelPath: 'Emplacement d\'installation',
        labelComponents: 'Emplacement des modules et extensions',
        hintComponents: 'Les modules et extensions vont dans les dossiers modules et extensions d\'ici. Les fichiers déjà installés sont déplacés.',
        compNotAbsolute: 'Choisissez un chemin complet, par exemple D:\\Games\\Saba.',
        compRootOrHome: 'Choisissez un dossier à l\'intérieur d\'un lecteur ou de votre dossier utilisateur, pas le lecteur ni le dossier utilisateur lui-même.',
        compOverlap: 'Ce dossier chevauche le dossier actuel des modules ou extensions. Choisissez-en un autre.',
        compInsideInstall: 'Ce dossier est dans le dossier d\'installation ou placerait les modules et extensions l\'un dans l\'autre. Choisissez-en un autre.',
        compInvalid: 'Saba-chan ne peut pas utiliser ce dossier pour les modules et extensions. Choisissez-en un autre.',
        errComponentsDir: 'Impossible de déplacer les modules et extensions. Rien n\'a été modifié. Choisissez un autre dossier ou réessayez.',
        errInstallDir: 'Impossible d\'installer dans ce dossier. Choisissez un dossier qui n\'est ni la racine d\'un lecteur, ni votre dossier personnel, ni le dossier des paramètres.',
        labelModules: 'Modules de jeu',
        hintModules: 'Vous pourrez en ajouter d\'autres plus tard.',
        labelChannel: 'Canal de mise à jour',
        channelStable: 'Stable',
        channelRc: 'Aperçu (version candidate)',
        channelBeta: 'Bêta (en avance, peut être instable)',
        hintChannel: 'Installe la dernière version de ce canal ; les mises à jour suivantes en viendront aussi. Vous pourrez le changer dans les paramètres de l\'application après l\'installation.',
        labelOptions: 'Options',
        labelDesktop: 'Raccourci sur le bureau',
        labelStartMenu: 'Raccourci dans le menu Démarrer',
        privacyNotice: 'Saba-chan envoie un identifiant anonyme et les numéros de version au serveur Saba-chan pour rechercher les mises à jour et afficher les listes de modules et d\'extensions. Vous pouvez désactiver la recherche automatique des mises à jour dans les paramètres de l\'app. Les fonctions Discord n\'envoient des données qu\'après leur connexion. Politique de confidentialité : saba-chan.online/legal/privacy',
        btnInstall: 'Installer',
        installing: 'Installation en cours...',
        installSub: 'Veuillez patienter...',
        preparing: 'Préparation...',
        fetchingRelease: 'Vérification de la dernière version...',
        modulesUnavailable: "Impossible de charger la liste des modules de jeu. Tu pourras ajouter des modules dans l'app après l'installation.",
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
        errElsewhere: 'Saba-chan est déjà installé dans un autre dossier. Mettez à jour ou désinstallez d\'abord cette installation (une seule installation par PC).',
        errDaemonRunning: 'Saba-chan est en cours d\'exécution et le programme d\'installation ne sait pas à quelle installation il appartient. Quittez d\'abord Saba-chan, puis réessayez.',
        launchFailed: 'Impossible de démarrer Saba-chan. Lancez-le depuis le menu Démarrer ou le raccourci du bureau.',
        installFailed: 'Échec de l\'installation',
        fetchFailed: 'Impossible de récupérer les versions',
        errChannelOlder: 'La dernière version de ce canal est plus ancienne que ce programme d\'installation ; elle ne peut donc pas être installée depuis ce canal. Choisissez un autre canal.',
        uninstallTitle: 'Désinstaller Saba-chan',
        uninstallDesc: 'Ceci supprime le programme Saba-chan et tous les fichiers qu\'il a créés : environnements, modules, caches, journaux, fichiers temporaires, raccourcis et entrées du registre. Les serveurs en cours sont d\'abord arrêtés. Choisissez ci-dessous ce qu\'il faut conserver.',
        uninstallKeepInstances: 'Conserver les serveurs',
        uninstallKeepInstancesHint: 'Conserve les fichiers des serveurs, les mondes, les copies de sécurité et les paramètres de chaque serveur pour une réinstallation.',
        uninstallKeepSettings: 'Conserver les paramètres',
        uninstallKeepSettingsHint: 'Conserve les paramètres de l\'application et la connexion Discord pour une réinstallation.',
        uninstallPath: 'Emplacement d\'installation : {{path}}',
        btnCancel: 'Annuler',
        btnUninstall: 'Désinstaller',
        uninstalling: 'Désinstallation...',
        uninstallSub: 'Veuillez patienter...',
        uninstallComplete: 'Désinstallation terminée !',
        uninstallCompleteSub: 'Saba-chan a été supprimé.',
        uninstallCompletePartial: 'Désinstallé, mais certains fichiers restent',
        uninstallCompletePartialSub: 'Ils n\'ont pas pu être supprimés. Fermez les programmes qui les utilisent et supprimez-les manuellement :',
        uninstallWarnServersUnlisted: 'Impossible d\'obtenir la liste des serveurs en cours, ils ont donc été arrêtés de force ({{detail}}).',
        uninstallWarnServerForceStopped: 'Le serveur {{server}} ne s\'est pas arrêté proprement et a été arrêté de force ({{detail}}). Les changements depuis la dernière sauvegarde peuvent être perdus.',
        uninstallWarnDaemonUnanswered: 'Le démon n\'a pas répondu, les serveurs en cours ont donc été arrêtés de force. Les changements depuis la dernière sauvegarde peuvent être perdus.',
        uninstallFailed: 'Échec de la désinstallation',
        labelLanguage: 'Langue',
        // Install progress
        progressPrepare: 'Création du répertoire d\'installation...',
        progressElevate: 'Demande de privilèges administrateur...',
        progressFetch: 'Récupération des informations de version...',
        progressDownload: 'Téléchargement...',
        progressExtract: 'Extraction...',
        progressComponentsDir: 'Déplacement des modules et extensions...',
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
        uninstallProgressSandbox: 'Suppression des profils d\'isolation et des droits sur les dossiers...',
        uninstallProgressShortcuts: 'Suppression des raccourcis...',
        uninstallProgressFiles: 'Suppression des fichiers d\'installation...',
        uninstallProgressData: 'Suppression des données utilisateur et des environnements d\'exécution...',
        uninstallProgressDataKeep: 'Suppression des environnements (ce que vous avez choisi est conservé)...',
        uninstallProgressTemp: 'Nettoyage des fichiers temporaires...',
        uninstallProgressModules: 'Nettoyage du répertoire des modules...',
        uninstallProgressModulesKeep: 'Conservation des paramètres des modules...',
        uninstallProgressRegistry: 'Suppression des entrées de registre...',
        uninstallProgressSelfDelete: 'Planification de l\'auto-suppression...',
    },
};

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
