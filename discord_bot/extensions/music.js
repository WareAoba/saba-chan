/**
 * 🎵 사바쨩 Music Extension
 * 
 * 이스터에그 / 추가 기능으로 포함된 음성 채널 음악 재생 익스텐션.
 * 게임 서버 관리와는 별개로, 디스코드 음성 채널에서 유튜브 음악을 재생합니다.
 * 
 * 의존성:
 *   - Node.js (discord_bot/package.json): @discordjs/voice, opusscript, play-dl, sodium-native
 *   - Python (extensions/music/music_deps.py 자동설치): ffmpeg, yt-dlp
 */

const i18n = require('../i18n');
const { spawn, execSync } = require('child_process');
const { PassThrough } = require('stream');
const path = require('path');
const fs = require('fs');
const { ActionRowBuilder, ButtonBuilder, ButtonStyle } = require('discord.js');
const { getSabaDataDir } = require('../utils/constants');

// ── Lazy imports (패키지 미설치 시 graceful fallback) ──
let voice, playDl;
let musicAvailable = false;
let ffmpegPath = 'ffmpeg';
let ytDlpPath = 'yt-dlp';

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

    // Python 실행 파일 결정
    const pythonCandidates = [];
    if (process.platform === 'win32') {
        // venv (개발 환경)
        const venvPython = path.resolve(__dirname, '..', '..', '.venv', 'Scripts', 'python.exe');
        if (fs.existsSync(venvPython)) pythonCandidates.push(venvPython);
        // 사바쨩 내장 Python (프로덕션)
        const embeddedPython = path.join(getSabaDataDir(), 'python', 'python.exe');
        if (fs.existsSync(embeddedPython)) pythonCandidates.push(embeddedPython);
    }
    pythonCandidates.push('python3', 'python');

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

try {
    // ── Python 익스텐션이 설치한 바이너리 경로 읽기 ──
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
        // .deps-resolved.json 미발견 — Python music_deps.py 직접 실행하여 생성 시도
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

    // ── prism-media FFmpeg 경로 등록 ──────────────────────────────
    // @discordjs/voice 는 내부적으로 prism-media 의 FFmpeg 을 사용합니다.
    // prism-media 는 process.env.FFMPEG_PATH 를 확인하지 않고
    // require('ffmpeg-static') / PATH 의 'ffmpeg' 만 탐색하므로,
    // 우리 ffmpegPath 를 직접 등록해야 합니다.
    try {
        const prism = require('prism-media');
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
    try { playDl = require('play-dl'); } catch (_) {}
    musicAvailable = true;
    console.log('[Music] Extension loaded successfully 🎵');
} catch (e) {
    console.warn('[Music] Extension not available — missing Node.js packages.');
    console.warn('[Music] Run: cd discord_bot && npm install');
    console.warn('[Music] Binary tools (ffmpeg, yt-dlp) are managed by the Python music extension.');
}

// ── Per-guild state ──
const guildQueues = new Map();
const guildLocks = new Map(); // 길드별 비동기 락 (동시 playNext 방지)

/**
 * 길드별 비동기 락 — playNext 등 동시에 하나만 실행되도록 보장
 */
function acquireLock(guildId) {
    if (!guildLocks.has(guildId)) {
        guildLocks.set(guildId, Promise.resolve());
    }
    let release;
    const prev = guildLocks.get(guildId);
    const next = new Promise((resolve) => { release = resolve; });
    guildLocks.set(guildId, prev.then(() => next));
    // 이전 락이 풀릴 때까지 대기 후 release 함수 반환
    return prev.then(() => release);
}

// 기본 볼륨 (0.0 ~ 1.0)
const DEFAULT_VOLUME = 0.5;
// 아무도 안 들으면 자동 퇴장 (ms)
const IDLE_TIMEOUT = 5 * 60 * 1000; // 5분

/**
 * Guild별 Queue 객체 생성
 */
function createQueue(guildId) {
    return {
        guildId,
        tracks: [],         // { title, url, duration, requester }
        current: null,
        connection: null,
        player: null,
        resource: null,
        volume: DEFAULT_VOLUME,
        loop: false,
        idleTimer: null,
        prefetch: null,     // { url, stream } — 다음 곡 미리 버퍼링
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

function destroyQueue(guildId) {
    const queue = guildQueues.get(guildId);
    if (queue) {
        if (queue.idleTimer) clearTimeout(queue.idleTimer);
        if (queue.player) queue.player.stop(true);
        cleanupPrefetch(queue);
        if (queue.connection) {
            try { queue.connection.destroy(); } catch (_) {}
        }
        guildQueues.delete(guildId);
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
    help:    { handler: handleHelp,    needsVoice: false },
};

/**
 * 음악 명령어의 기본 별명 맵 (코드 내장)
 * bot-config.json의 commandAliases.music 에서 사용자 커스텀 가능
 */
const DEFAULT_COMMAND_ALIASES = {
    play:    ['재생', 'p', 'ㅈㅅ'],
    search:  ['검색', 'find', 'ㄱㅅ'],
    pause:   ['일시정지', 'ㅇㅅㅈㅈ'],
    resume:  ['계속', 'ㄱㅅㄱ'],
    skip:    ['다음', 'ㄷㅇ', 's', 'next'],
    stop:    ['정지', 'ㅈㅈ', 'leave', 'disconnect', 'dc'],
    queue:   ['대기열', 'ㄷㄱㅇ', 'q', 'list'],
    np:      ['지금', 'ㅈㄱ', 'nowplaying', 'now'],
    volume:  ['볼륨', 'ㅂㄹ', 'vol', 'v'],
    shuffle: ['섞기', 'ㅅㄱ', 'random'],
    loop:    ['반복', 'ㅂㅂ', 'repeat'],
    loopoff: ['반복해제', 'ㅂㅂㅎㅈ', 'unloop'],
    help:    ['도움', 'ㄷㅇ말'],
};

/**
 * 기본 모듈 별명 (music 모듈 접근용)
 */
const DEFAULT_MODULE_ALIASES = ['music', '음악', 'ㄴㄹ', 'ㅇㅇ', 'dj'];

/**
 * 명령어 별명 해석
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
    
    // 3. 기본 내장 별명
    for (const [cmdName, aliases] of Object.entries(DEFAULT_COMMAND_ALIASES)) {
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
    
    // 기본 별명
    if (DEFAULT_MODULE_ALIASES.includes(lower)) return true;
    
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
    
    // "사바쨩 정지/ㅈㅈ/stop/leave/dc" → 음악이 재생 중이면 음악 정지
    const stopAliases = ['정지', 'ㅈㅈ', 'stop', 'leave', 'disconnect', 'dc'];
    if (args.length === 1 && stopAliases.includes(firstArg.toLowerCase())) {
        if (hasActiveQueue(message.guild?.id)) {
            if (!await requireVoiceChannel(message)) return true;
            safeDelete(message);
            await handleStop(message);
            return true;
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
async function extractTrackInfo(query, requester) {
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
                console.warn('[Music] play-dl video_info failed, trying yt-dlp:', e.message);
            }
        }

        // yt-dlp fallback
        const info = await getTrackInfoViaYtDlp(query);
        if (info) return [{ ...info, requester }];

        throw new Error(i18n.t('bot:music.invalid_url'));
    }
    
    // 검색: play-dl 시도 → yt-dlp fallback
    if (playDl) {
        try {
            const results = await playDl.search(query, { limit: 5, source: { youtube: 'video' } });
            if (results.length > 0) {
                return results.map(v => ({
                    title: v.title || 'Unknown',
                    url: v.url,
                    duration: v.durationRaw || '??:??',
                    requester,
                }));
            }
        } catch (e) {
            console.warn('[Music] play-dl search failed, trying yt-dlp:', e.message);
        }
    }

    // yt-dlp 검색 fallback (최대 5개)
    const info = await getTrackInfoViaYtDlp(`ytsearch5:${query}`);
    if (info) {
        // yt-dlp -j with ytsearchN returns one JSON per line
        if (Array.isArray(info)) return info.map(t => ({ ...t, requester }));
        return [{ ...info, requester }];
    }

    throw new Error(i18n.t('bot:music.no_results'));
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
                            const duration = data.duration
                                ? `${Math.floor(data.duration / 60)}:${String(Math.floor(data.duration) % 60).padStart(2, '0')}`
                                : '??:??';
                            tracks.push({
                                title: data.title || data.fulltitle || 'Unknown',
                                url: data.webpage_url || data.url || query,
                                duration,
                            });
                        } catch (_) {}
                    }
                    resolve(tracks.length > 0 ? tracks : null);
                    return;
                }

                const data = JSON.parse(result);
                const duration = data.duration
                    ? `${Math.floor(data.duration / 60)}:${String(Math.floor(data.duration) % 60).padStart(2, '0')}`
                    : '??:??';
                resolve({
                    title: data.title || data.fulltitle || 'Unknown',
                    url: data.webpage_url || data.url || query,
                    duration,
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
 * yt-dlp stdout → ffmpeg stdin → ffmpeg stdout (Ogg/Opus) → Discord
 */
function createYtDlpStream(url) {
    // yt-dlp: 오디오만 추출, stdout으로 출력
    const ytdlp = spawn(ytDlpPath, [
        '-f', 'worstaudio',       // 최소 용량 소스 — 대역폭 절약
        '--no-playlist',
        '-o', '-',  // stdout으로 출력
        '--quiet',
        '--no-warnings',
        '--buffer-size', '64K',     // HTTP 다운로드 버퍼 (기본 1K)
        '--concurrent-fragments', '4', // 병렬 다운로드
        url,
    ], { stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });

    ytdlp.stderr.on('data', (data) => {
        const msg = data.toString().trim();
        if (msg) console.warn(`[Music] yt-dlp stderr: ${msg}`);
    });

    // ffmpeg: stdin에서 받아서 Ogg/Opus로 변환, stdout으로 출력
    const ffmpeg = spawn(ffmpegPath, [
        '-hide_banner',
        '-loglevel', 'error',
        '-i', 'pipe:0',           // stdin에서 입력
        '-vn',                    // 영상 제거
        '-acodec', 'libopus',     // Opus 코덱
        '-b:a', '64k',            // 64kbps — 대역폭 절약 (Discord 채널 상한)
        '-f', 'ogg',              // Ogg 컨테이너
        '-ar', '48000',           // 48kHz (Discord 표준)
        '-ac', '2',               // 스테레오
        'pipe:1',                 // stdout으로 출력
    ], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });

    ffmpeg.stderr.on('data', (data) => {
        const msg = data.toString().trim();
        if (msg) console.warn(`[Music] ffmpeg stderr: ${msg}`);
    });

    // 파이프라인: yt-dlp stdout → ffmpeg stdin
    // EPIPE 방지: pipe 양쪽에 에러 핸들러 등록
    ytdlp.stdout.on('error', (err) => {
        if (err.code !== 'EPIPE') console.warn('[Music] yt-dlp stdout error:', err.message);
    });
    ffmpeg.stdin.on('error', (err) => {
        if (err.code !== 'EPIPE') console.warn('[Music] ffmpeg stdin error:', err.message);
    });
    ytdlp.stdout.pipe(ffmpeg.stdin);

    // 에러 처리
    ytdlp.on('error', (err) => {
        console.error('[Music] yt-dlp spawn error:', err.message);
        try { ffmpeg.kill(); } catch (_) {}
    });
    ffmpeg.on('error', (err) => {
        console.error('[Music] ffmpeg spawn error:', err.message);
        try { ytdlp.kill(); } catch (_) {}
    });

    // yt-dlp가 비정상 종료되면 ffmpeg stdin 닫기
    ytdlp.on('close', (code) => {
        if (code !== 0) {
            console.warn(`[Music] yt-dlp exited with code ${code}`);
        }
        ffmpeg.stdin.end();
    });

    // 대용량 중간 버퍼: YouTube throttling에 의한 끊김 방지
    // 8MB ≈ Opus 96kbps 기준 약 10분 분량의 오디오
    const AUDIO_BUFFER_SIZE = 8 * 1024 * 1024;
    const buffer = new PassThrough({ highWaterMark: AUDIO_BUFFER_SIZE });
    
    ffmpeg.stdout.pipe(buffer);
    
    // cleanup: 버퍼 스트림이 닫히면 프로세스도 정리
    buffer.on('close', () => {
        ytdlp.kill();
        ffmpeg.kill();
    });
    buffer.on('error', () => {
        ytdlp.kill();
        ffmpeg.kill();
    });

    // 프리버퍼 진행률 추적용
    buffer._ytdlp = ytdlp;
    buffer._ffmpeg = ffmpeg;

    return buffer;
}

// ── 다음 곡 프리페치 ──

/**
 * 현재 재생 중일 때 대기열의 다음 1곡을 미리 다운로드+버퍼링 시작
 */
function startPrefetch(guildId) {
    const queue = getQueue(guildId);
    if (!queue) return;
    
    // 대기열에 다음 곡이 없으면 패스
    if (queue.tracks.length === 0) return;
    
    const nextTrack = queue.tracks[0]; // peek (shift하지 않음)
    
    // 이미 같은 URL을 프리페치 중이면 스킵
    if (queue.prefetch && queue.prefetch.url === nextTrack.url) return;
    
    // 기존 프리페치 정리
    cleanupPrefetch(queue);
    
    console.log(`[Music] Prefetching next: ${nextTrack.title}`);
    const stream = createYtDlpStream(nextTrack.url);
    queue.prefetch = { url: nextTrack.url, stream };
}

/**
 * 프리페치 스트림 정리
 */
function cleanupPrefetch(queue) {
    if (queue.prefetch) {
        try {
            const s = queue.prefetch.stream;
            if (s._ytdlp) s._ytdlp.kill();
            if (s._ffmpeg) s._ffmpeg.kill();
            s.destroy();
        } catch (_) {}
        queue.prefetch = null;
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
        queue.current = null;
        // 대기열 비었음 → 일정 시간 후 자동 퇴장
        startIdleTimer(guildId);
        return;
    }
    
    clearIdleTimer(guildId);
    
    const track = queue.tracks.shift();
    queue.current = track;
    
    try {
        // 연결이 Ready 상태가 될 때까지 대기 (최대 15초)
        if (queue.connection.state.status !== voice.VoiceConnectionStatus.Ready) {
            console.log(`[Music] Waiting for voice connection ready (current: ${queue.connection.state.status})...`);
            try {
                await voice.entersState(queue.connection, voice.VoiceConnectionStatus.Ready, 15_000);
            } catch (e) {
                console.error('[Music] Voice connection failed to become ready:', e.message);
                queue.current = null;
                return;
            }
        }

        console.log(`[Music] Streaming: ${track.title} (${track.url})`);
        
        // 프리페치된 스트림이 있으면 재활용
        let audioStream;
        if (queue.prefetch && queue.prefetch.url === track.url) {
            audioStream = queue.prefetch.stream;
            queue.prefetch = null; // 소유권 이전 (정리 방지)
            console.log(`[Music] Using prefetched stream (${audioStream.readableLength} bytes already buffered)`);
        } else {
            cleanupPrefetch(queue); // URL 불일치 → 기존 프리페치 폐기
            audioStream = createYtDlpStream(track.url);
            console.log(`[Music] yt-dlp+ffmpeg stream created, pre-buffering...`);
        }
        
        // 프리버퍼링: 재생 전 최소 데이터 축적 대기 (끊김 방지)
        // 128KB ≈ Opus 64kbps 기준 약 16초 분량 — 8MB PassThrough 버퍼가
        // 재생 중 계속 채우므로 초기에 많이 기다릴 필요 없음
        const PRE_BUFFER_BYTES = 128 * 1024; // 128KB
        const PRE_BUFFER_TIMEOUT = 5000;      // 최대 5초 대기
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
                console.log(`[Music] Pre-buffer timeout, starting with ${audioStream.readableLength} bytes`);
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
            });
            
            queue.player.on(voice.AudioPlayerStatus.Idle, () => {
                if (queue.loop && queue.current) {
                    queue.tracks.unshift(queue.current);
                }
                playNext(guildId).catch(err => {
                    console.error('[Music] playNext error (from Idle handler):', err.message);
                });
            });
            
            queue.player.on('error', (err) => {
                console.error('[Music] Player error:', err.message);
                playNext(guildId).catch(err2 => {
                    console.error('[Music] playNext error (from error handler):', err2.message);
                });
            });
            
            queue.connection.subscribe(queue.player);
            console.log('[Music] Player created and subscribed to connection');
        }
        
        queue.player.play(resource);
        console.log(`[Music] play() called — player status: ${queue.player.state.status}`);
        
        // 다음 곡 프리페치 시작
        startPrefetch(guildId);
    } catch (e) {
        console.error('[Music] Stream error:', e.message);
        // 스트림 실패 → 다음 곡으로
        playNext(guildId);
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
        const candidates = await extractTrackInfo(query, message.author.tag);
        
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
        await statusMsg.edit(`❌ ${e.message}`).catch(() => {});
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
    
    const query = args.join(' ');
    
    safeDelete(message);
    
    const statusMsg = await message.channel.send(i18n.t('bot:music.searching', {
        query: query.length > 60 ? query.substring(0, 57) + '...' : query
    }));
    
    try {
        // URL이면 바로 재생 (검색 UI 불필요)
        if (isYouTubeUrl(query)) {
            const tracks = await extractTrackInfo(query, message.author.tag);
            await enqueueAndPlay(message, statusMsg, tracks, voiceChannel);
            return;
        }
        
        const candidates = await extractTrackInfo(query, message.author.tag);
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
        await statusMsg.edit(`❌ ${e.message}`).catch(() => {});
    }
}

/**
 * 대기열에 추가하고 재생 시작 (공통 로직)
 */
async function enqueueAndPlay(message, statusMsg, tracks, voiceChannel) {
    const queue = getOrCreateQueue(message.guild.id);
    
    // 음성 채널 연결 (미연결 시)
    if (!queue.connection || queue.connection.state.status === voice.VoiceConnectionStatus.Destroyed) {
        queue.connection = voice.joinVoiceChannel({
            channelId: voiceChannel.id,
            guildId: message.guild.id,
            adapterCreator: message.guild.voiceAdapterCreator,
            selfDeaf: true,
        });

        // 연결 끊김 처리
        queue.connection.on(voice.VoiceConnectionStatus.Disconnected, async () => {
            try {
                await Promise.race([
                    voice.entersState(queue.connection, voice.VoiceConnectionStatus.Signalling, 5_000),
                    voice.entersState(queue.connection, voice.VoiceConnectionStatus.Connecting, 5_000),
                ]);
            } catch (_) {
                destroyQueue(message.guild.id);
            }
        });
    }
    
    queue.tracks.push(...tracks);
    
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
    
    queue.player.stop(); // triggers AudioPlayerStatus.Idle → playNext
    
    await message.channel.send(i18n.t('bot:music.skipped_next', {
        title: nextTrack.title,
        duration: nextTrack.duration
    }));
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
            text += `${idx + 1}. **${track.title}** [${track.duration}] — ${track.requester}\n`;
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
    
    // 셔플로 tracks[0]이 바뀌므로 기존 프리페치 무효화 후 재시작
    cleanupPrefetch(queue);
    startPrefetch(message.guild.id);
    
    await message.channel.send(i18n.t('bot:music.shuffled', {
        count: queue.tracks.length
    }));
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

module.exports = {
    handleMusicMessage,
    handleMusicShortcut,
    handleVoiceStateUpdate,
    isMusicModule,
    hasActiveQueue,
    musicAvailable: () => musicAvailable,
    MUSIC_COMMAND_LIST,
    DEFAULT_MODULE_ALIASES,
    DEFAULT_COMMAND_ALIASES,
};
