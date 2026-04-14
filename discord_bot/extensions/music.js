/**
 * 🎵 사바쨩 Music Extension
 * 
 * 이스터에그 / 추가 기능으로 포함된 음성 채널 음악 재생 익스텐션.
 * 게임 서버 관리와는 별개로, 디스코드 음성 채널에서 유튜브 음악을 재생합니다.
 * 
 * 의존성:
 *   - Node.js (extensions/music/package.json): @discordjs/voice, opusscript, play-dl, sodium-native
 *     → 익스텐션 활성화 시 music_deps.py 가 npm install 로 자동 설치
 *   - Python (extensions/music/music_deps.py): ffmpeg, yt-dlp
 */

const i18n = require('../i18n');
const { spawn, execSync } = require('child_process');
const { PassThrough } = require('stream');
const path = require('path');
const fs = require('fs');
const { ActionRowBuilder, ButtonBuilder, ButtonStyle } = require('discord.js');
const { getSabaDataDir } = require('../utils/constants');
const channelUI = require('./musicChannelUI');

// ── Lazy imports (패키지 미설치 시 graceful fallback) ──
let voice, playDl;
let musicAvailable = false;
let ffmpegPath = 'ffmpeg';
let ytDlpPath = 'yt-dlp';

// ── 환경변수 플래그 원본값 보존 (cleanup 시 복원) ──
let _originalNodePath = process.env.NODE_PATH;
let _originalFfmpegPath = process.env.FFMPEG_PATH;

/**
 * music_deps.py 의 위치 후보를 반환합니다.
 */
function getMusicDepsScriptCandidates() {
    const candidates = [];
    if (process.env.SABA_EXTENSIONS_DIR) {
        candidates.push(path.join(process.env.SABA_EXTENSIONS_DIR, 'music', 'music_deps.py'));
    }
    candidates.push(path.join(getSabaDataDir(), 'extensions', 'music', 'music_deps.py'));
    candidates.push(path.resolve(__dirname, '..', '..', '..', 'saba-chan-extensions', 'music', 'music_deps.py'));
    return candidates;
}

/**
 * .deps-resolved.json 이 없을 때 Python music_deps.py 를 직접 실행하여
 * 의존성을 검사하고 .deps-resolved.json 을 생성합니다.
 * 생성된 결과를 반환하거나, 실패 시 null 을 반환합니다.
 */
function tryRunMusicDeps() {
    const scriptCandidates = getMusicDepsScriptCandidates();
    let scriptPath = null;
    for (const p of scriptCandidates) {
        if (fs.existsSync(p)) { scriptPath = p; break; }
    }
    if (!scriptPath) {
        console.warn('[Music] music_deps.py not found in any candidate path');
        return null;
    }

    // Python 실행 파일 결정 — 사바쨩 격리 환경 우선, 시스템 Python fallback 없음
    const pythonCandidates = [];
    if (process.platform === 'win32') {
        // 사바쨩 프로덕션 python-env (설치 루트 기준, 최우선)
        const installRoot = path.resolve(__dirname, '..', '..');
        const prodPython = path.join(installRoot, 'python-env', 'Scripts', 'python.exe');
        if (fs.existsSync(prodPython)) pythonCandidates.push(prodPython);
        // venv (개발 환경)
        const venvPython = path.resolve(__dirname, '..', '..', '.venv', 'Scripts', 'python.exe');
        if (fs.existsSync(venvPython)) pythonCandidates.push(venvPython);
    } else {
        // Linux: 사바쨩 격리 venv
        const installRoot = path.resolve(__dirname, '..', '..');
        const linuxVenv = path.join(installRoot, 'python-env', 'bin', 'python3');
        if (fs.existsSync(linuxVenv)) pythonCandidates.push(linuxVenv);
        const devVenv = path.resolve(__dirname, '..', '..', '.venv', 'bin', 'python3');
        if (fs.existsSync(devVenv)) pythonCandidates.push(devVenv);
    }
    // 격리 환경을 찾지 못한 경우 에러 — 시스템 Python 사용 금지 (포터블 원칙)
    if (pythonCandidates.length === 0) {
        console.error('[Music] No saba-chan isolated Python found. Install portable Python via the installer. System Python will NOT be used.');
        return null;
    }

    for (const pyCmd of pythonCandidates) {
        try {
            // SABA_EXTENSIONS_DIR 을 Python에 전달하여 올바른 위치에 .deps-resolved.json 기록
            const spawnEnv = { ...process.env };
            if (!spawnEnv.SABA_EXTENSIONS_DIR) {
                spawnEnv.SABA_EXTENSIONS_DIR = path.join(getSabaDataDir(), 'extensions');
            }
            const result = require('child_process').spawnSync(
                pyCmd,
                [scriptPath, 'check_dependencies'],
                {
                    input: '{}',
                    stdio: ['pipe', 'pipe', 'pipe'],
                    timeout: 30000,
                    windowsHide: true,
                    env: spawnEnv,
                }
            );
            if (result.error) continue;
            if (result.status !== 0) {
                const stderr = result.stderr?.toString().trim();
                if (stderr) console.warn(`[Music] music_deps.py stderr: ${stderr}`);
                continue;
            }
            const output = result.stdout?.toString().trim();
            if (!output) continue;
            const parsed = JSON.parse(output);
            console.log(`[Music] music_deps.py executed successfully (python: ${pyCmd})`);
            // music_deps.py 가 .deps-resolved.json 을 이미 기록했으므로,
            // 다시 loadDepsResolved() 로 읽을 수도 있지만 파싱된 결과를 직접 반환
            return parsed.dependencies || parsed;
        } catch (e) {
            // 이 python 후보로 실패 — 다음 시도
        }
    }

    console.warn('[Music] Failed to run music_deps.py with any Python interpreter');
    return null;
}

/**
 * Python music_deps.py 가 기록한 .deps-resolved.json 에서 바이너리 경로를 불러옵니다.
 * 파일 위치: <extensions_dir>/music/.deps-resolved.json
 */
function loadDepsResolved() {
    const candidates = [];
    // 1. SABA_EXTENSIONS_DIR 환경변수
    if (process.env.SABA_EXTENSIONS_DIR) {
        candidates.push(path.join(process.env.SABA_EXTENSIONS_DIR, 'music', '.deps-resolved.json'));
    }
    // 2. 플랫폼별 기본 경로 (SSOT: getSabaDataDir)
    candidates.push(path.join(getSabaDataDir(), 'extensions', 'music', '.deps-resolved.json'));
    // 3. 개발 환경 — 워크스페이스 인접 폴더
    const devPath = path.resolve(__dirname, '..', '..', '..', 'saba-chan-extensions', 'music', '.deps-resolved.json');
    candidates.push(devPath);

    for (const p of candidates) {
        try {
            if (fs.existsSync(p)) {
                const data = JSON.parse(fs.readFileSync(p, 'utf8'));
                console.log(`[Music] Loaded deps from: ${p}`);
                return data;
            }
        } catch (_) {}
    }
    return null;
}

/**
 * Music extension 디렉토리 (package.json 존재) 를 반환합니다.
 * node_modules 설치 여부와 무관하게, 익스텐션 자체가 존재하는 디렉토리를 찾습니다.
 * @returns {string|null}
 */
function getMusicExtDir() {
    const candidates = [];
    if (process.env.SABA_EXTENSIONS_DIR) {
        candidates.push(path.join(process.env.SABA_EXTENSIONS_DIR, 'music'));
    }
    candidates.push(path.join(getSabaDataDir(), 'extensions', 'music'));
    candidates.push(path.resolve(__dirname, '..', '..', '..', 'saba-chan-extensions', 'music'));

    for (const dir of candidates) {
        if (fs.existsSync(path.join(dir, 'package.json'))) {
            return dir;
        }
    }
    return null;
}

/**
 * Music extension의 node_modules 가 설치된 디렉토리를 반환합니다.
 * node_modules/opusscript 를 마커로 사용합니다 (핵심 보조 패키지).
 * @returns {string|null} 익스텐션 디렉토리 경로 (node_modules 존재 시)
 */
function getMusicExtNodeModules() {
    const dir = getMusicExtDir();
    if (dir && fs.existsSync(path.join(dir, 'node_modules', 'opusscript'))) {
        return dir;
    }
    return null;
}

/**
 * Music extension 초기화.
 * 모듈 로드 시 자동 실행되며, musicAvailable이 false인 경우 외부에서 재호출하여
 * 환경 구축 완료 후 재시작 없이 동적으로 활성화할 수 있습니다.
 * @returns {boolean} 초기화 성공 여부
 */
function init() {
    if (musicAvailable) return true;
    try {
        // ── 1단계: 익스텐션 디렉토리 & node_modules 확인 ──
        // node_modules가 없으면 music_deps.py를 실행하여 npm install + yt-dlp 설치
        let musicExtDir = getMusicExtNodeModules();
        if (!musicExtDir) {
            const extDir = getMusicExtDir();
            if (extDir) {
                console.log(`[Music] node_modules not found in ${extDir}, running music_deps.py to bootstrap...`);
                tryRunMusicDeps();
                // npm install 완료 후 재확인
                musicExtDir = getMusicExtNodeModules();
            }
        }

        // ── 2단계: Python deps 경로 읽기 (ffmpeg, yt-dlp) ──
        const deps = loadDepsResolved();
        if (deps) {
            if (deps.ffmpeg && deps.ffmpeg.available && deps.ffmpeg.path) {
                ffmpegPath = deps.ffmpeg.path;
                if (!process.env.FFMPEG_PATH) process.env.FFMPEG_PATH = ffmpegPath;
                console.log(`[Music] FFmpeg path (from Python): ${ffmpegPath}`);
            }
            if (deps.yt_dlp && deps.yt_dlp.available && deps.yt_dlp.path) {
                ytDlpPath = deps.yt_dlp.path;
                console.log(`[Music] yt-dlp path (from Python): ${ytDlpPath}`);
            }
        } else {
            console.log('[Music] .deps-resolved.json not found, running music_deps.py to resolve...');
            const depsGenerated = tryRunMusicDeps();
            if (depsGenerated) {
                if (depsGenerated.ffmpeg && depsGenerated.ffmpeg.available && depsGenerated.ffmpeg.path) {
                    ffmpegPath = depsGenerated.ffmpeg.path;
                    if (!process.env.FFMPEG_PATH) process.env.FFMPEG_PATH = ffmpegPath;
                    console.log(`[Music] FFmpeg path (from deps check): ${ffmpegPath}`);
                }
                if (depsGenerated.yt_dlp && depsGenerated.yt_dlp.available && depsGenerated.yt_dlp.path) {
                    ytDlpPath = depsGenerated.yt_dlp.path;
                    console.log(`[Music] yt-dlp path (from deps check): ${ytDlpPath}`);
                }
            } else {
                console.warn('[Music] Could not resolve dependencies. ffmpeg/yt-dlp may not work.');
            }
        }

        // ── 2.5단계: ffmpeg-static fallback ──
        // Python deps에서 ffmpeg을 못 찾았으면 npm ffmpeg-static 패키지 사용
        if (ffmpegPath === 'ffmpeg') {
            try {
                const musicDir = getMusicExtDir();
                if (musicDir) {
                    const { createRequire } = require('module');
                    const staticPath = createRequire(path.join(musicDir, 'package.json'))('ffmpeg-static');
                    if (staticPath && fs.existsSync(staticPath)) {
                        ffmpegPath = staticPath;
                        process.env.FFMPEG_PATH = ffmpegPath;
                        console.log(`[Music] FFmpeg path (from ffmpeg-static): ${ffmpegPath}`);
                    }
                }
            } catch (e) {
                // ffmpeg-static 패키지 미설치 → npm install 자가복구 시도
                const musicDir = getMusicExtDir();
                if (musicDir) {
                    console.log('[Music] ffmpeg-static not found, attempting npm install for self-recovery...');
                    try {
                        execSync('npm install --omit=dev --no-fund --no-audit', {
                            cwd: musicDir,
                            stdio: ['ignore', 'pipe', 'pipe'],
                            timeout: 120_000,
                            windowsHide: true,
                        });
                        const { createRequire } = require('module');
                        const staticPath = createRequire(path.join(musicDir, 'package.json'))('ffmpeg-static');
                        if (staticPath && fs.existsSync(staticPath)) {
                            ffmpegPath = staticPath;
                            process.env.FFMPEG_PATH = ffmpegPath;
                            console.log(`[Music] FFmpeg path (self-recovered via npm install): ${ffmpegPath}`);
                        }
                    } catch (npmErr) {
                        console.warn('[Music] npm install self-recovery failed:', npmErr.message);
                    }
                }
            }
        }

        // ── 3단계: extension node_modules 에서 보조 패키지 로드 ──
        // @discordjs/voice 는 discord.js 와 동일 node_modules에 있어야 합니다.
        // opusscript, sodium-native, play-dl, prism-media 등은 extension 쪽에서 로드합니다.
        let extRequire = null;
        if (musicExtDir) {
            const { createRequire } = require('module');
            extRequire = createRequire(path.join(musicExtDir, 'package.json'));
            console.log(`[Music] Auxiliary deps from extension: ${musicExtDir}`);

            // voice 가 extension 의 encryption/opus 라이브러리를 찾을 수 있도록
            // extension 의 node_modules 를 NODE_PATH 에 추가
            const extNodeModules = path.join(musicExtDir, 'node_modules');
            const sep = process.platform === 'win32' ? ';' : ':';
            if (!process.env.NODE_PATH || !process.env.NODE_PATH.includes(extNodeModules)) {
                process.env.NODE_PATH = process.env.NODE_PATH
                    ? process.env.NODE_PATH + sep + extNodeModules
                    : extNodeModules;
                require('module').Module._initPaths();
                console.log(`[Music] Added extension node_modules to NODE_PATH`);
            }
        } else {
            // 익스텐션 node_modules가 없으면 음악 기능 불가 — musicAvailable 설정하지 않음
            // daemon.startup 훅(npm install)이 완료된 후 재호출 시 정상 로드됨
            console.warn('[Music] Music extension node_modules not found — init deferred until packages are installed');
            return false;
        }

        // ── prism-media FFmpeg 경로 등록 ──────────────────────────────
        // @discordjs/voice 는 내부적으로 prism-media 의 FFmpeg 을 사용합니다.
        // prism-media 는 process.env.FFMPEG_PATH 를 확인하지 않고
        // require('ffmpeg-static') / PATH 의 'ffmpeg' 만 탐색하므로,
        // 우리 ffmpegPath 를 직접 등록해야 합니다.
        try {
            const prism = extRequire ? extRequire('prism-media') : require('prism-media');
            const { spawnSync } = require('child_process');
            const probe = spawnSync(ffmpegPath, ['-h'], {
                windowsHide: true,
                timeout: 5000,
                stdio: ['ignore', 'pipe', 'pipe'],
            });
            if (!probe.error) {
                const output = Buffer.concat([probe.stdout, probe.stderr].filter(Boolean)).toString();
                const cachedInfo = { command: ffmpegPath, output };
                Object.defineProperty(cachedInfo, 'version', {
                    get() { return (/version (.+) Copyright/mi.exec(this.output) || [])[1] || 'unknown'; },
                    enumerable: true,
                });
                prism.FFmpeg.getInfo = () => cachedInfo;
                console.log(`[Music] Registered ffmpeg for prism-media: ${ffmpegPath}`);
            }
        } catch (e) {
            console.warn('[Music] Could not register ffmpeg with prism-media:', e.message);
        }

        voice = require('@discordjs/voice');
        // play-dl은 검색/메타데이터 전용으로 사용 (stream은 yt-dlp)
        try { playDl = extRequire ? extRequire('play-dl') : require('play-dl'); } catch (_) {}

        // ── 필수 바이너리 검증 (C4: musicAvailable 조건 강화) ──
        // ffmpeg 실제 실행 가능 여부 검증
        try {
            const ffProbe = require('child_process').spawnSync(ffmpegPath, ['-version'], {
                timeout: 5000, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
            });
            if (ffProbe.error) {
                console.error(`[Music] ffmpeg not executable at '${ffmpegPath}': ${ffProbe.error.message}`);
                console.warn('[Music] Extension loaded WITHOUT ffmpeg — playback will fail');
            }
        } catch (e) {
            console.error(`[Music] ffmpeg validation failed: ${e.message}`);
        }
        // yt-dlp 실제 실행 가능 여부 검증
        try {
            const ytProbe = require('child_process').spawnSync(ytDlpPath, ['--version'], {
                timeout: 5000, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
            });
            if (ytProbe.error) {
                console.error(`[Music] yt-dlp not executable at '${ytDlpPath}': ${ytProbe.error.message}`);
                console.warn('[Music] Extension loaded WITHOUT yt-dlp — search/playback will fail');
            }
        } catch (e) {
            console.error(`[Music] yt-dlp validation failed: ${e.message}`);
        }

        musicAvailable = true;
        console.log('[Music] Extension loaded successfully 🎵');
        return true;
    } catch (e) {
        console.warn('[Music] Extension not available:', e.message);
        console.warn('[Music] discord_bot/에 @discordjs/voice, 그리고 extensions/music/에 보조 패키지가 필요합니다.');
        return false;
    }
}

// 모듈 로드 시 첫 초기화 시도
init();

// ── Per-guild state ──
const guildQueues = new Map();
const guildLocks = new Map(); // 길드별 비동기 락 (동시 playNext 방지)

/**
 * 길드별 비동기 락 — playNext 등 동시에 하나만 실행되도록 보장
 * H7: 타임아웃 안전장치 — 30초 후 자동 release (교착 방지)
 */
function acquireLock(guildId) {
    if (!guildLocks.has(guildId)) {
        guildLocks.set(guildId, Promise.resolve());
    }
    let release;
    const prev = guildLocks.get(guildId);
    const next = new Promise((resolve) => { release = resolve; });
    guildLocks.set(guildId, prev.then(() => next));

    // 안전장치: release가 30초 내 호출되지 않으면 자동 해제
    let safetyTimer;
    const wrappedRelease = () => {
        clearTimeout(safetyTimer);
        release();
    };
    const lockPromise = prev.then(() => {
        safetyTimer = setTimeout(() => {
            console.warn(`[Music] Lock timeout for guild ${guildId}, force-releasing to prevent deadlock`);
            release();
        }, 30_000);
        return wrappedRelease;
    });
    return lockPromise;
}

// 기본 볼륨 (0.0 ~ 1.0)
const DEFAULT_VOLUME = 0.5;
// 기본 오디오 정규화 상태 (bot-config.json에서 로드)
let _defaultNormalize = true;
// 아무도 안 들으면 자동 퇴장 (ms)
const IDLE_TIMEOUT = 5 * 60 * 1000; // 5분
// 라디오 자동 추가 트랙의 requester 식별자
const RADIO_REQUESTER = '📻 Radio';
// 라디오 한 번에 채우는 곡 수
const RADIO_FILL_COUNT = 5;
// 자동 보충 트리거 임계값 (이하이면 보충)
const RADIO_REPLENISH_THRESHOLD = 2;
// 시드풀 최대 크기
const RADIO_SEED_POOL_MAX = 50;
// 스킵 판정 임계값 (ms) — 이 시간 이내에 넘기면 시드에서 제외
const RADIO_SKIP_THRESHOLD_MS = 5000;
// ffmpeg loudnorm 필터 — 트랙 간 볼륨 정규화 (EBU R128, 싱글패스 스트리밍)
// I=-14: 목표 라우드니스 -14 LUFS (스트리밍 표준)
// TP=-1: 트루피크 제한 -1 dBTP (클리핑 방지)
// LRA=11: 허용 라우드니스 레인지 11 LU
// 참고: linear=true는 투패스에서만 유효, 싱글패스에서는 정규화 효과를 약화시키므로 사용하지 않음
const LOUDNORM_FILTER = 'loudnorm=I=-14:TP=-1:LRA=11';

/**
 * 라디오가 자동으로 추가한 트랙인지 판별
 */
function isRadioTrack(track) {
    return track.requester === RADIO_REQUESTER;
}

/**
 * 수동 트랙을 삽입할 위치를 반환 — 라디오 트랙 앞에 삽입
 * 라디오 트랙이 아닌 마지막 위치 바로 뒤 = 첫 번째 라디오 트랙의 인덱스
 */
function findManualInsertIndex(tracks) {
    for (let i = 0; i < tracks.length; i++) {
        if (isRadioTrack(tracks[i])) return i;
    }
    return tracks.length;
}

/**
 * Guild별 Queue 객체 생성
 */
function createQueue(guildId) {
    return {
        guildId,
        tracks: [],         // { title, url, duration, requester }
        history: [],        // 이전 곡 최대 5개 (최근 → 오래된 순)
        _radioHistory: [],  // 라디오 전용 히스토리 (최대 30곡, 중복 방지용)
        _radioSeedPool: [], // 라디오 시드풀 — ON 이후 모든 곡 (추천 다양성용, 최대 50곡)
        current: null,
        connection: null,
        player: null,
        resource: null,
        volume: DEFAULT_VOLUME,
        loop: false,
        radio: false,       // 라디오 모드 — 큐가 비면 자동 추천곡 재생
        _radioFetching: false, // 라디오 곡 fetch 중 동시 요청 방지 플래그
        normalize: _defaultNormalize, // 오디오 정규화 (loudnorm) — bot-config에서 기본값 로드
        idleTimer: null,
        prefetchMap: new Map(), // Map<url, stream> — 대기열 전체 프리페치
        _startedAt: null,   // 현재 곡 재생 시작 timestamp (진행바용)
        _paused: false,
        _pausedElapsed: 0,  // 일시정지 시점까지의 경과 초
        _playNextPending: false, // enqueueAndPlay에서 동시 playNext 방지
        _activeStream: null,     // 현재 재생 중인 스트림 참조 (cleanup용)
        _adapterCreator: null,   // voice rejoin용 adapterCreator 보존
        _autoPauseCount: 0,      // autopaused 반복 감지 카운터
        _autoPauseResetTimer: null, // autopaused 카운터 리셋 타이머
        _rejoining: false,       // rejoin 진행 중 플래그
    };
}

function getQueue(guildId) {
    return guildQueues.get(guildId);
}

function getOrCreateQueue(guildId) {
    if (!guildQueues.has(guildId)) {
        guildQueues.set(guildId, createQueue(guildId));
    }
    return guildQueues.get(guildId);
}

/**
 * 깨진 voice connection 파괴 후 같은 채널에 재접속
 * UDP 소켓이 죽어서 autopaused 루프에 빠졌을 때 호출
 */
async function _rejoinVoice(guildId) {
    const queue = getQueue(guildId);
    if (!queue || !queue.connection || queue._rejoining) return false;
    queue._rejoining = true;

    try {
        const { channelId } = queue.connection.joinConfig;
        const adapterCreator = queue._adapterCreator;
        if (!channelId || !adapterCreator) {
            console.warn('[Music] Cannot rejoin — missing channelId or adapterCreator');
            return false;
        }

        console.log(`[Music] Destroying broken connection and rejoining channel ${channelId}`);

        // 기존 커넥션 파괴
        try { queue.connection.destroy(); } catch (_) {}

        // 새 커넥션 생성
        queue.connection = voice.joinVoiceChannel({
            channelId,
            guildId,
            adapterCreator,
            selfDeaf: true,
        });

        // Disconnected 핸들러 재등록
        _registerDisconnectHandler(queue, guildId);

        // Ready 대기
        await voice.entersState(queue.connection, voice.VoiceConnectionStatus.Ready, 15_000);
        console.log('[Music] Rejoined voice connection successfully');

        // player 재구독
        if (queue.player) {
            queue.connection.subscribe(queue.player);
            console.log('[Music] Player resubscribed to new connection');
        }

        queue._autoPauseCount = 0;
        return true;
    } catch (e) {
        console.error('[Music] Rejoin failed:', e.message);
        return false;
    } finally {
        queue._rejoining = false;
    }
}

/**
 * Disconnected 이벤트 핸들러 등록 (커넥션 생성/재생성 시 사용)
 */
function _registerDisconnectHandler(queue, guildId) {
    let reconnectAttempts = 0;
    const MAX_RECONNECT = 3;
    queue.connection.on(voice.VoiceConnectionStatus.Disconnected, async () => {
        if (reconnectAttempts >= MAX_RECONNECT) {
            console.warn(`[Music] Max reconnect attempts reached (${MAX_RECONNECT}), trying rejoin...`);
            const rejoined = await _rejoinVoice(guildId);
            if (!rejoined) {
                console.warn('[Music] Rejoin also failed, destroying queue');
                destroyQueue(guildId);
            }
            return;
        }
        reconnectAttempts++;
        const delay = Math.min(1000 * Math.pow(2, reconnectAttempts), 10000);
        console.log(`[Music] Disconnected, reconnect attempt ${reconnectAttempts}/${MAX_RECONNECT} (wait ${delay}ms)...`);
        try {
            await Promise.race([
                voice.entersState(queue.connection, voice.VoiceConnectionStatus.Signalling, delay + 5000),
                voice.entersState(queue.connection, voice.VoiceConnectionStatus.Connecting, delay + 5000),
            ]);
            reconnectAttempts = 0;
        } catch (_) {
            if (reconnectAttempts >= MAX_RECONNECT) {
                const rejoined = await _rejoinVoice(guildId);
                if (!rejoined) destroyQueue(guildId);
            }
        }
    });
}

function destroyQueue(guildId) {
    const queue = guildQueues.get(guildId);
    if (queue) {
        if (queue._autoPauseResetTimer) clearTimeout(queue._autoPauseResetTimer);
        if (queue.idleTimer) clearTimeout(queue.idleTimer);
        if (queue.player) queue.player.stop(true);
        cleanupPrefetch(queue);
        // H3: 재생 중인 스트림 버퍼 정리 (메모리 누수 방지)
        if (queue._activeStream) {
            try {
                if (queue._activeStream._ytdlp) queue._activeStream._ytdlp.kill();
                if (queue._activeStream._ffmpeg) queue._activeStream._ffmpeg.kill();
                queue._activeStream.destroy();
            } catch (_) {}
            queue._activeStream = null;
        }
        if (queue.connection) {
            try { queue.connection.destroy(); } catch (_) {}
        }
        guildQueues.delete(guildId);
        // 전용 채널 UI 갱신 (정지 상태 표시)
        channelUI.refreshQueue(guildId).catch(() => {});
    }
}

// ── Music command definitions (alias system 호환) ──
const MUSIC_COMMANDS = {
    play:    { handler: handlePlay,    needsVoice: true  },
    search:  { handler: handleSearch,  needsVoice: true  },
    pause:   { handler: handlePause,   needsVoice: true  },
    resume:  { handler: handleResume,  needsVoice: true  },
    skip:    { handler: handleSkip,    needsVoice: true  },
    stop:    { handler: handleStop,    needsVoice: true  },
    queue:   { handler: handleQueue,   needsVoice: false },
    np:      { handler: handleNowPlaying, needsVoice: false },
    volume:  { handler: handleVolume,  needsVoice: true  },
    shuffle: { handler: handleShuffle, needsVoice: true  },
    loop:    { handler: handleLoop,    needsVoice: true  },
    loopoff: { handler: handleLoopOff, needsVoice: true  },
    radio:   { handler: handleRadio,   needsVoice: true  },
    normalize: { handler: handleNormalize, needsVoice: false },
    help:    { handler: handleHelp,    needsVoice: false },
};

/**
 * 언어 무관 범용 명령어 별명 (어떤 언어 설정이든 항상 동작)
 * 언어별 별명은 i18n(bot:music_commands)에서 로드
 */
const UNIVERSAL_COMMAND_ALIASES = {
    play:    ['p'],
    search:  ['find'],
    skip:    ['s', 'next'],
    stop:    ['leave', 'disconnect', 'dc'],
    queue:   ['q', 'list'],
    np:      ['nowplaying', 'now'],
    volume:  ['vol', 'v'],
    shuffle: ['random'],
    loop:    ['repeat'],
    loopoff: ['unloop'],
    radio:   ['autoplay'],
    normalize: ['norm', 'loudnorm'],
};

/**
 * 언어 무관 범용 모듈 별명 (어떤 언어 설정이든 항상 동작)
 */
const UNIVERSAL_MODULE_ALIASES = ['music', 'dj'];

// ── i18n 기반 별명 캐시 (언어는 프로세스 수명 동안 불변) ──
let _cachedCommandAliases = null;
let _cachedModuleAliases = null;

/**
 * 현재 언어의 명령어 별명 + 범용 별명을 병합하여 반환
 * i18n(bot:music_commands)에서 언어별 별명을 로드하고,
 * UNIVERSAL_COMMAND_ALIASES를 추가로 병합합니다.
 * @returns {object} { cmdName: [alias1, alias2, ...], ... }
 */
function getEffectiveCommandAliases() {
    if (_cachedCommandAliases) return _cachedCommandAliases;

    const i18nCmds = i18n.t('bot:music_commands', { returnObjects: true });
    const hasI18n = i18nCmds && typeof i18nCmds === 'object' && !Array.isArray(i18nCmds)
        && typeof i18nCmds !== 'string'; // i18next가 키를 그대로 반환하면 string

    const merged = {};
    for (const cmdName of Object.keys(MUSIC_COMMANDS)) {
        const universal = UNIVERSAL_COMMAND_ALIASES[cmdName] || [];
        const fromI18n = (hasI18n && Array.isArray(i18nCmds[cmdName])) ? i18nCmds[cmdName] : [];

        // i18n 별명 → 범용 별명 순으로 병합 (중복 제거, 대소문자 무시)
        const seen = new Set();
        const allAliases = [];
        for (const a of [...fromI18n, ...universal]) {
            const lower = a.toLowerCase();
            if (!seen.has(lower)) {
                seen.add(lower);
                allAliases.push(a);
            }
        }
        merged[cmdName] = allAliases;
    }

    _cachedCommandAliases = merged;
    return merged;
}

/**
 * 현재 언어의 모듈 별명 + 범용 모듈 별명을 병합하여 반환
 * @returns {string[]}
 */
function getEffectiveModuleAliases() {
    if (_cachedModuleAliases) return _cachedModuleAliases;

    const i18nCmds = i18n.t('bot:music_commands', { returnObjects: true });
    const fromI18n = (i18nCmds && typeof i18nCmds === 'object' && Array.isArray(i18nCmds.module_aliases))
        ? i18nCmds.module_aliases : [];

    const seen = new Set();
    const merged = [];
    for (const a of [...fromI18n, ...UNIVERSAL_MODULE_ALIASES]) {
        const lower = a.toLowerCase();
        if (!seen.has(lower)) {
            seen.add(lower);
            merged.push(a);
        }
    }

    _cachedModuleAliases = merged;
    return merged;
}

// 하위 호환용 — 외부에서 참조하는 코드를 위해 getter로 제공
const DEFAULT_COMMAND_ALIASES = new Proxy({}, {
    get(_, prop) {
        const effective = getEffectiveCommandAliases();
        return effective[prop];
    },
    ownKeys() {
        return Object.keys(getEffectiveCommandAliases());
    },
    getOwnPropertyDescriptor(_, prop) {
        const effective = getEffectiveCommandAliases();
        if (prop in effective) return { configurable: true, enumerable: true, value: effective[prop] };
    },
    has(_, prop) {
        return prop in getEffectiveCommandAliases();
    },
});
const DEFAULT_MODULE_ALIASES = new Proxy([], {
    get(target, prop) {
        const effective = getEffectiveModuleAliases();
        if (prop === Symbol.iterator) return effective[Symbol.iterator].bind(effective);
        if (typeof prop === 'string' && !isNaN(prop)) return effective[Number(prop)];
        if (prop === 'length') return effective.length;
        if (prop === 'includes') return effective.includes.bind(effective);
        if (prop === 'map') return effective.map.bind(effective);
        if (prop === 'join') return effective.join.bind(effective);
        if (prop === 'filter') return effective.filter.bind(effective);
        if (prop === 'forEach') return effective.forEach.bind(effective);
        return effective[prop];
    },
});

/**
 * 명령어 별명 해석
 * 우선순위: 1. 정확한 명령어명 2. 사용자 커스텀 별명 3. i18n 언어별 + 범용 별명
 * @param {string} input - 사용자 입력
 * @param {object} customAliases - bot-config의 commandAliases.music
 * @returns {string|null} 실제 명령어 이름
 */
function resolveMusicCommand(input, customAliases = {}) {
    const lower = input.toLowerCase();
    
    // 1. 정확한 명령어 이름 매칭
    if (MUSIC_COMMANDS[lower]) return lower;
    
    // 2. 사용자 커스텀 별명 (bot-config.json)
    for (const [cmdName, aliasStr] of Object.entries(customAliases)) {
        if (typeof aliasStr === 'string' && aliasStr.trim().length > 0) {
            const aliases = aliasStr.split(',').map(a => a.trim().toLowerCase());
            if (aliases.includes(lower)) return cmdName;
        }
    }
    
    // 3. i18n 언어별 별명 + 범용 별명
    const effectiveAliases = getEffectiveCommandAliases();
    for (const [cmdName, aliases] of Object.entries(effectiveAliases)) {
        if (aliases.map(a => a.toLowerCase()).includes(lower)) return cmdName;
    }
    
    return null;
}

/**
 * 모듈 별명 확인 (이 메시지가 음악 명령인지)
 * @param {string} modulePart - prefix 뒤의 첫 번째 토큰
 * @param {object} botConfig - bot-config.json
 * @returns {boolean}
 */
function isMusicModule(modulePart, botConfig) {
    const lower = modulePart.toLowerCase();
    
    // i18n 언어별 + 범용 모듈 별명
    const effectiveModuleAliases = getEffectiveModuleAliases();
    if (effectiveModuleAliases.map(a => a.toLowerCase()).includes(lower)) return true;
    
    // 사용자 커스텀 모듈 별명
    const customAlias = (botConfig.moduleAliases?.music || '').trim();
    if (customAlias) {
        const aliases = customAlias.split(',').map(a => a.trim().toLowerCase());
        if (aliases.includes(lower)) return true;
    }
    
    return false;
}

/**
 * 릴레이 에이전트의 mock 메시지인지 판별
 * — member/guild 프로퍼티가 없으면 relay mock으로 간주
 */
function isRelayMessage(message) {
    return !message.member || !message.guild;
}

/**
 * 음성 채널 체크 유틸 — 사용자가 보이스룸에 있는지 확인하고 에러 메시지까지 처리
 * @returns {VoiceChannel|null} 사용자가 있는 음성 채널, 없으면 null (에러 메시지 이미 전송됨)
 */
async function requireVoiceChannel(message) {
    const voiceChannel = message.member?.voice?.channel;
    if (!voiceChannel) {
        await message.reply(i18n.t('bot:music.join_voice_first'));
        return null;
    }
    return voiceChannel;
}

/**
 * 바로가기 진입점 — "사바쨩 <유튜브URL>" 또는 "사바쨩 정지" (음악 재생 중일 때)
 * 모듈명 없이 prefix + URL/명령어만으로 음악을 제어합니다.
 * @returns {boolean} 처리했으면 true
 */
async function handleMusicShortcut(message, args, botConfig) {
    if (args.length === 0) return false;
    if (!musicAvailable) {
        // 패키지 미설치라도 음악 명령어라면 안내 메시지 표시 (IPC로 넘기지 않음)
        const firstArg = args[0];
        if (isYouTubeUrl(firstArg)) {
            await message.reply(i18n.t('bot:music.not_available'));
            return true;
        }
        if (args.length >= 2 && isMusicModule(firstArg, botConfig)) {
            await message.reply(i18n.t('bot:music.not_available'));
            return true;
        }
        // 숏컷 명령어 (재생, 정지 등) — 활성 큐가 없으므로 false 반환하되,
        // 음악 전용 명령어일 경우는 잡아야 함
        const customAliases = botConfig.commandAliases?.music || {};
        const commandName = resolveMusicCommand(firstArg, customAliases);
        if (commandName && (commandName === 'play' || commandName === 'search')) {
            await message.reply(i18n.t('bot:music.not_available'));
            return true;
        }
        return false;
    }
    
    // GUI에서 뮤직봇 비활성화 시 무시
    if (botConfig.musicEnabled === false) return false;

    // 릴레이 모드(mock message)에서는 음악 바로가기 스킵 → IPC 라우팅으로 넘김
    if (isRelayMessage(message)) return false;
    
    const firstArg = args[0];
    
    // "사바쨩 <유튜브URL>" → 바로 재생
    if (isYouTubeUrl(firstArg)) {
        if (!await requireVoiceChannel(message)) return true;
        await handlePlay(message, args, botConfig);
        return true;
    }
    
    // "사바쨩 정지/stop/leave/dc" → 음악이 재생 중이면 음악 정지 (i18n 별명 자동 반영)
    if (args.length === 1) {
        const customAliases = botConfig.commandAliases?.music || {};
        if (resolveMusicCommand(firstArg, customAliases) === 'stop') {
            if (hasActiveQueue(message.guild?.id)) {
                if (!await requireVoiceChannel(message)) return true;
                safeDelete(message);
                await handleStop(message);
                return true;
            }
        }
    }
    
    // "사바쨩 일시정지/계속/다음/대기열/지금/볼륨/섞기" 등도 음악 활성 시 바로 처리
    if (args.length >= 1 && hasActiveQueue(message.guild?.id)) {
        const customAliases = botConfig.commandAliases?.music || {};
        const commandName = resolveMusicCommand(firstArg, customAliases);
        if (commandName && commandName !== 'play' && commandName !== 'search' && commandName !== 'help') {
            const cmdDef = MUSIC_COMMANDS[commandName];
            if (cmdDef.needsVoice && !await requireVoiceChannel(message)) return true;
            safeDelete(message);
            await cmdDef.handler(message, args.slice(1), botConfig);
            return true;
        }
    }
    
    // "사바쨩 재생 <검색어>" / "사바쨩 검색 <검색어>" → 큐 유무와 무관하게 바로 처리
    if (args.length >= 2) {
        const customAliases = botConfig.commandAliases?.music || {};
        const commandName = resolveMusicCommand(firstArg, customAliases);
        if (commandName === 'play' || commandName === 'search') {
            if (!await requireVoiceChannel(message)) return true;
            safeDelete(message);
            try {
                await MUSIC_COMMANDS[commandName].handler(message, args.slice(1), botConfig);
            } catch (e) {
                console.error(`[Music] Shortcut ${commandName} error:`, e.message);
            }
            return true; // 에러가 나더라도 IPC로 넘기지 않음
        }
    }
    
    return false;
}

/**
 * 해당 길드에 음악이 활성 상태인지 확인
 */
function hasActiveQueue(guildId) {
    if (!guildId) return false;
    const queue = guildQueues.get(guildId);
    return !!(queue && (queue.current || queue.tracks.length > 0));
}

/**
 * 메인 진입점 — index.js의 messageCreate에서 호출
 * @param {Message} message - Discord message
 * @param {string[]} args - prefix 이후의 토큰 배열 [모듈, 명령어, ...나머지]
 * @param {object} botConfig - bot-config.json
 * @returns {boolean} 처리했으면 true (이후 IPC 라우팅 스킵)
 */
async function handleMusicMessage(message, args, botConfig) {
    if (args.length === 0) return false;
    
    // GUI에서 뮤직봇 비활성화 시 무시
    if (botConfig.musicEnabled === false) return false;
    
    const modulePart = args[0];
    if (!isMusicModule(modulePart, botConfig)) return false;

    // 릴레이 모드(mock message)에서는 음악 불가 — Discord 보이스 인프라 없음
    if (isRelayMessage(message)) {
        await message.reply(i18n.t('bot:music.not_available_relay'));
        return true;
    }
    
    // 패키지 미설치 시 안내
    if (!musicAvailable) {
        await message.reply(i18n.t('bot:music.not_available'));
        return true;
    }
    
    // 명령어 없이 모듈명만 입력 → 도움말
    if (args.length < 2) {
        await handleHelp(message, [], botConfig);
        return true;
    }
    
    const commandInput = args[1];
    const customAliases = botConfig.commandAliases?.music || {};
    const commandName = resolveMusicCommand(commandInput, customAliases);
    
    if (!commandName) {
        // URL이 직접 입력된 경우 → play로 취급
        if (isYouTubeUrl(commandInput)) {
            if (!await requireVoiceChannel(message)) return true;
            await handlePlay(message, [commandInput, ...args.slice(2)], botConfig);
            return true;
        }
        
        // 검색어로 취급 → play로 전달
        if (!await requireVoiceChannel(message)) return true;
        const searchArgs = args.slice(1);
        await handlePlay(message, searchArgs, botConfig);
        return true;
    }
    
    const cmdDef = MUSIC_COMMANDS[commandName];
    const extraArgs = args.slice(2);
    
    // 음성 채널 필수인 명령어 체크
    if (cmdDef.needsVoice && !await requireVoiceChannel(message)) return true;
    
    // play/search는 내부에서 직접 삭제 처리, 그 외 명령어는 여기서 삭제
    if (commandName !== 'play' && commandName !== 'search') safeDelete(message);
    
    await cmdDef.handler(message, extraArgs, botConfig);
    return true;
}

// ── 에러 분류 ──
/**
 * 음악 재생 관련 에러를 분류하여 사용자 친화적 메시지 키와 상세 정보를 반환
 * @param {Error|null} err
 * @returns {{ type: string, detail: string }}
 */
function _classifyError(err) {
    if (!err) return { type: 'unknown', detail: '' };
    const msg = (err.message || '').toLowerCase();
    const code = err.code || '';

    // 연령 제한
    if (msg.includes('age') && (msg.includes('confirm') || msg.includes('restrict') || msg.includes('gate'))) {
        return { type: 'age_restricted', detail: err.message };
    }
    // 로그인 필요
    if (msg.includes('sign in') || msg.includes('login') || msg.includes('log in')) {
        return { type: 'login_required', detail: err.message };
    }
    // 비공개/삭제/사용 불가
    if (msg.includes('unavailable') || msg.includes('private video') || msg.includes('removed') || msg.includes('deleted')) {
        return { type: 'unavailable', detail: err.message };
    }
    // 지역 제한
    if (msg.includes('country') || msg.includes('geo') || msg.includes('region') || msg.includes('blocked in')) {
        return { type: 'geo_blocked', detail: err.message };
    }
    // 저작권
    if (msg.includes('copyright') || msg.includes('dmca') || msg.includes('taken down')) {
        return { type: 'copyright', detail: err.message };
    }
    // 네트워크
    if (msg.includes('etimedout') || msg.includes('econnrefused') || msg.includes('econnreset') ||
        msg.includes('enotfound') || msg.includes('network') || msg.includes('fetch failed') ||
        code === 'ETIMEDOUT' || code === 'ECONNREFUSED' || code === 'ECONNRESET' || code === 'ENOTFOUND') {
        return { type: 'network', detail: err.message };
    }
    // 도구 미설치 (yt-dlp, ffmpeg)
    if (code === 'ENOENT' || msg.includes('enoent') || msg.includes('not found') && (msg.includes('spawn') || msg.includes('yt-dlp') || msg.includes('ffmpeg'))) {
        return { type: 'tool_missing', detail: err.message };
    }
    // ffmpeg 오류
    if (msg.includes('ffmpeg')) {
        return { type: 'ffmpeg', detail: err.message };
    }

    return { type: 'unknown', detail: err.message };
}

/**
 * 에러 분류 결과를 i18n 키에 매핑하여 사용자에게 표시할 문자열 반환
 * @param {Error} err
 * @param {{ title?: string }} trackInfo - 트랙 정보 (있으면 포함)
 * @returns {string}
 */
function _formatPlayError(err, trackInfo = {}) {
    const { type, detail } = _classifyError(err);
    const title = trackInfo.title || '';

    switch (type) {
        case 'age_restricted':
            return i18n.t('bot:music.age_restricted', { title: title || 'Unknown' });
        case 'login_required':
            return i18n.t('bot:music.login_required', { title: title || 'Unknown' });
        case 'unavailable':
            return i18n.t('bot:music.play_error_detail', { reason: i18n.t('bot:music.error_unavailable') });
        case 'geo_blocked':
            return i18n.t('bot:music.play_error_detail', { reason: i18n.t('bot:music.error_geo_blocked') });
        case 'copyright':
            return i18n.t('bot:music.play_error_detail', { reason: i18n.t('bot:music.error_copyright') });
        case 'network':
            return i18n.t('bot:music.play_error_detail', { reason: i18n.t('bot:music.error_network') });
        case 'tool_missing':
            return i18n.t('bot:music.play_error_detail', { reason: i18n.t('bot:music.error_tool_missing') });
        case 'ffmpeg':
            return i18n.t('bot:music.play_error_detail', { reason: i18n.t('bot:music.error_ffmpeg') });
        default:
            // unknown — 원본 에러 메시지를 포함
            return i18n.t('bot:music.play_error_detail', { reason: detail || i18n.t('bot:music.play_error') });
    }
}

/**
 * 스트림/플레이어 오류 발생 시 사용자에게 알림 전송
 * channelUI 채널이 있으면 거기로, 없으면 로그만 남김
 * @param {string} guildId
 * @param {string} errorMsg - 이미 포맷된 에러 메시지
 */
function _notifyStreamError(guildId, errorMsg) {
    const ch = channelUI.getChannel(guildId);
    if (ch) {
        ch.send(errorMsg).catch(() => {});
    }
}

// ── URL 검증 ──
function isYouTubeUrl(str) {
    return /^(https?:\/\/)?(www\.)?(youtube\.com|youtu\.be|music\.youtube\.com)\/.+/.test(str);
}

function isPlaylistUrl(str) {
    // 순수 재생리스트 URL만 (youtube.com/playlist?list=...)
    // watch?v=xxx&list=yyy 같은 개별 영상+재생리스트 조합은 단일 영상으로 취급
    return /youtube\.com\/playlist\?/.test(str) && /[?&]list=/.test(str);
}

// ── 트랙 정보 추출 (play-dl 우선, yt-dlp fallback) ──
async function extractTrackInfo(query, requester, maxResults = 5) {
    let lastError = null; // 최종 실패 시 상세 원인 보존용

    // URL인 경우
    if (isYouTubeUrl(query)) {
        // play-dl로 플레이리스트 시도
        if (isPlaylistUrl(query) && playDl) {
            try {
                const playlist = await playDl.playlist_info(query, { incomplete: true });
                const videos = await playlist.all_videos();
                return videos.map(v => ({
                    title: v.title || 'Unknown',
                    url: v.url,
                    duration: v.durationRaw || '??:??',
                    requester,
                }));
            } catch (e) {
                lastError = e;
                console.warn('[Music] Playlist fetch failed, trying as single video:', e.message);
            }
        }
        
        // 단일 영상: play-dl 시도 → yt-dlp fallback
        if (playDl) {
            try {
                const info = await playDl.video_info(query);
                return [{
                    title: info.video_details.title || 'Unknown',
                    url: info.video_details.url,
                    duration: info.video_details.durationRaw || '??:??',
                    requester,
                }];
            } catch (e) {
                lastError = e;
                console.warn('[Music] play-dl video_info failed, trying yt-dlp:', e.message);
            }
        }

        // yt-dlp fallback
        const info = await getTrackInfoViaYtDlp(query);
        if (info) return [{ ...info, requester }];

        // 모든 방법 실패 — lastError가 있으면 원본 에러 그대로 throw
        throw lastError || new Error(i18n.t('bot:music.invalid_url'));
    }
    
    // 검색: play-dl.search() 우선 (API 호출이라 훨씬 빠름), yt-dlp fallback
    if (playDl) {
        try {
            const results = await playDl.search(query, { source: { youtube: 'video' }, limit: maxResults });
            if (results && results.length > 0) {
                return results.map(v => ({
                    title: v.title || 'Unknown',
                    url: v.url,
                    duration: v.durationRaw || '??:??',
                    requester,
                }));
            }
        } catch (e) {
            lastError = e;
            console.warn('[Music] play-dl search failed, trying yt-dlp:', e.message);
        }
    }

    // yt-dlp fallback (느림)
    const info = await getTrackInfoViaYtDlp(`ytsearch${maxResults}:${query}`);
    if (info) {
        const tracks = Array.isArray(info) ? info : [info];
        const results = tracks.filter(t => t.url).map(t => ({
            title: t.title,
            url: t.url,
            duration: t.duration,
            requester,
        }));
        if (results.length > 0) return results;
    }

    throw lastError || new Error(i18n.t('bot:music.no_results'));
}

/**
 * yt-dlp로 트랙 메타데이터 추출 (JSON)
 * ytsearchN: 쿼리의 경우 여러 개의 JSON 객체가 줄바꿈으로 구분되어 반환됨
 * 
 * spawn 기반 비동기 — execSync의 maxBuffer(ENOBUFS) 문제 해결
 */
function getTrackInfoViaYtDlp(query) {
    return new Promise((resolve) => {
        const proc = spawn(ytDlpPath, [
            '--no-playlist', '--no-warnings', '-j', query,
        ], { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });

        let stdout = '';
        proc.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
        proc.stderr.on('data', () => {}); // 무시

        const timer = setTimeout(() => {
            proc.kill();
            console.warn('[Music] yt-dlp info timed out (20s)');
            resolve(null);
        }, 20_000);

        proc.on('error', (err) => {
            clearTimeout(timer);
            console.warn('[Music] yt-dlp info spawn error:', err.message);
            resolve(null);
        });

        proc.on('close', () => {
            clearTimeout(timer);
            try {
                const result = stdout.trim();
                if (!result) { resolve(null); return; }

                // ytsearchN:의 경우 여러 줄의 JSON
                const lines = result.split('\n').filter(l => l.trim());
                if (lines.length > 1) {
                    const tracks = [];
                    for (const line of lines) {
                        try {
                            const data = JSON.parse(line);
                            // H5: URL 없으면 검색어가 URL로 저장되는 것 방지
                            const trackUrl = data.webpage_url || data.url;
                            if (!trackUrl) {
                                console.warn(`[Music] yt-dlp result missing URL, skipping: ${data.title || 'unknown'}`);
                                continue;
                            }
                            const duration = data.duration
                                ? `${Math.floor(data.duration / 60)}:${String(Math.floor(data.duration) % 60).padStart(2, '0')}`
                                : '??:??';
                            tracks.push({
                                title: data.title || data.fulltitle || 'Unknown',
                                url: trackUrl,
                                duration,
                                channel: data.channel || data.uploader || '',
                                channelId: data.channel_id || '',
                                artist: data.artist || '',
                                viewCount: data.view_count || 0,
                            });
                        } catch (_) {}
                    }
                    resolve(tracks.length > 0 ? tracks : null);
                    return;
                }

                const data = JSON.parse(result);
                // H5: URL 없으면 검색어가 URL로 저장되는 것 방지
                const trackUrl = data.webpage_url || data.url;
                if (!trackUrl) {
                    console.warn(`[Music] yt-dlp single result missing URL: ${data.title || 'unknown'}`);
                    resolve(null);
                    return;
                }
                const duration = data.duration
                    ? `${Math.floor(data.duration / 60)}:${String(Math.floor(data.duration) % 60).padStart(2, '0')}`
                    : '??:??';
                resolve({
                    title: data.title || data.fulltitle || 'Unknown',
                    url: trackUrl,
                    duration,
                    channel: data.channel || data.uploader || '',
                    channelId: data.channel_id || '',
                    artist: data.artist || '',
                    viewCount: data.view_count || 0,
                });
            } catch (e) {
                console.warn('[Music] yt-dlp info parse failed:', e.message);
                resolve(null);
            }
        });
    });
}

// ── yt-dlp + ffmpeg 스트리밍 ──
/**
 * yt-dlp로 유튜브 오디오를 추출하고 ffmpeg로 Ogg/Opus로 변환해 스트림으로 반환
 *
 * normalize=true (투패스):
 *   yt-dlp → 메모리 수집 → ffmpeg 분석(1패스) → ffmpeg 인코딩(2패스, 고정 게인) → Discord
 *   트랙 내부는 원본 다이내믹 그대로, 트랙 간 볼륨만 -14 LUFS로 통일
 *
 * normalize=false (다이렉트 파이프):
 *   yt-dlp stdout → ffmpeg stdin → ffmpeg stdout (Ogg/Opus) → Discord
 */
function createYtDlpStream(url, { normalize = true } = {}) {
    // 대용량 중간 버퍼: YouTube throttling에 의한 끊김 방지
    const AUDIO_BUFFER_SIZE = 8 * 1024 * 1024;
    const buffer = new PassThrough({ highWaterMark: AUDIO_BUFFER_SIZE });

    // 활성 프로세스 추적 (외부 cleanup용)
    buffer._ytdlp = null;
    buffer._ffmpeg = null;
    buffer._streamTimer = null;

    // 전체 타임아웃 — 30분
    const STREAM_TIMEOUT = 30 * 60 * 1000;
    buffer._streamTimer = setTimeout(() => {
        console.warn('[Music] Stream timeout (30min), killing processes');
        _killStreamProcs(buffer);
        buffer.destroy();
    }, STREAM_TIMEOUT);

    // cleanup: 버퍼 스트림이 닫히면 프로세스도 정리
    const cleanup = () => {
        if (buffer._streamTimer) clearTimeout(buffer._streamTimer);
        _killStreamProcs(buffer);
    };
    buffer.on('close', cleanup);
    buffer.on('error', cleanup);

    if (normalize) {
        _startTwoPassStream(url, buffer);
    } else {
        _startDirectStream(url, buffer);
    }

    return buffer;
}

/** 버퍼에 연결된 자식 프로세스 정리 */
function _killStreamProcs(buffer) {
    try { buffer._ytdlp?.kill(); } catch (_) {}
    try { buffer._ffmpeg?.kill(); } catch (_) {}
    buffer._ytdlp = null;
    buffer._ffmpeg = null;
}

/**
 * 다이렉트 파이프 (normalize=false)
 * yt-dlp stdout → ffmpeg stdin → buffer
 */
function _startDirectStream(url, buffer) {
    const ytdlpArgs = [
        '-f', 'worstaudio', '--no-playlist', '-o', '-',
        '--quiet', '--no-warnings',
        '--buffer-size', '64K', '--concurrent-fragments', '4',
        url,
    ];
    const ytdlp = spawn(ytDlpPath, ytdlpArgs,
        { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    buffer._ytdlp = ytdlp;

    ytdlp.stderr.on('data', (data) => {
        const msg = data.toString().trim();
        if (msg) console.warn(`[Music] yt-dlp stderr: ${msg}`);
    });

    const ffmpegArgs = [
        '-hide_banner', '-loglevel', 'error',
        '-i', 'pipe:0', '-vn',
        '-acodec', 'libopus', '-b:a', '64k',
        '-f', 'ogg', '-ar', '48000', '-ac', '2',
        'pipe:1',
    ];
    const ffmpeg = spawn(ffmpegPath, ffmpegArgs,
        { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
    buffer._ffmpeg = ffmpeg;

    ffmpeg.stderr.on('data', (data) => {
        const msg = data.toString().trim();
        if (msg) console.warn(`[Music] ffmpeg stderr: ${msg}`);
    });

    ytdlp.stdout.on('error', (err) => {
        if (err.code !== 'EPIPE') console.warn('[Music] yt-dlp stdout error:', err.message);
    });
    ffmpeg.stdin.on('error', (err) => {
        if (err.code !== 'EPIPE') console.warn('[Music] ffmpeg stdin error:', err.message);
    });
    ytdlp.stdout.pipe(ffmpeg.stdin);

    ytdlp.on('error', (err) => {
        console.error('[Music] yt-dlp spawn error:', err.message);
        try { ffmpeg.kill(); } catch (_) {}
    });
    ffmpeg.on('error', (err) => {
        console.error('[Music] ffmpeg spawn error:', err.message);
        try { ytdlp.kill(); } catch (_) {}
    });
    ytdlp.on('close', (code) => {
        if (code !== 0) console.warn(`[Music] yt-dlp exited with code ${code}`);
        ffmpeg.stdin.end();
    });

    ffmpeg.stdout.pipe(buffer);
}

// 투패스 분석 없이 인코딩만 (폴백용)
const FFMPEG_ENCODE_ARGS = [
    '-hide_banner', '-loglevel', 'error',
    '-i', 'pipe:0', '-vn',
    '-acodec', 'libopus', '-b:a', '64k',
    '-f', 'ogg', '-ar', '48000', '-ac', '2',
];

/**
 * 투패스 정규화 (normalize=true)
 *   1) yt-dlp → 메모리 수집 (worstaudio: 보통 1~10MB)
 *   2) ffmpeg 1패스: loudnorm 분석 → measured_I, measured_TP 등
 *   3) ffmpeg 2패스: measured 값 + linear=true → 고정 게인으로 인코딩
 */
function _startTwoPassStream(url, buffer) {
    // 너무 큰 파일은 메모리 안전을 위해 싱글패스 폴백 (50MB)
    const MAX_RAW_SIZE = 50 * 1024 * 1024;

    const ytdlpArgs = [
        '-f', 'worstaudio', '--no-playlist', '-o', '-',
        '--quiet', '--no-warnings',
        '--buffer-size', '64K', '--concurrent-fragments', '4',
        url,
    ];
    const ytdlp = spawn(ytDlpPath, ytdlpArgs,
        { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
    buffer._ytdlp = ytdlp;

    ytdlp.stderr.on('data', (data) => {
        const msg = data.toString().trim();
        if (msg) console.warn(`[Music] yt-dlp stderr: ${msg}`);
    });

    // 메모리에 원본 오디오 수집
    const chunks = [];
    let totalSize = 0;
    let oversized = false;
    ytdlp.stdout.on('data', (chunk) => {
        totalSize += chunk.length;
        if (totalSize > MAX_RAW_SIZE) {
            oversized = true;
            return; // 초과분은 버림
        }
        chunks.push(chunk);
    });

    ytdlp.on('error', (err) => {
        console.error('[Music] yt-dlp spawn error:', err.message);
        buffer.destroy();
    });

    ytdlp.on('close', (code) => {
        buffer._ytdlp = null;
        if (code !== 0 || chunks.length === 0) {
            if (code !== 0) console.warn(`[Music] yt-dlp exited with code ${code}`);
            buffer.destroy();
            return;
        }

        const rawAudio = Buffer.concat(chunks);

        if (oversized) {
            console.warn(`[Music] Raw audio too large (${totalSize} bytes), falling back to single-pass`);
            _encodePipe(rawAudio, ['-af', LOUDNORM_FILTER], buffer);
            return;
        }

        console.log(`[Music] Raw audio collected: ${rawAudio.length} bytes, starting two-pass analysis`);

        // 1패스: 라우드니스 분석
        _analyzeLoudness(rawAudio).then((measured) => {
            // 2패스: 고정 게인 인코딩
            const filter = `loudnorm=I=-14:TP=-1:LRA=11:` +
                `measured_I=${measured.input_i}:` +
                `measured_TP=${measured.input_tp}:` +
                `measured_LRA=${measured.input_lra}:` +
                `measured_thresh=${measured.input_thresh}:` +
                `linear=true`;
            console.log(`[Music] Loudness: ${measured.input_i} LUFS → applying fixed gain (linear=true)`);
            _encodePipe(rawAudio, ['-af', filter], buffer);
        }).catch((err) => {
            console.warn('[Music] Loudness analysis failed, encoding without normalization:', err.message);
            _encodePipe(rawAudio, [], buffer);
        });
    });
}

/**
 * ffmpeg 1패스: loudnorm 분석 → 측정값 반환
 * @param {Buffer} rawAudio - 원본 오디오 데이터
 * @returns {Promise<{input_i, input_tp, input_lra, input_thresh}>}
 */
function _analyzeLoudness(rawAudio) {
    return new Promise((resolve, reject) => {
        const args = [
            '-hide_banner',
            '-i', 'pipe:0',
            '-af', 'loudnorm=I=-14:TP=-1:LRA=11:print_format=json',
            '-f', 'null', '-',
        ];
        const proc = spawn(ffmpegPath, args,
            { stdio: ['pipe', 'ignore', 'pipe'], windowsHide: true });

        let stderrData = '';
        proc.stderr.on('data', (chunk) => { stderrData += chunk.toString(); });
        proc.stdin.on('error', (err) => {
            if (err.code !== 'EPIPE') console.warn('[Music] analyze stdin error:', err.message);
        });

        proc.on('close', () => {
            // loudnorm은 stderr 마지막에 JSON 블록을 출력
            const jsonMatch = stderrData.match(/\{[\s\S]*"input_i"[\s\S]*?\}/);
            if (!jsonMatch) {
                reject(new Error('Failed to parse loudnorm analysis output'));
                return;
            }
            try {
                const data = JSON.parse(jsonMatch[0]);
                resolve({
                    input_i: data.input_i,
                    input_tp: data.input_tp,
                    input_lra: data.input_lra,
                    input_thresh: data.input_thresh,
                });
            } catch (e) {
                reject(new Error(`loudnorm JSON parse error: ${e.message}`));
            }
        });

        proc.on('error', (err) => reject(err));

        // 원본 데이터를 stdin에 주입
        proc.stdin.end(rawAudio);
    });
}

/**
 * ffmpeg 인코딩 — 원본 데이터를 stdin에 주입하고 Ogg/Opus를 buffer에 출력
 * @param {Buffer} rawAudio - 원본 오디오 데이터
 * @param {string[]} extraArgs - 추가 ffmpeg 인자 (예: ['-af', 'loudnorm=...'])
 * @param {PassThrough} buffer - 출력 스트림
 */
function _encodePipe(rawAudio, extraArgs, buffer) {
    const args = [...FFMPEG_ENCODE_ARGS, ...extraArgs, 'pipe:1'];
    const ffmpeg = spawn(ffmpegPath, args,
        { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
    buffer._ffmpeg = ffmpeg;

    ffmpeg.stderr.on('data', (data) => {
        const msg = data.toString().trim();
        if (msg) console.warn(`[Music] ffmpeg encode stderr: ${msg}`);
    });
    ffmpeg.stdin.on('error', (err) => {
        if (err.code !== 'EPIPE') console.warn('[Music] ffmpeg encode stdin error:', err.message);
    });
    ffmpeg.on('error', (err) => {
        console.error('[Music] ffmpeg encode error:', err.message);
        buffer.destroy();
    });

    ffmpeg.stdout.pipe(buffer);
    ffmpeg.stdin.end(rawAudio);
}

// ── 대기열 전체 프리페치 ──

/**
 * 대기열의 모든 곡을 미리 다운로드+버퍼링 시작
 * - 이미 프리페치 중인 URL은 스킵
 * - 큐에서 빠진 URL의 프리페치는 정리
 */
function startPrefetchAll(guildId) {
    const queue = getQueue(guildId);
    if (!queue) return;
    if (queue.tracks.length === 0) return;

    const queueUrls = new Set(queue.tracks.map(t => t.url));

    // 큐에서 빠진 URL의 프리페치 정리
    for (const [url, stream] of queue.prefetchMap) {
        if (!queueUrls.has(url)) {
            _cleanupOneStream(stream);
            queue.prefetchMap.delete(url);
        }
    }

    // 아직 프리페치되지 않은 트랙 시작
    for (const track of queue.tracks) {
        if (queue.prefetchMap.has(track.url)) continue;
        console.log(`[Music] Prefetching: ${track.title}`);
        const stream = createYtDlpStream(track.url, { normalize: queue.normalize });
        queue.prefetchMap.set(track.url, stream);
    }
}

/**
 * 단일 스트림 정리 헬퍼
 */
function _cleanupOneStream(stream) {
    try {
        if (stream._streamTimer) clearTimeout(stream._streamTimer);
        if (stream._ytdlp) { try { stream._ytdlp.kill('SIGTERM'); } catch (_) {} }
        if (stream._ffmpeg) { try { stream._ffmpeg.kill('SIGTERM'); } catch (_) {} }
        stream.destroy();
    } catch (_) {}
}

/**
 * 프리페치 전체 정리
 */
function cleanupPrefetch(queue) {
    if (queue.prefetchMap && queue.prefetchMap.size > 0) {
        for (const [, stream] of queue.prefetchMap) {
            _cleanupOneStream(stream);
        }
        queue.prefetchMap.clear();
    }
}

// ── 재생 엔진 ──
async function playNext(guildId) {
    const release = await acquireLock(guildId);
    try {
        await _playNextInner(guildId);
    } finally {
        release();
    }
}

async function _playNextInner(guildId) {
    const queue = getQueue(guildId);
    if (!queue || !queue.connection) return;
    
    if (queue.tracks.length === 0) {
        // 라디오 모드 활성 시 비동기로 큐 보충 시도
        if (queue.radio) {
            const added = await replenishRadioQueue(guildId);
            if (added === 0) {
                console.log('[Music/Radio] No more recommendations, radio ending');
                queue.radio = false;
                queue.current = null;
                startIdleTimer(guildId);
                return;
            }
            // 보충 성공 → fall-through하여 계속 재생
        }

        if (queue.tracks.length === 0) {
            queue.current = null;
            // 대기열 비었음 → 일정 시간 후 자동 퇴장
            startIdleTimer(guildId);
            return;
        }
    }
    
    clearIdleTimer(guildId);
    
    const track = queue.tracks.shift();
    queue.current = track;

    // 라디오 모드: 곡을 꺼낸 뒤 남은 라디오 곡이 적으면 백그라운드로 보충
    if (queue.radio) {
        replenishRadioQueue(guildId).catch(e => {
            console.warn('[Music/Radio] Background replenish error:', e.message);
        });
    }
    
    try {
        // 연결이 Ready 상태가 될 때까지 대기 (최대 15초)
        if (queue.connection.state.status !== voice.VoiceConnectionStatus.Ready) {
            console.log(`[Music] Waiting for voice connection ready (current: ${queue.connection.state.status})...`);
            try {
                await voice.entersState(queue.connection, voice.VoiceConnectionStatus.Ready, 15_000);
            } catch (e) {
                console.warn('[Music] Connection not ready, attempting rejoin...');
                const rejoined = await _rejoinVoice(guildId);
                if (!rejoined) {
                    console.error('[Music] Rejoin failed, cannot play');
                    queue.current = null;
                    return;
                }
            }
        }

        console.log(`[Music] Streaming: ${track.title} (${track.url})`);
        
        // 프리페치된 스트림이 있으면 재활용 (destroyed 체크 포함)
        let audioStream;
        const prefetched = queue.prefetchMap.get(track.url);
        if (prefetched && !prefetched.destroyed) {
            audioStream = prefetched;
            queue.prefetchMap.delete(track.url); // 소유권 이전
            console.log(`[Music] Using prefetched stream (${audioStream.readableLength} bytes already buffered)`);
        } else {
            if (prefetched) queue.prefetchMap.delete(track.url); // 만료된 엔트리 정리
            audioStream = createYtDlpStream(track.url, { normalize: queue.normalize });
            console.log(`[Music] yt-dlp+ffmpeg stream created, pre-buffering...`);
        }
        
        // 프리버퍼링: 재생 전 최소 데이터 축적 대기 (끊김 방지)
        // 32KB — 8MB PassThrough 버퍼가 재생 중 계속 채우므로 초기에 많이 기다릴 필요 없음
        const PRE_BUFFER_BYTES = 32 * 1024;  // 32KB
        const PRE_BUFFER_MIN = 8 * 1024;     // 최소 8KB — 이하면 경고
        const PRE_BUFFER_TIMEOUT = 2000;      // 최대 2초 대기
        await new Promise((resolve) => {
            let resolved = false;
            let timer = null;
            const done = () => {
                if (resolved) return;
                resolved = true;
                audioStream.removeListener('readable', checkReadable);
                audioStream.removeListener('end', onEnd);
                if (timer) clearTimeout(timer);
                resolve();
            };
            // readable 이벤트로 데이터 소비 없이 버퍼 채움 감시
            const checkReadable = () => {
                if (audioStream.readableLength >= PRE_BUFFER_BYTES) {
                    done();
                }
            };
            audioStream.on('readable', checkReadable);
            // 스트림 종료 시에도 resolve (짧은 오디오)
            const onEnd = () => done();
            audioStream.once('end', onEnd);
            // 이미 충분히 쌓여 있으면 바로 진행
            if (audioStream.readableLength >= PRE_BUFFER_BYTES) {
                done();
                return;
            }
            // 타임아웃: 느린 네트워크에서도 최대 대기 후 재생 시작
            timer = setTimeout(() => {
                // H6: 프리버퍼 부족 경고
                if (audioStream.readableLength < PRE_BUFFER_MIN) {
                    console.warn(`[Music] Pre-buffer critically low: ${audioStream.readableLength} bytes < ${PRE_BUFFER_MIN} — audio may stutter`);
                } else {
                    console.log(`[Music] Pre-buffer timeout, starting with ${audioStream.readableLength} bytes`);
                }
                done();
            }, PRE_BUFFER_TIMEOUT);
        });
        console.log(`[Music] Pre-buffer done (${audioStream.readableLength} bytes in buffer)`);
        
        const resource = voice.createAudioResource(audioStream, {
            inputType: voice.StreamType.OggOpus,
            inlineVolume: true,
        });
        resource.volume?.setVolume(queue.volume);
        queue.resource = resource;
        
        if (!queue.player) {
            queue.player = voice.createAudioPlayer({
                behaviors: { noSubscriber: voice.NoSubscriberBehavior.Pause },
            });
            
            queue.player.on('stateChange', (oldState, newState) => {
                console.log(`[Music] Player: ${oldState.status} → ${newState.status}`);

                // autopaused 반복 감지 — UDP 소켓 죽음 감지 후 rejoin
                if (newState.status === voice.AudioPlayerStatus.AutoPaused) {
                    queue._autoPauseCount = (queue._autoPauseCount || 0) + 1;
                    if (queue._autoPauseResetTimer) clearTimeout(queue._autoPauseResetTimer);
                    queue._autoPauseResetTimer = setTimeout(() => { queue._autoPauseCount = 0; }, 15_000);

                    if (queue._autoPauseCount >= 3 && !queue._rejoining) {
                        console.warn(`[Music] AutoPaused ${queue._autoPauseCount} times in 15s — voice connection likely dead, rejoining`);
                        queue._autoPauseCount = 0;
                        _rejoinVoice(guildId).then((ok) => {
                            if (!ok) {
                                console.error('[Music] Auto-rejoin failed, destroying queue');
                                destroyQueue(guildId);
                            }
                        }).catch(() => {});
                    }
                } else if (newState.status === voice.AudioPlayerStatus.Playing) {
                    // 정상 재생 시작 시 카운터 리셋
                    if (oldState.status !== voice.AudioPlayerStatus.AutoPaused) {
                        queue._autoPauseCount = 0;
                    }
                }
            });
            
            queue.player.on(voice.AudioPlayerStatus.Idle, () => {
                // 이전 곡 히스토리에 보존 (최대 5곡)
                if (queue.current) {
                    // 라디오 시드풀: 5초 이내 스킵 → 시드에서 제거 (취향이 아닌 곡)
                    if (queue.radio && queue._radioSeedPool.length > 0 && queue._startedAt) {
                        const playedMs = Date.now() - queue._startedAt;
                        if (playedMs < RADIO_SKIP_THRESHOLD_MS) {
                            const skipIdx = queue._radioSeedPool.findIndex(t => t.url === queue.current.url);
                            if (skipIdx !== -1) {
                                console.log(`[Music/Radio] Skipped in ${Math.round(playedMs / 1000)}s, removing from seed pool: ${queue.current.title}`);
                                queue._radioSeedPool.splice(skipIdx, 1);
                            }
                        }
                    }

                    queue.history.unshift(queue.current);
                    if (queue.history.length > 5) queue.history.pop();
                    // 라디오 전용 히스토리 (최대 30곡, 중복 방지 범위 확장)
                    queue._radioHistory.unshift(queue.current);
                    if (queue._radioHistory.length > 30) queue._radioHistory.pop();
                }
                // 진행바 잔류 방지 — 즉시 리셋
                queue._startedAt = null;
                queue._paused = false;
                queue._pausedElapsed = 0;

                if (queue.loop && queue.current) {
                    queue.tracks.unshift(queue.current);
                }
                playNext(guildId).catch(err => {
                    console.error('[Music] playNext error (from Idle handler):', err.message);
                });
            });
            
            queue.player.on('error', (err) => {
                console.error('[Music] Player error:', err.message);
                const trackTitle = queue.current?.title || 'Unknown';
                const { detail } = _classifyError(err);
                const errorMsg = i18n.t('bot:music.stream_error', { title: trackTitle, reason: detail || err.message });
                _notifyStreamError(guildId, errorMsg);
                playNext(guildId).catch(err2 => {
                    console.error('[Music] playNext error (from error handler):', err2.message);
                });
            });
            
            queue.connection.subscribe(queue.player);
            console.log('[Music] Player created and subscribed to connection');
        }
        
        queue.player.play(resource);
        queue._startedAt = Date.now();
        queue._paused = false;
        queue._pausedElapsed = 0;
        queue._activeStream = audioStream; // H3: 스트림 참조 보존 (cleanup용)
        console.log(`[Music] play() called — player status: ${queue.player.state.status}`);
        
        // 대기열 전체 프리페치 시작
        startPrefetchAll(guildId);

        // 전용 채널 UI 갱신 (곡 변경)
        channelUI.refreshQueue(guildId).catch(() => {});
    } catch (e) {
        console.error('[Music] Stream error:', e.message);
        // 사용자에게 에러 알림 시도 — channelUI 채널 또는 큐 정보로 전달
        const queue = getQueue(guildId);
        const trackTitle = queue?.current?.title || 'Unknown';
        const { detail } = _classifyError(e);
        const errorMsg = i18n.t('bot:music.stream_error', { title: trackTitle, reason: detail || e.message });
        _notifyStreamError(guildId, errorMsg);
        // 스트림 실패 → 다음 곡으로
        playNext(guildId).catch(err2 => {
            console.error('[Music] playNext error after stream failure:', err2.message);
        });
    }
}

function startIdleTimer(guildId) {
    const queue = getQueue(guildId);
    if (!queue) return;
    clearIdleTimer(guildId);
    queue.idleTimer = setTimeout(() => {
        destroyQueue(guildId);
    }, IDLE_TIMEOUT);
}

function clearIdleTimer(guildId) {
    const queue = getQueue(guildId);
    if (queue?.idleTimer) {
        clearTimeout(queue.idleTimer);
        queue.idleTimer = null;
    }
}

/**
 * 봇이 이미 다른 음성 채널에 있는지 체크.
 * 다른 채널이면 경고 메시지를 보내고 true 반환, 아니면 false.
 */
async function checkBotInOtherChannel(message, voiceChannel) {
    if (!message.guild) return false;
    const queue = getQueue(message.guild.id);
    if (!queue || !queue.connection) return false;
    if (queue.connection.state.status === voice.VoiceConnectionStatus.Destroyed) return false;
    if (queue.connection.joinConfig.channelId === voiceChannel.id) return false;
    // 봇이 다른 채널에서 사용 중
    const botChannel = message.guild.channels.cache.get(queue.connection.joinConfig.channelId);
    const channelName = botChannel ? botChannel.name : queue.connection.joinConfig.channelId;
    await message.channel.send(i18n.t('bot:music.bot_in_other_channel', { channel: channelName })).catch(() => {});
    return true;
}

// 안전한 메시지 삭제
function safeDelete(msg) {
    if (msg && msg.deletable) {
        msg.delete().catch(() => {});
    }
}

// ── Command Handlers ──

async function handlePlay(message, args, botConfig) {
    if (args.length === 0) {
        await message.reply(i18n.t('bot:music.play_usage', {
            prefix: botConfig.prefix
        }));
        return;
    }
    
    // 음성 채널 체크 (호출자 측에서 이미 체크했더라도 방어적으로)
    const voiceChannel = await requireVoiceChannel(message);
    if (!voiceChannel) return;
    
    // ── 봇이 이미 다른 음성 채널에 있으면 검색 전에 차단 ──
    if (await checkBotInOtherChannel(message, voiceChannel)) return;
    
    // 봇 권한 체크
    const permissions = voiceChannel.permissionsFor(message.client.user);
    if (!permissions?.has('Connect') || !permissions?.has('Speak')) {
        await message.reply(i18n.t('bot:music.no_permission'));
        return;
    }
    
    const query = args.join(' ');
    const isUrl = isYouTubeUrl(query);
    
    // 원본 명령어 메시지 삭제
    safeDelete(message);
    
    const statusMsg = await message.channel.send(i18n.t('bot:music.searching', {
        query: query.length > 60 ? query.substring(0, 57) + '...' : query
    }));
    
    try {
        const requesterName = message.member?.displayName || message.author.displayName || message.author.username;
        // URL이면 전체 메타데이터, 검색어면 1개만 가져오기 (속도 최적화)
        const searchCount = isUrl ? 5 : 1;
        const candidates = await extractTrackInfo(query, requesterName, searchCount);
        
        // URL이거나 플레이리스트면 전체 재생
        if (isUrl || isPlaylistUrl(query)) {
            await enqueueAndPlay(message, statusMsg, candidates, voiceChannel);
            return;
        }
        
        // 검색 결과 → 첫 번째 결과로 바로 재생
        if (candidates.length > 0) {
            await enqueueAndPlay(message, statusMsg, [candidates[0]], voiceChannel);
            return;
        }
        
        await statusMsg.edit(`❌ ${i18n.t('bot:music.no_results')}`).catch(() => {});
    } catch (e) {
        console.error('[Music] Play error:', e.message);
        const errorMsg = _formatPlayError(e);
        await statusMsg.edit(errorMsg).catch(() => {});
    }
}

/**
 * 검색 — 상위 5개 결과를 버튼으로 보여주고 요청자만 선택 가능
 */
async function handleSearch(message, args, botConfig) {
    if (args.length === 0) {
        await message.channel.send(i18n.t('bot:music.search_usage', {
            prefix: botConfig.prefix
        }));
        return;
    }
    
    const voiceChannel = await requireVoiceChannel(message);
    if (!voiceChannel) return;
    
    // ── 봇이 이미 다른 음성 채널에 있으면 검색 전에 차단 ──
    if (await checkBotInOtherChannel(message, voiceChannel)) return;
    
    const query = args.join(' ');
    
    safeDelete(message);
    
    const statusMsg = await message.channel.send(i18n.t('bot:music.searching', {
        query: query.length > 60 ? query.substring(0, 57) + '...' : query
    }));
    
    try {
        // URL이면 바로 재생 (검색 UI 불필요)
        if (isYouTubeUrl(query)) {
            const requesterName = message.member?.displayName || message.author.displayName || message.author.username;
            const tracks = await extractTrackInfo(query, requesterName);
            await enqueueAndPlay(message, statusMsg, tracks, voiceChannel);
            return;
        }
        
        const requesterName = message.member?.displayName || message.author.displayName || message.author.username;
        const candidates = await extractTrackInfo(query, requesterName);
        if (candidates.length === 0) {
            await statusMsg.edit(`❌ ${i18n.t('bot:music.no_results')}`);
            return;
        }
        
        const display = candidates.slice(0, 5);
        
        // 검색 결과 텍스트
        let text = i18n.t('bot:music.search_results', {
            query
        }) + '\n';
        display.forEach((t, idx) => {
            text += `\n\`${idx + 1}.\` **${t.title}** [${t.duration}]`;
        });
        
        // 버튼 생성 (1~5 + 취소) — ActionRow는 최대 5개 버튼이므로 2행으로 분리
        const numberButtons = display.map((t, idx) =>
            new ButtonBuilder()
                .setCustomId(`music_search_${idx}`)
                .setLabel(`${idx + 1}`)
                .setStyle(ButtonStyle.Primary)
        );
        const cancelButton = new ButtonBuilder()
            .setCustomId('music_search_cancel')
            .setLabel('✖')
            .setStyle(ButtonStyle.Secondary);
        const row1 = new ActionRowBuilder().addComponents(numberButtons);
        const row2 = new ActionRowBuilder().addComponents(cancelButton);
        
        await statusMsg.edit({ content: text, components: [row1, row2] });
        
        // 요청자만 클릭 가능한 콜렉터 (30초)
        const collector = statusMsg.createMessageComponentCollector({
            filter: (i) => i.user.id === message.author.id,
            time: 30_000,
            max: 1,
        });
        
        collector.on('collect', async (interaction) => {
            if (interaction.customId === 'music_search_cancel') {
                await interaction.deferUpdate();
                safeDelete(statusMsg);
                return;
            }
            
            const idx = parseInt(interaction.customId.replace('music_search_', ''), 10);
            if (isNaN(idx) || idx < 0 || idx >= display.length) {
                safeDelete(statusMsg);
                return;
            }
            
            // 버튼 제거 + 선택 반영
            await interaction.deferUpdate();
            await enqueueAndPlay(message, statusMsg, [display[idx]], voiceChannel);
        });
        
        collector.on('end', (collected) => {
            if (collected.size === 0) {
                // 타임아웃 → 삭제
                safeDelete(statusMsg);
            }
        });
        
    } catch (e) {
        console.error('[Music] Search error:', e.message);
        const errorMsg = _formatPlayError(e);
        await statusMsg.edit(errorMsg).catch(() => {});
    }
}

/**
 * 대기열에 추가하고 재생 시작 (공통 로직)
 */
async function enqueueAndPlay(message, statusMsg, tracks, voiceChannel) {
    if (!message.guild) {
        await message.reply(i18n.t('bot:music.not_available')).catch(() => {});
        return;
    }
    const queue = getOrCreateQueue(message.guild.id);
    
    // ── 봇이 이미 다른 음성 채널에 있는지 체크 ──
    if (queue.connection
        && queue.connection.state.status !== voice.VoiceConnectionStatus.Destroyed
        && queue.connection.joinConfig.channelId !== voiceChannel.id) {
        const botChannel = message.guild.channels.cache.get(queue.connection.joinConfig.channelId);
        const channelName = botChannel ? botChannel.name : queue.connection.joinConfig.channelId;
        await statusMsg.edit(i18n.t('bot:music.bot_in_other_channel', { channel: channelName })).catch(() => {});
        return;
    }

    // 음성 채널 연결 (미연결 시)
    if (!queue.connection || queue.connection.state.status === voice.VoiceConnectionStatus.Destroyed) {
        queue.connection = voice.joinVoiceChannel({
            channelId: voiceChannel.id,
            guildId: message.guild.id,
            adapterCreator: message.guild.voiceAdapterCreator,
            selfDeaf: true,
        });
        queue._adapterCreator = message.guild.voiceAdapterCreator;

        _registerDisconnectHandler(queue, message.guild.id);
    }
    
    // 수동 트랙은 라디오 자동 트랙 앞에 삽입 (라디오 곡보다 먼저 재생)
    if (queue.radio && queue.tracks.some(isRadioTrack)) {
        const insertIdx = findManualInsertIndex(queue.tracks);
        queue.tracks.splice(insertIdx, 0, ...tracks);
    } else {
        queue.tracks.push(...tracks);
    }

    // 새 트랙 추가 후 전체 프리페치 갱신
    startPrefetchAll(message.guild.id);

    // 라디오 시드풀에 수동 추가곡도 반영 (추천 다양성 확장)
    if (queue.radio && queue._radioSeedPool) {
        queue._radioSeedPool.push(...tracks.filter(t => t?.url));
        if (queue._radioSeedPool.length > RADIO_SEED_POOL_MAX) {
            queue._radioSeedPool.splice(0, queue._radioSeedPool.length - RADIO_SEED_POOL_MAX);
        }
    }
    
    const requester = `<@${message.author.id}>`;
    
    if (tracks.length === 1) {
        const track = tracks[0];
        const position = queue.current ? queue.tracks.length : 0;
        
        if (!queue.current && !queue._playNextPending) {
            queue._playNextPending = true;
            await statusMsg.edit({ content: i18n.t('bot:music.now_playing', {
                title: track.title,
                duration: track.duration,
                requester
            }), components: [] });
            playNext(message.guild.id).finally(() => { queue._playNextPending = false; });
        } else {
            await statusMsg.edit({ content: i18n.t('bot:music.added_to_queue', {
                title: track.title,
                duration: track.duration,
                position: position,
                requester
            }), components: [] });
        }
    } else {
        await statusMsg.edit({ content: i18n.t('bot:music.playlist_added', {
            count: tracks.length,
            requester
        }), components: [] });
        if (!queue.current && !queue._playNextPending) {
            queue._playNextPending = true;
            playNext(message.guild.id).finally(() => { queue._playNextPending = false; });
        }
    }

    // 전용 채널 UI 큐 갱신
    channelUI.refreshQueue(message.guild.id).catch(() => {});
}

async function handlePause(message) {
    const queue = getQueue(message.guild.id);
    if (!queue?.player || !queue.current) {
        await message.channel.send(i18n.t('bot:music.nothing_playing'));
        return;
    }
    
    if (queue.player.state.status === voice.AudioPlayerStatus.Paused) {
        await message.channel.send(i18n.t('bot:music.already_paused'));
        return;
    }
    
    queue.player.pause();
    queue._paused = true;
    queue._pausedElapsed = queue._startedAt
        ? Math.floor((Date.now() - queue._startedAt) / 1000)
        : 0;
    await message.channel.send(i18n.t('bot:music.paused', {
        title: queue.current.title
    }));
}

async function handleResume(message) {
    const queue = getQueue(message.guild.id);
    if (!queue?.player || !queue.current) {
        await message.channel.send(i18n.t('bot:music.nothing_playing'));
        return;
    }
    
    if (queue.player.state.status !== voice.AudioPlayerStatus.Paused) {
        await message.channel.send(i18n.t('bot:music.not_paused'));
        return;
    }
    
    queue.player.unpause();
    // 일시정지 해제 — startedAt 조정하여 진행바 연속성 유지
    if (queue._paused && queue._pausedElapsed > 0) {
        queue._startedAt = Date.now() - queue._pausedElapsed * 1000;
    }
    queue._paused = false;
    await message.channel.send(i18n.t('bot:music.resumed', {
        title: queue.current.title
    }));
}

async function handleSkip(message) {
    const queue = getQueue(message.guild.id);
    if (!queue?.player || !queue.current) {
        await message.channel.send(i18n.t('bot:music.nothing_playing'));
        return;
    }
    
    const nextTrack = queue.tracks.length > 0 ? queue.tracks[0] : null;
    
    if (!nextTrack) {
        // 다음 곡이 없으면 현재 곡 계속 재생, 안내만
        await message.channel.send(i18n.t('bot:music.skipped_no_next'));
        return;
    }
    
    queue.player.stop(true); // force=true: Paused 상태에서도 즉시 Idle 전환
    
    await message.channel.send(i18n.t('bot:music.skipped_next', {
        title: nextTrack.title,
        duration: nextTrack.duration
    }));
}

/**
 * 이전 곡 재생 — 음악 플레이어 방식
 * 7.5초 이상 재생됐으면 현재 곡 처음부터, 이전이면 히스토리에서 복원
 */
async function handlePrevious(guildId) {
    const queue = getQueue(guildId);
    if (!queue?.player) return false;

    // 7.5초 이상 재생됐으면 현재 곡 다시 재생 (restart)
    if (queue.current && queue._startedAt) {
        const elapsedMs = queue._paused
            ? queue._pausedElapsed * 1000
            : Date.now() - queue._startedAt;
        if (elapsedMs >= 7500) {
            // 현재 곡을 큐 맨 앞에 넣고 재생 중단 → playNext가 같은 곡 재생
            queue.tracks.unshift(queue.current);
            queue.current = null;
            queue._startedAt = null;
            queue._paused = false;
            queue._pausedElapsed = 0;
            cleanupPrefetch(queue);
            queue.player.stop(true); // force=true: Paused 상태에서도 즉시 Idle 전환
            return true;
        }
    }

    // 7.5초 이전: 히스토리에서 이전 곡 복원
    if (!queue.history || queue.history.length === 0) {
        return false; // 이전 곡 없음
    }

    const prev = queue.history.shift();

    // 현재 곡을 대기열 맨 앞에 넣기 (다음에 다시 재생될 수 있도록)
    if (queue.current) {
        queue.tracks.unshift(queue.current);
    }

    // 이전 곡을 대기열 맨 앞에 삽입하여 playNext가 재생하도록
    queue.tracks.unshift(prev);

    // 현재 재생 중단 → Idle 핸들러가 playNext 호출
    // Idle 핸들러에서 history에 current를 또 push하는 것 방지를 위해
    // current를 null로 설정
    queue.current = null;
    queue._startedAt = null;
    queue._paused = false;
    queue._pausedElapsed = 0;

    cleanupPrefetch(queue);
    queue.player.stop(true); // force=true: Paused 상태에서도 즉시 Idle 전환
    return true;
}



/**
 * 전용 채널 UI 버튼 인터랙션 핸들러
 * @param {import('discord.js').ButtonInteraction} interaction
 * @returns {boolean} 처리했으면 true
 */
async function handleButtonInteraction(interaction) {
    if (!interaction.isButton()) return false;

    const customId = interaction.customId;
    if (!customId.startsWith('music_')) return false;

    const guildId = interaction.guildId;
    if (!guildId) return false;

    const queue = getQueue(guildId);
    if (!queue) {
        await interaction.deferUpdate().catch(() => {});
        return true;
    }

    await interaction.deferUpdate().catch(() => {});

    switch (customId) {
        case 'music_vol_down': {
            queue.volume = Math.max(queue.volume - 0.1, 0.0);
            if (queue.resource?.volume) queue.resource.volume.setVolume(queue.volume);
            break;
        }
        case 'music_vol_up': {
            queue.volume = Math.min(queue.volume + 0.1, 2.0);
            if (queue.resource?.volume) queue.resource.volume.setVolume(queue.volume);
            break;
        }
        case 'music_prev': {
            await handlePrevious(guildId);
            break;
        }
        case 'music_next': {
            if (queue.player && queue.current) {
                queue.player.stop(true); // force=true: Paused 상태에서도 즉시 Idle 전환
            }
            break;
        }
        case 'music_pause_resume': {
            if (!queue.player || !queue.current) break;
            if (queue._paused) {
                queue.player.unpause();
                if (queue._pausedElapsed > 0) {
                    queue._startedAt = Date.now() - queue._pausedElapsed * 1000;
                }
                queue._paused = false;
            } else {
                queue.player.pause();
                queue._paused = true;
                queue._pausedElapsed = queue._startedAt
                    ? Math.floor((Date.now() - queue._startedAt) / 1000)
                    : 0;
            }
            break;
        }
        case 'music_radio': {
            if (!queue.current) break;
            queue.radio = !queue.radio;
            if (queue.radio) {
                // ON: 시드풀 초기화 + 즉시 큐에 추천곡 채움 (백그라운드)
                queue._radioSeedPool = [queue.current, ...queue.tracks].filter(t => t?.url);
                replenishRadioQueue(guildId).catch(e => {
                    console.warn('[Music/Radio] Button replenish error:', e.message);
                });
            } else {
                // OFF: 큐에서 라디오 트랙 제거 + 시드풀 초기화
                queue._radioFetching = false;
                queue._radioSeedPool = [];
                queue.tracks = queue.tracks.filter(t => !isRadioTrack(t));
                // 제거된 라디오 트랙의 프리페치 정리
                startPrefetchAll(guildId);
            }
            break;
        }
        case 'music_normalize': {
            queue.normalize = !queue.normalize;
            // normalize 설정 변경 → 기존 프리페치 무효화 후 재시작
            cleanupPrefetch(queue);
            startPrefetchAll(guildId);
            break;
        }
        default:
            return false;
    }

    // 버튼 조작 후 UI 즉시 갱신
    channelUI.refreshQueue(guildId).catch(() => {});
    return true;
}

async function handleStop(message) {
    const queue = getQueue(message.guild.id);
    if (!queue) {
        await message.channel.send(i18n.t('bot:music.nothing_playing'));
        return;
    }
    
    destroyQueue(message.guild.id);
    await message.channel.send(i18n.t('bot:music.stopped'));
}

async function handleQueue(message) {
    const queue = getQueue(message.guild.id);
    if (!queue?.current && (!queue?.tracks || queue.tracks.length === 0)) {
        await message.channel.send(i18n.t('bot:music.empty_queue'));
        return;
    }
    
    let text = '';
    
    if (queue.current) {
        text += i18n.t('bot:music.queue_now_playing', {
            title: queue.current.title,
            duration: queue.current.duration
        }) + '\n\n';
    }
    
    if (queue.tracks.length > 0) {
        const display = queue.tracks.slice(0, 10);
        text += i18n.t('bot:music.queue_title', {
            count: queue.tracks.length
        }) + '\n';
        
        display.forEach((track, idx) => {
            const radioPrefix = isRadioTrack(track) ? '📻 ' : '';
            text += `${idx + 1}. ${radioPrefix}**${track.title}** [${track.duration}] — ${track.requester}\n`;
        });
        
        if (queue.tracks.length > 10) {
            text += i18n.t('bot:music.queue_more', {
                count: queue.tracks.length - 10
            });
        }
    }
    
    await message.channel.send(text);
}

async function handleNowPlaying(message) {
    const queue = getQueue(message.guild.id);
    if (!queue?.current) {
        await message.channel.send(i18n.t('bot:music.nothing_playing'));
        return;
    }
    
    const track = queue.current;
    const vol = Math.round(queue.volume * 100);
    await message.channel.send(i18n.t('bot:music.now_playing_detail', {
        title: track.title,
        duration: track.duration,
        requester: track.requester,
        volume: vol,
        url: track.url
    }));
}

async function handleVolume(message, args) {
    const queue = getQueue(message.guild.id);
    if (!queue) {
        await message.channel.send(i18n.t('bot:music.nothing_playing'));
        return;
    }
    
    if (args.length === 0) {
        const vol = Math.round(queue.volume * 100);
        await message.channel.send(i18n.t('bot:music.current_volume', {
            volume: vol
        }));
        return;
    }
    
    const vol = parseInt(args[0], 10);
    if (isNaN(vol) || vol < 0 || vol > 200) {
        await message.channel.send(i18n.t('bot:music.volume_range'));
        return;
    }
    
    queue.volume = vol / 100;
    if (queue.resource?.volume) {
        queue.resource.volume.setVolume(queue.volume);
    }
    
    const emoji = vol === 0 ? '🔇' : vol < 50 ? '🔉' : '🔊';
    await message.channel.send(i18n.t('bot:music.volume_set', {
        volume: vol,
        emoji
    }));
}

async function handleLoop(message, args) {
    const queue = getQueue(message.guild.id);
    if (!queue || !queue.current) {
        await message.channel.send(i18n.t('bot:music.no_track'));
        return;
    }

    // "사바쨩 반복 해제" → args = ['해제']
    const offKeywords = ['해제', 'off', 'disable', '끄기'];
    if (args.length > 0 && offKeywords.includes(args[0].toLowerCase())) {
        return handleLoopOff(message);
    }

    if (queue.loop) {
        await message.channel.send(i18n.t('bot:music.loop_already_on', {
            title: queue.current.title,
        }));
        return;
    }

    queue.loop = true;
    await message.channel.send(i18n.t('bot:music.loop_enabled', {
        title: queue.current.title,
    }));
}

async function handleLoopOff(message) {
    const queue = getQueue(message.guild.id);
    if (!queue || !queue.current) {
        await message.channel.send(i18n.t('bot:music.no_track'));
        return;
    }

    if (!queue.loop) {
        await message.channel.send(i18n.t('bot:music.loop_already_off'));
        return;
    }

    queue.loop = false;
    await message.channel.send(i18n.t('bot:music.loop_disabled', {
        title: queue.current.title,
    }));
}

async function handleShuffle(message) {
    const queue = getQueue(message.guild.id);
    if (!queue || queue.tracks.length < 2) {
        await message.channel.send(i18n.t('bot:music.shuffle_need_more'));
        return;
    }
    
    // Fisher-Yates shuffle
    for (let i = queue.tracks.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [queue.tracks[i], queue.tracks[j]] = [queue.tracks[j], queue.tracks[i]];
    }
    
    // 셔플 후 전체 프리페치 갱신 (URL 자체는 변경 없으므로 기존 프리페치 유지됨)
    startPrefetchAll(message.guild.id);
    
    // 전용 채널 UI 큐 갱신
    channelUI.refreshQueue(message.guild.id).catch(() => {});

    await message.channel.send(i18n.t('bot:music.shuffled', {
        count: queue.tracks.length
    }));
}

// ── 라디오 모드 — YouTube Mix 기반 자동 추천곡 재생 ──

/**
 * YouTube URL에서 비디오 ID(11자) 추출
 * @param {string} url
 * @returns {string|null}
 */
function extractVideoId(url) {
    if (!url) return null;
    const match = url.match(
        /(?:youtube\.com\/watch\?.*v=|youtu\.be\/|youtube\.com\/embed\/|music\.youtube\.com\/watch\?.*v=)([A-Za-z0-9_-]{11})/
    );
    return match ? match[1] : null;
}

/**
 * 이미 재생한 곡을 후보에서 제거
 */
function filterPlayedTracks(candidates, history, currentUrl) {
    const played = new Set(history.map(t => t.url));
    if (currentUrl) played.add(currentUrl);
    return candidates.filter(t => !played.has(t.url));
}

/**
 * YouTube Mix(Radio) 플레이리스트에서 추천곡을 가져옵니다.
 * seed 비디오 기반으로 YouTube의 자동 추천 알고리즘을 활용합니다.
 * @param {string} seedUrl - 기준이 되는 YouTube URL
 * @param {object[]} history - 이미 재생한 곡 목록 (중복 방지)
 * @param {number} [count=RADIO_FILL_COUNT] - 가져올 곡 수
 * @returns {Promise<object[]>} 추천 트랙 배열 (빈 배열이면 실패)
 */
async function fetchRadioTracks(seedUrl, history, count = RADIO_FILL_COUNT) {
    const videoId = extractVideoId(seedUrl);
    if (!videoId) {
        console.warn('[Music/Radio] Cannot extract video ID from seed URL');
        return [];
    }

    const mixUrl = `https://www.youtube.com/watch?v=${videoId}&list=RD${videoId}`;
    console.log(`[Music/Radio] Fetching mix playlist: ${mixUrl}`);

    // yt-dlp --flat-playlist -j 로 Mix 플레이리스트 메타데이터 추출
    // --playlist-end: YouTube Mix는 동적 무한 생성이므로 반드시 상한 필요
    const playlistEnd = Math.max(count * 3, 15); // dedup 여유분 확보
    const tracks = await new Promise((resolve) => {
        const proc = spawn(ytDlpPath, [
            '--flat-playlist', '--no-warnings', '-j',
            '--playlist-end', String(playlistEnd),
            mixUrl,
        ], { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });

        let stdout = '';
        proc.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
        proc.stderr.on('data', () => {});

        const timer = setTimeout(() => {
            proc.kill();
            console.warn('[Music/Radio] yt-dlp mix fetch timed out (20s)');
            resolve(null);
        }, 20_000);

        proc.on('error', (err) => {
            clearTimeout(timer);
            console.warn('[Music/Radio] yt-dlp spawn error:', err.message);
            resolve(null);
        });

        proc.on('close', () => {
            clearTimeout(timer);
            try {
                const result = stdout.trim();
                if (!result) { resolve(null); return; }

                const lines = result.split('\n').filter(l => l.trim());
                const parsed = [];
                for (const line of lines) {
                    try {
                        const data = JSON.parse(line);
                        // --flat-playlist: url 필드가 비디오 ID(11자)이면 전체 URL로 변환
                        // webpage_url이 있으면 우선 사용 (전체 URL)
                        let trackUrl = data.webpage_url;
                        if (!trackUrl && data.url) {
                            trackUrl = /^https?:\/\//.test(data.url)
                                ? data.url
                                : `https://www.youtube.com/watch?v=${data.id || data.url}`;
                        }
                        if (!trackUrl) continue;
                        const duration = data.duration
                            ? `${Math.floor(data.duration / 60)}:${String(Math.floor(data.duration) % 60).padStart(2, '0')}`
                            : '??:??';
                        parsed.push({
                            title: data.title || 'Unknown',
                            url: trackUrl,
                            duration,
                        });
                    } catch (_) {}
                }
                resolve(parsed.length > 0 ? parsed : null);
            } catch (e) {
                console.warn('[Music/Radio] Mix parse failed:', e.message);
                resolve(null);
            }
        });
    });

    if (!tracks) {
        // Mix 실패 시, 현재 곡 제목으로 검색 fallback
        console.log('[Music/Radio] Mix fetch failed, trying search fallback');
        try {
            const fallbackResults = await extractTrackInfo(seedUrl, RADIO_REQUESTER, count);
            const filtered = filterPlayedTracks(fallbackResults, history, seedUrl);
            return filtered.slice(0, count).map(t => ({ ...t, requester: RADIO_REQUESTER }));
        } catch (e) {
            console.warn('[Music/Radio] Search fallback also failed:', e.message);
            return [];
        }
    }

    // 히스토리와 현재 곡 URL 제외
    const filtered = filterPlayedTracks(tracks, history, seedUrl);
    if (filtered.length === 0) {
        console.log('[Music/Radio] All mix tracks already played');
        return [];
    }

    return filtered.slice(0, count).map(t => ({ ...t, requester: RADIO_REQUESTER }));
}

/**
 * 라디오 큐에 추천곡을 보충합니다 (프로액티브).
 * _radioFetching 플래그로 동시 호출을 방지합니다.
 * @param {string} guildId
 * @returns {Promise<number>} 추가된 곡 수
 */
async function replenishRadioQueue(guildId) {
    const queue = getQueue(guildId);
    if (!queue || !queue.radio || queue._radioFetching) return 0;

    const radioCount = queue.tracks.filter(isRadioTrack).length;
    if (radioCount > RADIO_REPLENISH_THRESHOLD) return 0;

    queue._radioFetching = true;
    try {
        // seed: 시드풀에서 랜덤 선택 (다양한 추천을 위해)
        // 시드풀이 비어있으면 current → history fallback
        let seed;
        if (queue._radioSeedPool.length > 0) {
            const idx = Math.floor(Math.random() * queue._radioSeedPool.length);
            seed = queue._radioSeedPool[idx];
        } else {
            seed = queue.current || (queue.history.length > 0 ? queue.history[0] : null);
        }
        if (!seed?.url) return 0;

        const needed = RADIO_FILL_COUNT - radioCount;
        console.log(`[Music/Radio] Replenishing: ${radioCount} radio tracks left, fetching ${needed} more (seed: ${seed.title})`);
        const newTracks = await fetchRadioTracks(seed.url, queue._radioHistory, needed);
        if (newTracks.length === 0) return 0;

        queue.tracks.push(...newTracks);
        // _radioHistory에 새 곡 추가 (중복 방지 범위 확장)
        for (const t of newTracks) {
            queue._radioHistory.unshift(t);
        }
        if (queue._radioHistory.length > 30) queue._radioHistory.splice(30);

        // 시드풀에도 새 곡 추가 (다양성 확장)
        queue._radioSeedPool.push(...newTracks);
        if (queue._radioSeedPool.length > RADIO_SEED_POOL_MAX) {
            queue._radioSeedPool.splice(0, queue._radioSeedPool.length - RADIO_SEED_POOL_MAX);
        }

        channelUI.refreshQueue(guildId).catch(() => {});
        // 새로 추가된 라디오 곡도 프리페치 시작
        startPrefetchAll(guildId);
        console.log(`[Music/Radio] Added ${newTracks.length} tracks to queue`);
        return newTracks.length;
    } catch (e) {
        console.error('[Music/Radio] Replenish failed:', e.message);
        return 0;
    } finally {
        queue._radioFetching = false;
    }
}

/**
 * 라디오 모드 토글 명령어
 */
async function handleRadio(message, args) {
    const queue = getQueue(message.guild.id);

    // "라디오 끄기/off" 명시적 지정
    const offKeywords = ['off', 'disable', '끄기', '해제'];
    const onKeywords = ['on', 'enable', '켜기', '활성'];

    /**
     * 라디오 ON 공통 로직: 시드풀 초기화 + 즉시 큐에 추천곡 채움
     */
    async function activateRadio() {
        queue.radio = true;
        // 시드풀 초기화: current + 기존 큐의 모든 곡
        queue._radioSeedPool = [queue.current, ...queue.tracks].filter(t => t?.url);
        await message.channel.send(i18n.t('bot:music.radio_enabled', {
            title: queue.current.title,
        }));
        // 즉시 추천곡으로 큐 채움 (프로액티브)
        const added = await replenishRadioQueue(message.guild.id);
        if (added > 0) {
            await message.channel.send(i18n.t('bot:music.radio_filled', {
                count: added,
            }));
        }
    }

    /**
     * 라디오 OFF 공통 로직: 큐에서 라디오 트랙 제거
     */
    function deactivateRadio() {
        queue.radio = false;
        queue._radioFetching = false;
        queue._radioSeedPool = [];
        // 큐에서 라디오 자동 트랙 제거
        queue.tracks = queue.tracks.filter(t => !isRadioTrack(t));
    }

    if (args.length > 0) {
        if (offKeywords.includes(args[0].toLowerCase())) {
            if (!queue || !queue.radio) {
                await message.channel.send(i18n.t('bot:music.radio_already_off'));
                return;
            }
            deactivateRadio();
            await message.channel.send(i18n.t('bot:music.radio_disabled'));
            channelUI.refreshQueue(message.guild.id).catch(() => {});
            return;
        }
        if (onKeywords.includes(args[0].toLowerCase())) {
            if (!queue?.current) {
                await message.channel.send(i18n.t('bot:music.radio_need_track'));
                return;
            }
            if (queue.radio) {
                await message.channel.send(i18n.t('bot:music.radio_already_on'));
                return;
            }
            await activateRadio();
            channelUI.refreshQueue(message.guild.id).catch(() => {});
            return;
        }
    }

    // 토글
    if (!queue?.current) {
        await message.channel.send(i18n.t('bot:music.radio_need_track'));
        return;
    }

    if (queue.radio) {
        deactivateRadio();
        await message.channel.send(i18n.t('bot:music.radio_disabled'));
    } else {
        await activateRadio();
    }
    channelUI.refreshQueue(message.guild.id).catch(() => {});
}

// ── 오디오 정규화 토글 ──
async function handleNormalize(message, args) {
    const queue = getQueue(message.guild.id);

    const offKeywords = ['off', 'disable', '끄기', '해제'];
    const onKeywords = ['on', 'enable', '켜기', '활성'];

    if (args.length > 0) {
        const arg = args[0].toLowerCase();
        if (offKeywords.includes(arg)) {
            if (queue) {
                queue.normalize = false;
                cleanupPrefetch(queue);
                startPrefetchAll(message.guild.id);
            }
            await message.channel.send(i18n.t('bot:music.normalize_disabled'));
            return;
        }
        if (onKeywords.includes(arg)) {
            if (queue) {
                queue.normalize = true;
                cleanupPrefetch(queue);
                startPrefetchAll(message.guild.id);
            }
            await message.channel.send(i18n.t('bot:music.normalize_enabled'));
            return;
        }
    }

    // 토글
    if (queue) {
        queue.normalize = !queue.normalize;
        cleanupPrefetch(queue);
        startPrefetchAll(message.guild.id);
    }
    const enabled = queue ? queue.normalize : true;
    if (enabled) {
        await message.channel.send(i18n.t('bot:music.normalize_enabled'));
    } else {
        await message.channel.send(i18n.t('bot:music.normalize_disabled'));
    }
}

async function handleHelp(message, args, botConfig) {
    const prefix = botConfig.prefix;
    const mod = i18n.t('bot:music.mod_name');
    
    const help = i18n.t('bot:music.help', {
        prefix,
        mod
    });
    
    await message.channel.send(help);
}

// ── 음악 명령어 목록 (GUI 설정용 export) ──
const MUSIC_COMMAND_LIST = Object.keys(MUSIC_COMMANDS);

/**
 * 음성 채널 상태 변경 핸들러 — 채널에 봇만 남으면 자동 퇴장
 */
function handleVoiceStateUpdate(oldState, newState) {
    if (!musicAvailable) return;

    // 누군가 음성 채널을 떠났을 때만 처리 (oldState.channel이 있어야 함)
    const channel = oldState.channel;
    if (!channel) return;

    const guildId = oldState.guild.id;
    const queue = getQueue(guildId);
    if (!queue || !queue.connection) return;

    // 봇이 있는 채널인지 확인
    const botMember = oldState.guild.members.me;
    if (!botMember || !botMember.voice.channel) return;
    if (channel.id !== botMember.voice.channel.id) return;

    // 봇 외에 사람이 남아 있는지 확인
    const humans = channel.members.filter(m => !m.user.bot);
    if (humans.size === 0) {
        console.log(`[Music] Voice channel empty in guild ${guildId}, auto-leaving`);
        destroyQueue(guildId);
    }
}

// ══════════════════════════════════════════════
// ── 전용 음악 채널 처리 ──
// ══════════════════════════════════════════════

/**
 * 전용 채널용 "임시" 메시지 래퍼.
 * 핸들러가 message.reply() / message.channel.send() 를 호출하면
 * 실제 메시지를 전송하되, 10초 후 자동 삭제합니다.
 * 재생 로직에 필요한 message.guild, message.member 등은 원본을 그대로 전달합니다.
 */
const EPHEMERAL_DELETE_DELAY = 10_000; // 10초

function _createEphemeralMessage(message) {
    /**
     * 임시 메시지를 감싸서 마지막 edit() 이후 10초 후 삭제되도록 관리.
     * 최초 send 시점이 아닌, 마지막 수정 시점 기준으로 삭제 타이머를 갱신합니다.
     */
    function _wrapEphemeral(sentMsg) {
        let deleteTimer = setTimeout(() => {
            if (sentMsg.deletable) sentMsg.delete().catch(() => {});
        }, EPHEMERAL_DELETE_DELAY);

        const origEdit = sentMsg.edit.bind(sentMsg);
        sentMsg.edit = async (...args) => {
            // edit할 때마다 삭제 타이머 리셋
            clearTimeout(deleteTimer);
            try {
                const result = await origEdit(...args);
                deleteTimer = setTimeout(() => {
                    if (sentMsg.deletable) sentMsg.delete().catch(() => {});
                }, EPHEMERAL_DELETE_DELAY);
                return result;
            } catch (_) {
                return sentMsg;
            }
        };

        return sentMsg;
    }

    const _fakeSentMsg = {
        edit: async () => _fakeSentMsg,
        delete: async () => {},
        reply: async () => _fakeSentMsg,
        id: '0',
        deletable: false,
        components: [],
        content: '',
    };

    const ephemeralSend = async (...sendArgs) => {
        try {
            const sent = await message.channel.send(...sendArgs);
            return _wrapEphemeral(sent);
        } catch (_) {
            return _fakeSentMsg;
        }
    };

    const ephemeralReply = async (...replyArgs) => {
        // reply 대신 send로 전환 (전용 채널에서 reply 인용 불필요)
        const content = typeof replyArgs[0] === 'string' ? replyArgs[0] : replyArgs[0];
        return ephemeralSend(content);
    };

    const ephemeralChannel = Object.create(message.channel);
    ephemeralChannel.send = ephemeralSend;

    return new Proxy(message, {
        get(target, prop) {
            if (prop === 'reply') return ephemeralReply;
            if (prop === 'channel') return ephemeralChannel;
            return Reflect.get(target, prop);
        },
    });
}

/**
 * 전용 음악 채널에서의 메시지 처리 — prefix 없이 바로 명령어/검색어 입력
 * 전용 채널에서는 텍스트 응답 없이 UI(진행바·큐)만 갱신합니다.
 * @param {import('discord.js').Message} message
 * @param {object} botConfig
 * @returns {boolean} 처리했으면 true
 */
async function handleMusicChannelMessage(message, botConfig) {
    if (!musicAvailable) return false;
    if (message.author.bot) return false;
    if (botConfig.musicEnabled === false) return false;
    if (isRelayMessage(message)) return false;

    const guildId = message.guildId;
    if (!channelUI.isMusicChannel(guildId, message.channel.id, botConfig)) return false;

    // 전용 채널 메시지 → 항상 삭제 (채널 청결 유지)
    channelUI.deleteUserMessage(message);

    const content = message.content.trim();
    if (!content) return true;

    // 핸들러용 임시 메시지 — 오류/상태 표시 후 10초 뒤 자동 삭제
    const silent = _createEphemeralMessage(message);

    const args = content.split(/\s+/);
    const firstArg = args[0];

    // 명령어 해석 시도
    const customAliases = botConfig.commandAliases?.music || {};
    const commandName = resolveMusicCommand(firstArg, customAliases);

    if (commandName) {
        // 전용 채널에서 search → play로 리다이렉트 (버튼 UI 비활성 채널)
        if (commandName === 'search') {
            if (!await requireVoiceChannel(silent)) return true;
            await handlePlay(silent, args.slice(1), botConfig);
            return true;
        }
        const cmdDef = MUSIC_COMMANDS[commandName];
        if (cmdDef.needsVoice && !await requireVoiceChannel(silent)) return true;
        await cmdDef.handler(silent, args.slice(1), botConfig);
        return true;
    }

    // URL이면 바로 재생
    if (isYouTubeUrl(firstArg)) {
        if (!await requireVoiceChannel(silent)) return true;
        await handlePlay(silent, args, botConfig);
        return true;
    }

    // 그 외 → 검색어로 간주하여 바로 재생
    if (!await requireVoiceChannel(silent)) return true;
    await handlePlay(silent, args, botConfig);
    return true;
}

/**
 * 전용 채널 UI 시작 — 봇 Ready 시 또는 설정 변경 시 호출
 * @param {import('discord.js').Client} client
 * @param {object} botConfig
 */
async function initMusicChannelUI(client, botConfig) {
    // bot-config.json의 normalize 설정을 매 리로드 시 반영
    _defaultNormalize = botConfig?.musicUISettings?.normalize !== false;

    if (!musicAvailable) {
        console.warn('[Music] Channel UI skipped: music not available');
        return;
    }
    if (!botConfig.musicChannelId) {
        // 채널 ID가 비어있으면 모든 길드의 채널 UI 정지
        stopAllMusicChannelUI();
        return;
    }

    console.log(`[Music] Initializing channel UI (channelId=${botConfig.musicChannelId})`);

    // 길드별 또는 단일 채널 ID 지원
    const channelIds = typeof botConfig.musicChannelId === 'object'
        ? Object.entries(botConfig.musicChannelId)
        : [[null, botConfig.musicChannelId]];

    for (const [guildId, channelId] of channelIds) {
        try {
            const channel = await client.channels.fetch(channelId);
            if (!channel) {
                console.warn(`[Music] Music channel ${channelId} not found`);
                continue;
            }
            if (!channel.isTextBased()) {
                console.warn(`[Music] Music channel ${channelId} is not text-based (type: ${channel.type})`);
                continue;
            }
            const resolvedGuildId = guildId || channel.guildId;
            const settings = channelUI.getMusicChannelSettings(botConfig);

            await channelUI.startChannelUI(
                resolvedGuildId,
                channel,
                () => getQueue(resolvedGuildId),
                settings,
            );
            console.log(`[Music] Channel UI active for guild ${resolvedGuildId} in #${channel.name}`);
        } catch (e) {
            console.error(`[Music] Failed to init music channel UI for ${channelId}:`, e.message);
        }
    }
}

/**
 * 전용 채널 UI 정지 (모든 길드)
 */
function stopAllMusicChannelUI() {
    channelUI.stopAllChannelUI();
    console.log('[Music] All channel UIs stopped');
}

module.exports = {
    handleMusicMessage,
    handleMusicShortcut,
    handleMusicChannelMessage,
    handleVoiceStateUpdate,
    handleButtonInteraction,
    initMusicChannelUI,
    isMusicModule,
    hasActiveQueue,
    musicAvailable: () => musicAvailable,
    init,
    cleanup,
    destroyGuildQueue: destroyQueue,
    MUSIC_COMMAND_LIST,
    DEFAULT_MODULE_ALIASES,
    DEFAULT_COMMAND_ALIASES,
    getEffectiveCommandAliases,
    getEffectiveModuleAliases,
    channelUI,
    // 테스트 전용 내부 접근
    _test: {
        handlePrevious,
        getQueue,
        getOrCreateQueue,
        guildQueues,
        classifyError: _classifyError,
        formatPlayError: _formatPlayError,
    },
};

/**
 * Music extension 정리 — 플래그(환경변수) 해제.
 * 봇 종료 또는 익스텐션 언로드 시 반드시 호출해야 합니다.
 */
function cleanup() {
    // NODE_PATH 복원
    if (_originalNodePath === undefined) {
        delete process.env.NODE_PATH;
    } else {
        process.env.NODE_PATH = _originalNodePath;
    }
    // FFMPEG_PATH 복원
    if (_originalFfmpegPath === undefined) {
        delete process.env.FFMPEG_PATH;
    } else {
        process.env.FFMPEG_PATH = _originalFfmpegPath;
    }
    musicAvailable = false;
    console.log('[Music] Cleanup complete — environment flags restored');
}
