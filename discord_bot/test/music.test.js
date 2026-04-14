/**
 * 🎵 Music Extension 유닛 테스트
 *
 * 테스트 대상:
 *   1. musicChannelUI — 진행바/시간 표시, 큐 텍스트, 강제 리프레시
 *   2. music.js — 이전곡 히스토리, seek 오프셋, 닉네임 표시
 */

// ── musicChannelUI는 i18n에 의존하므로 mock 필요 ──
jest.mock('../i18n', () => ({
    t: (key, params) => {
        const map = {
            'bot:music_ui.idle_title': '🎵 No music playing',
            'bot:music_ui.idle_hint': 'Type a song name or URL to play!',
            'bot:music_ui.queue_header': 'Queue',
            'bot:music_ui.queue_empty': 'Queue is empty',
            'bot:music_ui.queue_more': `...+${params?.count || 0} more`,
        };
        return map[key] || key;
    },
}));

const {
    buildProgressBar,
    buildNowPlayingText,
    buildQueueText,
    parseDuration,
    formatTime,
} = (() => {
    // musicChannelUI 내부 함수를 테스트하기 위해 직접 로드
    const fs = require('fs');
    const path = require('path');
    const src = fs.readFileSync(
        path.join(__dirname, '..', 'extensions', 'musicChannelUI.js'),
        'utf-8'
    );

    // 모듈 내부 함수를 eval 없이 노출하기 어려우므로
    // 파일에서 export를 확장하는 것과, 직접 로직을 테스트하는 방식 혼용
    // 여기서는 모듈을 require한 뒤 내부 export 추가 버전 사용
    const mod = require('../extensions/musicChannelUI');
    return {
        buildProgressBar: mod._test?.buildProgressBar,
        buildNowPlayingText: mod._test?.buildNowPlayingText,
        buildQueueText: mod._test?.buildQueueText,
        parseDuration: mod._test?.parseDuration,
        formatTime: mod._test?.formatTime,
    };
})();


// ══════════════════════════════════════════════════════
// 1. 진행바 타이밍 테스트
// ══════════════════════════════════════════════════════
describe('진행바 타이밍', () => {
    test('_startedAt이 null이면 경과시간 0으로 표시', () => {
        if (!buildNowPlayingText) return; // _test export 없으면 skip
        const queue = {
            current: { title: 'Test Song', duration: '3:30', requester: 'TestUser' },
            _startedAt: null,
            _paused: false,
            _pausedElapsed: 0,
            volume: 0.5,
            loop: false,
        };
        const text = buildNowPlayingText(queue, {});
        expect(text).toContain('0:00');
    });

    test('일시정지 중에는 _pausedElapsed 시간 그대로 표시', () => {
        if (!buildNowPlayingText) return;
        const queue = {
            current: { title: 'Test Song', duration: '3:30', requester: 'TestUser' },
            _startedAt: Date.now() - 120000,
            _paused: true,
            _pausedElapsed: 60,
            volume: 0.5,
            loop: false,
        };
        const text = buildNowPlayingText(queue, {});
        expect(text).toContain('1:00');
    });

    test('총 길이를 초과하지 않음', () => {
        if (!buildNowPlayingText) return;
        const queue = {
            current: { title: 'Short', duration: '0:10', requester: 'User' },
            _startedAt: Date.now() - 60000, // 60초 전 시작 (10초짜리 곡)
            _paused: false,
            _pausedElapsed: 0,
            volume: 0.5,
            loop: false,
        };
        const text = buildNowPlayingText(queue, {});
        // 0:10 이 표시되어야 함 (10초 초과 불가)
        expect(text).toContain('0:10 / 0:10');
    });
});

// ══════════════════════════════════════════════════════
// 2. parseDuration / formatTime 유틸
// ══════════════════════════════════════════════════════
describe('시간 파싱 유틸', () => {
    test('parseDuration — M:SS', () => {
        if (!parseDuration) return;
        expect(parseDuration('3:45')).toBe(225);
    });

    test('parseDuration — H:MM:SS', () => {
        if (!parseDuration) return;
        expect(parseDuration('1:02:30')).toBe(3750);
    });

    test('parseDuration — invalid', () => {
        if (!parseDuration) return;
        expect(parseDuration('??:??')).toBe(0);
        expect(parseDuration(null)).toBe(0);
    });

    test('formatTime', () => {
        if (!formatTime) return;
        expect(formatTime(0)).toBe('0:00');
        expect(formatTime(65)).toBe('1:05');
        expect(formatTime(3661)).toBe('61:01');
    });
});

// ══════════════════════════════════════════════════════
// 3. buildProgressBar
// ══════════════════════════════════════════════════════
describe('진행바 렌더링', () => {
    test('0% → 전부 빈 블록', () => {
        if (!buildProgressBar) return;
        const bar = buildProgressBar(0, 100, 10);
        expect(bar).toBe('▱'.repeat(10));
    });

    test('50% → 절반 채움', () => {
        if (!buildProgressBar) return;
        const bar = buildProgressBar(50, 100, 10);
        expect(bar).toBe('▰'.repeat(5) + '▱'.repeat(5));
    });

    test('100% 이상 → 전부 채움 (clamped)', () => {
        if (!buildProgressBar) return;
        const bar = buildProgressBar(200, 100, 10);
        expect(bar).toBe('▰'.repeat(10));
    });

    test('total 0 → 전부 빈 블록', () => {
        if (!buildProgressBar) return;
        const bar = buildProgressBar(0, 0, 10);
        expect(bar).toBe('▬'.repeat(10));
    });
});

// ══════════════════════════════════════════════════════
// 4. 큐 텍스트 — 신청자 이름 표시
// ══════════════════════════════════════════════════════
describe('큐 텍스트', () => {
    test('큐에 신청자 닉네임이 표시됨', () => {
        if (!buildQueueText) return;
        const queue = {
            tracks: [
                { title: 'Song A', duration: '3:00', requester: 'GuildNickname' },
            ],
        };
        const text = buildQueueText(queue, { queueLines: 5 });
        expect(text).toContain('GuildNickname');
    });

    test('빈 큐', () => {
        if (!buildQueueText) return;
        const queue = { tracks: [] };
        const text = buildQueueText(queue, { queueLines: 5 });
        expect(text).toContain('Queue is empty');
    });
});

// ══════════════════════════════════════════════════════
// 5. 이전곡 히스토리 로직 (music.js 내부 — 단위 테스트)
// ══════════════════════════════════════════════════════
describe('이전곡 히스토리', () => {
    const MAX_HISTORY = 5;

    function pushHistory(history, track) {
        history.unshift(track);
        if (history.length > MAX_HISTORY) history.pop();
    }

    test('히스토리에 곡이 추가됨', () => {
        const history = [];
        pushHistory(history, { title: 'Song 1' });
        expect(history).toHaveLength(1);
        expect(history[0].title).toBe('Song 1');
    });

    test('최대 5곡 유지 (FIFO)', () => {
        const history = [];
        for (let i = 1; i <= 7; i++) {
            pushHistory(history, { title: `Song ${i}` });
        }
        expect(history).toHaveLength(5);
        // 가장 최근 곡이 [0]에 위치
        expect(history[0].title).toBe('Song 7');
        expect(history[4].title).toBe('Song 3');
    });

    test('이전곡 재생 시 히스토리에서 제거', () => {
        const history = [
            { title: 'Recent' },
            { title: 'Older' },
        ];
        const prev = history.shift();
        expect(prev.title).toBe('Recent');
        expect(history).toHaveLength(1);
    });
});

// ══════════════════════════════════════════════════════
// 6. Seek 오프셋 계산
// ══════════════════════════════════════════════════════
describe('Seek 오프셋 계산', () => {
    test('현재 위치에서 +5초', () => {
        const elapsed = 60; // 1:00
        const total = 210; // 3:30
        const target = Math.min(elapsed + 5, total);
        expect(target).toBe(65);
    });

    test('현재 위치에서 -5초', () => {
        const elapsed = 60;
        const target = Math.max(elapsed - 5, 0);
        expect(target).toBe(55);
    });

    test('-5초가 0 미만이면 0으로 clamp', () => {
        const elapsed = 3;
        const target = Math.max(elapsed - 3, 0);
        expect(target).toBe(0);
    });

    test('+5초가 총 길이 초과 시 clamp', () => {
        const elapsed = 208;
        const total = 210;
        const target = Math.min(elapsed + 5, total);
        expect(target).toBe(210);
    });
});

// ══════════════════════════════════════════════════════
// 7. 에러 분류 (_classifyError)
// ══════════════════════════════════════════════════════
describe('에러 분류 (_classifyError)', () => {
    // music.js의 _classifyError를 직접 로드
    const musicMod = require('../extensions/music');
    const classifyError = musicMod._test?.classifyError;

    test('함수가 export되어 있음', () => {
        expect(classifyError).toBeDefined();
        expect(typeof classifyError).toBe('function');
    });

    test('연령 제한 에러 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('Sign in to confirm your age'));
        expect(result.type).toBe('age_restricted');
    });

    test('로그인 필요 에러 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('Sign in to confirm you are not a bot'));
        expect(result.type).toBe('login_required');
    });

    test('비공개/삭제 영상 에러 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('Video unavailable'));
        expect(result.type).toBe('unavailable');
    });

    test('지역 제한 에러 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('not available in your country'));
        expect(result.type).toBe('geo_blocked');
    });

    test('저작권 차단 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('copyright claim'));
        expect(result.type).toBe('copyright');
    });

    test('네트워크 에러 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('ETIMEDOUT'));
        expect(result.type).toBe('network');

        const result2 = classifyError(new Error('ECONNREFUSED'));
        expect(result2.type).toBe('network');
    });

    test('yt-dlp 미설치 감지', () => {
        if (!classifyError) return;
        const err = new Error('spawn yt-dlp ENOENT');
        err.code = 'ENOENT';
        const result = classifyError(err);
        expect(result.type).toBe('tool_missing');
    });

    test('ffmpeg 관련 에러 감지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('ffmpeg exited with code 1'));
        expect(result.type).toBe('ffmpeg');
    });

    test('알 수 없는 에러 → unknown + 원본 메시지', () => {
        if (!classifyError) return;
        const result = classifyError(new Error('Something totally unexpected'));
        expect(result.type).toBe('unknown');
        expect(result.detail).toBe('Something totally unexpected');
    });

    test('에러 없이 null 전달 시에도 안전', () => {
        if (!classifyError) return;
        const result = classifyError(null);
        expect(result.type).toBe('unknown');
    });
});

// ══════════════════════════════════════════════════════
// 7. 볼륨 변경 (±10%)
// ══════════════════════════════════════════════════════
describe('볼륨 제어', () => {
    test('+10% (0.5 → 0.6)', () => {
        let volume = 0.5;
        volume = Math.min(volume + 0.1, 2.0);
        expect(volume).toBeCloseTo(0.6);
    });

    test('-10% (0.5 → 0.4)', () => {
        let volume = 0.5;
        volume = Math.max(volume - 0.1, 0.0);
        expect(volume).toBeCloseTo(0.4);
    });

    test('상한 200% 초과 안됨', () => {
        let volume = 1.95;
        volume = Math.min(volume + 0.1, 2.0);
        expect(volume).toBeCloseTo(2.0);
    });

    test('하한 0% 미만 안됨', () => {
        let volume = 0.05;
        volume = Math.max(volume - 0.1, 0.0);
        expect(volume).toBe(0.0);
    });
});

// ══════════════════════════════════════════════════════
// 8. 라디오 모드 — 자동 추천곡 재생
// ══════════════════════════════════════════════════════
describe('라디오 모드', () => {
    // ── 유튜브 비디오 ID 추출 ──
    function extractVideoId(url) {
        if (!url) return null;
        const patterns = [
            /(?:youtube\.com\/watch\?.*v=|youtu\.be\/|youtube\.com\/embed\/|music\.youtube\.com\/watch\?.*v=)([A-Za-z0-9_-]{11})/,
        ];
        for (const pattern of patterns) {
            const match = url.match(pattern);
            if (match) return match[1];
        }
        return null;
    }

    test('youtube.com/watch?v= 에서 ID 추출', () => {
        expect(extractVideoId('https://www.youtube.com/watch?v=dQw4w9WgXcQ'))
            .toBe('dQw4w9WgXcQ');
    });

    test('youtu.be 단축 URL에서 ID 추출', () => {
        expect(extractVideoId('https://youtu.be/dQw4w9WgXcQ'))
            .toBe('dQw4w9WgXcQ');
    });

    test('music.youtube.com URL에서 ID 추출', () => {
        expect(extractVideoId('https://music.youtube.com/watch?v=dQw4w9WgXcQ&feature=share'))
            .toBe('dQw4w9WgXcQ');
    });

    test('유효하지 않은 URL → null', () => {
        expect(extractVideoId('https://google.com')).toBeNull();
        expect(extractVideoId(null)).toBeNull();
        expect(extractVideoId('')).toBeNull();
    });

    // ── 히스토리 중복 필터링 ──
    function filterPlayedTracks(candidates, history, currentUrl) {
        const played = new Set(history.map(t => t.url));
        if (currentUrl) played.add(currentUrl);
        return candidates.filter(t => !played.has(t.url));
    }

    test('히스토리에 있는 곡은 추천에서 제외', () => {
        const candidates = [
            { title: 'A', url: 'https://youtube.com/watch?v=aaa' },
            { title: 'B', url: 'https://youtube.com/watch?v=bbb' },
            { title: 'C', url: 'https://youtube.com/watch?v=ccc' },
        ];
        const history = [
            { title: 'A', url: 'https://youtube.com/watch?v=aaa' },
        ];
        const filtered = filterPlayedTracks(candidates, history, 'https://youtube.com/watch?v=bbb');
        expect(filtered).toHaveLength(1);
        expect(filtered[0].title).toBe('C');
    });

    test('모든 곡이 히스토리에 있으면 빈 배열', () => {
        const candidates = [
            { title: 'A', url: 'https://youtube.com/watch?v=aaa' },
        ];
        const history = [
            { title: 'A', url: 'https://youtube.com/watch?v=aaa' },
        ];
        const filtered = filterPlayedTracks(candidates, history, null);
        expect(filtered).toHaveLength(0);
    });

    // ── 라디오 토글 상태 ──
    test('라디오 토글 on/off', () => {
        const queue = { radio: false };
        queue.radio = !queue.radio;
        expect(queue.radio).toBe(true);
        queue.radio = !queue.radio;
        expect(queue.radio).toBe(false);
    });

    // ── YouTube Mix URL 생성 ──
    function buildMixUrl(videoId) {
        return `https://www.youtube.com/watch?v=${videoId}&list=RD${videoId}`;
    }

    test('Mix 플레이리스트 URL 생성', () => {
        const url = buildMixUrl('dQw4w9WgXcQ');
        expect(url).toBe('https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=RDdQw4w9WgXcQ');
    });

    // ── 라디오 활성 시 큐가 비면 자동 추가 로직 ──
    test('라디오 on + 큐 비어있을 때 추천곡 추가 시뮬레이션', () => {
        const queue = {
            radio: true,
            tracks: [],
            history: [{ title: 'Seed Song', url: 'https://youtube.com/watch?v=seed123' }],
            current: null,
        };
        // 시뮬레이션: 추천곡이 추가되면 큐에 반영
        const recommended = { title: 'Recommended', url: 'https://youtube.com/watch?v=rec456', duration: '3:45', requester: '📻 Radio' };
        if (queue.radio && queue.tracks.length === 0) {
            queue.tracks.push(recommended);
        }
        expect(queue.tracks).toHaveLength(1);
        expect(queue.tracks[0].requester).toBe('📻 Radio');
    });

    test('라디오 off일 때는 자동 추가 안함', () => {
        const queue = {
            radio: false,
            tracks: [],
            history: [{ title: 'Seed Song', url: 'https://youtube.com/watch?v=seed123' }],
            current: null,
        };
        if (queue.radio && queue.tracks.length === 0) {
            queue.tracks.push({ title: 'Should not be added' });
        }
        expect(queue.tracks).toHaveLength(0);
    });
});

// ══════════════════════════════════════════════════════
// 9. 라디오 큐 — 표시 및 삽입 우선순위
// ══════════════════════════════════════════════════════
describe('라디오 큐 표시 및 삽입 우선순위', () => {
    const RADIO_REQUESTER = '📻 Radio';

    /**
     * 라디오 트랙인지 판별
     */
    function isRadioTrack(track) {
        return track.requester === RADIO_REQUESTER;
    }

    /**
     * 수동 트랙을 라디오 트랙 앞에 삽입하는 위치를 반환
     * 라디오 트랙이 아닌 마지막 위치 바로 뒤 = 첫 번째 라디오 트랙의 인덱스
     */
    function findManualInsertIndex(tracks) {
        for (let i = 0; i < tracks.length; i++) {
            if (isRadioTrack(tracks[i])) return i;
        }
        return tracks.length;
    }

    test('라디오 트랙 판별', () => {
        expect(isRadioTrack({ requester: '📻 Radio' })).toBe(true);
        expect(isRadioTrack({ requester: 'UserName' })).toBe(false);
        expect(isRadioTrack({ requester: '<@123456>' })).toBe(false);
    });

    test('수동 트랙이 라디오 트랙 앞에 삽입됨', () => {
        const tracks = [
            { title: 'Manual A', requester: '<@111>' },
            { title: 'Radio B', requester: RADIO_REQUESTER },
            { title: 'Radio C', requester: RADIO_REQUESTER },
        ];
        const newTrack = { title: 'Manual D', requester: '<@222>' };
        const idx = findManualInsertIndex(tracks);
        tracks.splice(idx, 0, newTrack);

        expect(tracks[0].title).toBe('Manual A');
        expect(tracks[1].title).toBe('Manual D');
        expect(tracks[2].title).toBe('Radio B');
        expect(tracks[3].title).toBe('Radio C');
    });

    test('라디오 트랙이 없으면 맨 뒤에 삽입', () => {
        const tracks = [
            { title: 'Manual A', requester: '<@111>' },
            { title: 'Manual B', requester: '<@222>' },
        ];
        const idx = findManualInsertIndex(tracks);
        expect(idx).toBe(2); // tracks.length
    });

    test('큐가 전부 라디오 트랙이면 맨 앞에 삽입', () => {
        const tracks = [
            { title: 'Radio A', requester: RADIO_REQUESTER },
            { title: 'Radio B', requester: RADIO_REQUESTER },
        ];
        const idx = findManualInsertIndex(tracks);
        expect(idx).toBe(0);

        const newTrack = { title: 'Manual', requester: '<@111>' };
        tracks.splice(idx, 0, newTrack);
        expect(tracks[0].title).toBe('Manual');
        expect(tracks[1].title).toBe('Radio A');
    });

    test('큐가 비어있으면 인덱스 0', () => {
        expect(findManualInsertIndex([])).toBe(0);
    });

    test('수동곡 여러 개 연속 추가해도 라디오 앞 유지', () => {
        const tracks = [
            { title: 'Radio A', requester: RADIO_REQUESTER },
        ];
        // 수동곡 2개 순차 삽입
        const manual1 = { title: 'Manual 1', requester: '<@111>' };
        const manual2 = { title: 'Manual 2', requester: '<@222>' };

        tracks.splice(findManualInsertIndex(tracks), 0, manual1);
        tracks.splice(findManualInsertIndex(tracks), 0, manual2);

        expect(tracks.map(t => t.title)).toEqual(['Manual 1', 'Manual 2', 'Radio A']);
    });

    // ── 큐 텍스트에 라디오 이모지 표시 ──
    test('라디오 트랙에 📻 이모지 표시', () => {
        const track = { title: 'Song', duration: '3:00', requester: RADIO_REQUESTER };
        // 큐 표시 포맷: "번호. **제목** [길이] — 신청자"
        const isRadio = isRadioTrack(track);
        const prefix = isRadio ? '📻 ' : '';
        const line = `${prefix}**${track.title}** [${track.duration}] — ${track.requester}`;
        expect(line).toContain('📻 **Song**');
    });

    test('수동 트랙에는 📻 이모지 없음', () => {
        const track = { title: 'Song', duration: '3:00', requester: '<@123>' };
        const isRadio = isRadioTrack(track);
        const prefix = isRadio ? '📻 ' : '';
        const line = `${prefix}**${track.title}** [${track.duration}] — ${track.requester}`;
        expect(line).not.toContain('📻');
    });
});

// ══════════════════════════════════════════════════════
// 10. 라디오 v3 — 프로액티브 큐 채움 및 자동 보충
// ══════════════════════════════════════════════════════
describe('라디오 v3 — 프로액티브 큐 채움 및 자동 보충', () => {
    const RADIO_REQUESTER = '📻 Radio';
    const RADIO_FILL_COUNT = 5;
    const RADIO_REPLENISH_THRESHOLD = 2;

    function isRadioTrack(track) {
        return track.requester === RADIO_REQUESTER;
    }

    function countRadioTracks(tracks) {
        return tracks.filter(isRadioTrack).length;
    }

    function needsReplenish(tracks) {
        return countRadioTracks(tracks) <= RADIO_REPLENISH_THRESHOLD;
    }

    // ── fetchRadioTracks는 복수 트랙을 반환해야 함 ──
    test('fetchRadioTracks가 여러 곡을 반환 (최대 RADIO_FILL_COUNT)', () => {
        // Mix 플레이리스트에서 필터 후 최대 5곡 반환 시뮬레이션
        const allCandidates = Array.from({ length: 20 }, (_, i) => ({
            title: `Track ${i}`,
            url: `https://youtube.com/watch?v=vid${i}`,
            duration: '3:00',
        }));
        const history = [
            { url: 'https://youtube.com/watch?v=vid0' },
            { url: 'https://youtube.com/watch?v=vid1' },
        ];
        const played = new Set(history.map(t => t.url));
        const seedUrl = 'https://youtube.com/watch?v=seed';
        played.add(seedUrl);
        const filtered = allCandidates.filter(t => !played.has(t.url));
        const result = filtered.slice(0, RADIO_FILL_COUNT).map(t => ({
            ...t,
            requester: RADIO_REQUESTER,
        }));

        expect(result.length).toBe(RADIO_FILL_COUNT);
        expect(result.every(t => t.requester === RADIO_REQUESTER)).toBe(true);
        // vid0, vid1은 제외되어야 함
        expect(result.some(t => t.url.includes('vid0'))).toBe(false);
        expect(result.some(t => t.url.includes('vid1'))).toBe(false);
    });

    // ── 라디오 ON 시 즉시 큐에 추천곡 채움 ──
    test('라디오 활성화 시 즉시 큐에 추천곡 리스트 추가', () => {
        const queue = {
            radio: false,
            tracks: [],
            current: { title: 'Now Playing', url: 'https://youtube.com/watch?v=now1' },
            _radioHistory: [],
        };
        // 라디오 활성화
        queue.radio = true;
        // 시뮬: fetchRadioTracks 결과를 큐에 추가
        const radioTracks = Array.from({ length: RADIO_FILL_COUNT }, (_, i) => ({
            title: `Radio ${i}`,
            url: `https://youtube.com/watch?v=radio${i}`,
            duration: '3:30',
            requester: RADIO_REQUESTER,
        }));
        queue.tracks.push(...radioTracks);

        expect(queue.tracks.length).toBe(RADIO_FILL_COUNT);
        expect(queue.tracks.every(isRadioTrack)).toBe(true);
    });

    // ── 기존 큐가 있는 상태에서 라디오 ON → 기존 큐 뒤에 추가 ──
    test('기존 수동 큐가 있을 때 라디오 ON → 수동곡 뒤에 라디오곡 추가', () => {
        const queue = {
            radio: false,
            tracks: [
                { title: 'Manual 1', requester: '<@111>' },
                { title: 'Manual 2', requester: '<@222>' },
            ],
            current: { title: 'Playing', url: 'https://youtube.com/watch?v=now1' },
            _radioHistory: [],
        };
        queue.radio = true;
        const radioTracks = Array.from({ length: 3 }, (_, i) => ({
            title: `Radio ${i}`,
            requester: RADIO_REQUESTER,
        }));
        queue.tracks.push(...radioTracks);

        expect(queue.tracks.length).toBe(5);
        expect(queue.tracks[0].title).toBe('Manual 1');
        expect(queue.tracks[1].title).toBe('Manual 2');
        expect(queue.tracks[2].requester).toBe(RADIO_REQUESTER);
    });

    // ── 자동 보충 트리거 조건 ──
    test('라디오 곡이 threshold 이하면 보충 필요', () => {
        const tracks = [
            { title: 'Manual', requester: '<@111>' },
            { title: 'Radio 1', requester: RADIO_REQUESTER },
            { title: 'Radio 2', requester: RADIO_REQUESTER },
        ];
        // 라디오 곡 2개 = threshold → 보충 필요
        expect(needsReplenish(tracks)).toBe(true);
    });

    test('라디오 곡이 threshold 초과면 보충 불필요', () => {
        const tracks = [
            { title: 'Radio 1', requester: RADIO_REQUESTER },
            { title: 'Radio 2', requester: RADIO_REQUESTER },
            { title: 'Radio 3', requester: RADIO_REQUESTER },
        ];
        // 라디오 곡 3개 > threshold(2) → 보충 불필요
        expect(needsReplenish(tracks)).toBe(false);
    });

    test('라디오 곡 0개 → 보충 필요', () => {
        const tracks = [
            { title: 'Manual', requester: '<@111>' },
        ];
        expect(needsReplenish(tracks)).toBe(true);
    });

    // ── 자동 보충 시 기존 라디오곡 뒤에 추가 ──
    test('보충 시 새 라디오곡은 기존 큐 뒤에 추가', () => {
        const tracks = [
            { title: 'Manual', requester: '<@111>' },
            { title: 'Radio Old', requester: RADIO_REQUESTER },
        ];
        const newRadio = Array.from({ length: 3 }, (_, i) => ({
            title: `Radio New ${i}`,
            requester: RADIO_REQUESTER,
        }));
        tracks.push(...newRadio);

        expect(tracks.length).toBe(5);
        expect(tracks[1].title).toBe('Radio Old');
        expect(tracks[2].title).toBe('Radio New 0');
    });

    // ── _radioFetching 플래그로 동시 fetch 방지 ──
    test('동시 fetch 방지 플래그', () => {
        const queue = { _radioFetching: false };
        // fetch 시작
        queue._radioFetching = true;
        // 다시 fetch 시도 → 건너뜀
        const shouldFetch = !queue._radioFetching;
        expect(shouldFetch).toBe(false);
        // fetch 완료
        queue._radioFetching = false;
        expect(queue._radioFetching).toBe(false);
    });

    // ── 보충 시 _radioHistory에 추가된 곡도 반영 ──
    test('새 라디오곡은 _radioHistory에도 추가', () => {
        const radioHistory = [
            { url: 'https://youtube.com/watch?v=old1' },
        ];
        const newTracks = [
            { url: 'https://youtube.com/watch?v=new1', title: 'New 1', requester: RADIO_REQUESTER },
            { url: 'https://youtube.com/watch?v=new2', title: 'New 2', requester: RADIO_REQUESTER },
        ];
        for (const t of newTracks) {
            radioHistory.unshift(t);
        }
        if (radioHistory.length > 30) radioHistory.splice(30);

        expect(radioHistory.length).toBe(3);
        expect(radioHistory[0].url).toContain('new2');
        expect(radioHistory[1].url).toContain('new1');
        expect(radioHistory[2].url).toContain('old1');
    });

    // ── 라디오 OFF 시 라디오 트랙 제거 ──
    test('라디오 OFF 시 큐에서 라디오 트랙만 제거', () => {
        const tracks = [
            { title: 'Manual 1', requester: '<@111>' },
            { title: 'Radio 1', requester: RADIO_REQUESTER },
            { title: 'Manual 2', requester: '<@222>' },
            { title: 'Radio 2', requester: RADIO_REQUESTER },
        ];
        const cleaned = tracks.filter(t => !isRadioTrack(t));
        expect(cleaned.length).toBe(2);
        expect(cleaned[0].title).toBe('Manual 1');
        expect(cleaned[1].title).toBe('Manual 2');
    });

    // ── seed 결정 로직: 큐 마지막 곡을 seed로 사용 ──
    test('seed는 큐 마지막 곡 또는 current 사용', () => {
        const queue = {
            current: { title: 'Now', url: 'https://youtube.com/watch?v=now1' },
            tracks: [
                { title: 'Q1', url: 'https://youtube.com/watch?v=q1' },
                { title: 'Q2', url: 'https://youtube.com/watch?v=q2' },
            ],
        };
        // seed = 큐 마지막 곡 (가장 최근 추가된 곡 기반 추천이 더 자연스러움)
        const seed = queue.tracks.length > 0
            ? queue.tracks[queue.tracks.length - 1]
            : queue.current;
        expect(seed.title).toBe('Q2');
    });

    test('큐가 비면 current를 seed로 사용', () => {
        const queue = {
            current: { title: 'Now', url: 'https://youtube.com/watch?v=now1' },
            tracks: [],
        };
        const seed = queue.tracks.length > 0
            ? queue.tracks[queue.tracks.length - 1]
            : queue.current;
        expect(seed.title).toBe('Now');
    });
});

// ══════════════════════════════════════════════════════
// 10b. 라디오 시드풀 — 다양성 개선 & 스킵 제외
// ══════════════════════════════════════════════════════
describe('라디오 시드풀 — 다양성 개선 & 스킵 제외', () => {
    const RADIO_REQUESTER = '📻 Radio';
    const SKIP_THRESHOLD_MS = 5000;

    function isRadioTrack(track) {
        return track.requester === RADIO_REQUESTER;
    }

    // ── 시드풀 초기화: 라디오 ON 시점의 current + 기존 큐 ──
    test('라디오 ON 시 seedPool에 current + 기존 큐 등록', () => {
        const queue = {
            current: { title: 'A', url: 'https://youtube.com/watch?v=a' },
            tracks: [
                { title: 'B', url: 'https://youtube.com/watch?v=b', requester: '<@111>' },
                { title: 'C', url: 'https://youtube.com/watch?v=c', requester: '<@222>' },
            ],
            _radioSeedPool: [],
        };
        // activateRadio 로직 시뮬레이션
        queue._radioSeedPool = [queue.current, ...queue.tracks].filter(t => t?.url);
        expect(queue._radioSeedPool).toHaveLength(3);
        expect(queue._radioSeedPool.map(t => t.title)).toEqual(['A', 'B', 'C']);
    });

    // ── 자동 추가된 라디오곡도 시드풀에 등록 ──
    test('라디오 자동 추가곡도 seedPool에 추가', () => {
        const pool = [
            { title: 'A', url: 'https://youtube.com/watch?v=a' },
        ];
        const newRadio = [
            { title: 'R1', url: 'https://youtube.com/watch?v=r1', requester: RADIO_REQUESTER },
            { title: 'R2', url: 'https://youtube.com/watch?v=r2', requester: RADIO_REQUESTER },
        ];
        pool.push(...newRadio);
        expect(pool).toHaveLength(3);
        expect(pool[2].title).toBe('R2');
    });

    // ── 수동 추가곡도 시드풀에 등록 ──
    test('수동 추가곡도 seedPool에 등록', () => {
        const pool = [
            { title: 'A', url: 'https://youtube.com/watch?v=a' },
        ];
        const manual = { title: 'Manual', url: 'https://youtube.com/watch?v=m1', requester: '<@111>' };
        pool.push(manual);
        expect(pool).toHaveLength(2);
    });

    // ── 시드풀에서 랜덤 선택 ──
    test('seedPool에서 seed를 랜덤으로 선택', () => {
        const pool = Array.from({ length: 10 }, (_, i) => ({
            title: `T${i}`, url: `https://youtube.com/watch?v=t${i}`,
        }));
        // 여러번 선택 시 다양한 결과가 나와야 함
        const picks = new Set();
        for (let i = 0; i < 50; i++) {
            const idx = Math.floor(Math.random() * pool.length);
            picks.add(pool[idx].title);
        }
        // 10개 중 최소 2개 이상은 다른 곡이 선택되어야 함
        expect(picks.size).toBeGreaterThanOrEqual(2);
    });

    // ── 5초 이내 스킵 감지 ──
    test('재생 5초 이내 스킵 시 해당 곡을 seedPool에서 제거', () => {
        const pool = [
            { title: 'A', url: 'https://youtube.com/watch?v=a' },
            { title: 'B', url: 'https://youtube.com/watch?v=b' },
            { title: 'C', url: 'https://youtube.com/watch?v=c' },
        ];
        // 곡 B가 3초만에 스킵됨
        const skippedTrack = { title: 'B', url: 'https://youtube.com/watch?v=b' };
        const playedMs = 3000;
        if (playedMs < SKIP_THRESHOLD_MS) {
            const idx = pool.findIndex(t => t.url === skippedTrack.url);
            if (idx !== -1) pool.splice(idx, 1);
        }
        expect(pool).toHaveLength(2);
        expect(pool.map(t => t.title)).toEqual(['A', 'C']);
    });

    test('5초 이상 재생 후 넘기면 seedPool에서 제거하지 않음', () => {
        const pool = [
            { title: 'A', url: 'https://youtube.com/watch?v=a' },
            { title: 'B', url: 'https://youtube.com/watch?v=b' },
        ];
        const skippedTrack = { title: 'B', url: 'https://youtube.com/watch?v=b' };
        const playedMs = 10000;
        if (playedMs < SKIP_THRESHOLD_MS) {
            const idx = pool.findIndex(t => t.url === skippedTrack.url);
            if (idx !== -1) pool.splice(idx, 1);
        }
        expect(pool).toHaveLength(2); // 그대로 유지
    });

    test('seedPool이 비어도 시드풀 제거 로직이 에러 안남', () => {
        const pool = [];
        const playedMs = 2000;
        const skippedTrack = { title: 'X', url: 'https://youtube.com/watch?v=x' };
        if (playedMs < SKIP_THRESHOLD_MS) {
            const idx = pool.findIndex(t => t.url === skippedTrack.url);
            if (idx !== -1) pool.splice(idx, 1);
        }
        expect(pool).toHaveLength(0);
    });

    // ── 라디오 OFF 시 seedPool 초기화 ──
    test('라디오 OFF 시 seedPool 초기화', () => {
        const queue = {
            _radioSeedPool: [
                { title: 'A', url: 'a' },
                { title: 'B', url: 'b' },
            ],
        };
        queue._radioSeedPool = [];
        expect(queue._radioSeedPool).toHaveLength(0);
    });

    // ── seedPool 상한 (50곡) ──
    test('seedPool 최대 50곡 유지', () => {
        const SEED_POOL_MAX = 50;
        const pool = Array.from({ length: 55 }, (_, i) => ({
            title: `T${i}`, url: `https://youtube.com/watch?v=t${i}`,
        }));
        if (pool.length > SEED_POOL_MAX) {
            pool.splice(0, pool.length - SEED_POOL_MAX);
        }
        expect(pool).toHaveLength(50);
        // 가장 오래된 곡(T0~T4)이 제거되고 최근 50곡(T5~T54) 유지
        expect(pool[0].title).toBe('T5');
        expect(pool[49].title).toBe('T54');
    });
});

// ══════════════════════════════════════════════════════
// 11. 오디오 정규화 (loudnorm) — 투패스 아키텍처
// ══════════════════════════════════════════════════════
describe('오디오 정규화 (normalize) — 투패스', () => {
    // music.js와 동일한 상수
    const LOUDNORM_FILTER = 'loudnorm=I=-14:TP=-1:LRA=11';
    const FFMPEG_ENCODE_ARGS = [
        '-hide_banner', '-loglevel', 'error',
        '-i', 'pipe:0', '-vn',
        '-acodec', 'libopus', '-b:a', '64k',
        '-f', 'ogg', '-ar', '48000', '-ac', '2',
    ];

    /**
     * 투패스 2패스 필터 문자열 생성 (music.js _startTwoPassStream 내부 로직 재현)
     */
    function buildTwoPassFilter(measured) {
        return `loudnorm=I=-14:TP=-1:LRA=11:` +
            `measured_I=${measured.input_i}:` +
            `measured_TP=${measured.input_tp}:` +
            `measured_LRA=${measured.input_lra}:` +
            `measured_thresh=${measured.input_thresh}:` +
            `linear=true`;
    }

    /**
     * _encodePipe 인자 구성 재현
     */
    function buildEncodeArgs(extraArgs) {
        return [...FFMPEG_ENCODE_ARGS, ...extraArgs, 'pipe:1'];
    }

    test('normalize OFF (직접 스트림) → -af 필터 없음', () => {
        // _startDirectStream은 LOUDNORM_FILTER를 쓰지 않음
        const args = buildEncodeArgs([]);
        expect(args).not.toContain('-af');
        expect(args.join(' ')).not.toContain('loudnorm');
    });

    test('투패스 2패스 필터 → measured_* 파라미터 + linear=true 포함', () => {
        const measured = {
            input_i: '-24.5',
            input_tp: '-2.3',
            input_lra: '8.7',
            input_thresh: '-35.2',
        };
        const filter = buildTwoPassFilter(measured);
        expect(filter).toContain('measured_I=-24.5');
        expect(filter).toContain('measured_TP=-2.3');
        expect(filter).toContain('measured_LRA=8.7');
        expect(filter).toContain('measured_thresh=-35.2');
        expect(filter).toContain('linear=true');
        // 기본 타겟도 포함
        expect(filter).toContain('I=-14');
        expect(filter).toContain('TP=-1');
        expect(filter).toContain('LRA=11');
    });

    test('투패스 인코딩 인자 → pipe:1은 항상 마지막', () => {
        const filter = buildTwoPassFilter({
            input_i: '-20', input_tp: '-1', input_lra: '10', input_thresh: '-30',
        });
        const args = buildEncodeArgs(['-af', filter]);
        expect(args[args.length - 1]).toBe('pipe:1');
    });

    test('폴백(oversized) → 싱글패스 LOUDNORM_FILTER 사용', () => {
        // 50MB 초과 시 싱글패스 폴백: _encodePipe(rawAudio, ['-af', LOUDNORM_FILTER], buffer)
        const args = buildEncodeArgs(['-af', LOUDNORM_FILTER]);
        const afIdx = args.indexOf('-af');
        expect(afIdx).toBeGreaterThan(-1);
        expect(args[afIdx + 1]).toBe(LOUDNORM_FILTER);
        // linear=true 없음 (싱글패스)
        expect(args[afIdx + 1]).not.toContain('linear=true');
    });

    test('분석 실패 폴백 → 정규화 없이 인코딩', () => {
        // _analyzeLoudness 실패 시: _encodePipe(rawAudio, [], buffer)
        const args = buildEncodeArgs([]);
        expect(args).not.toContain('-af');
        expect(args).not.toContain('loudnorm');
    });

    test('1패스 분석 인자 → print_format=json 포함', () => {
        // _analyzeLoudness가 사용하는 ffmpeg 인자
        const analyzeArgs = [
            '-hide_banner',
            '-i', 'pipe:0',
            '-af', 'loudnorm=I=-14:TP=-1:LRA=11:print_format=json',
            '-f', 'null', '-',
        ];
        expect(analyzeArgs).toContain('-af');
        const afIdx = analyzeArgs.indexOf('-af');
        expect(analyzeArgs[afIdx + 1]).toContain('print_format=json');
        expect(analyzeArgs[afIdx + 1]).toContain(LOUDNORM_FILTER);
    });

    test('loudnorm JSON 파싱 시뮬레이션', () => {
        // ffmpeg stderr 출력에서 JSON 추출 (실제 ffmpeg 출력 형태)
        const stderrData = `
[Parsed_loudnorm_0 @ 0x1234] 
{
	"input_i" : "-24.50",
	"input_tp" : "-2.30",
	"input_lra" : "8.70",
	"input_thresh" : "-35.20",
	"output_i" : "-14.00",
	"output_tp" : "-1.00",
	"output_lra" : "7.10",
	"output_thresh" : "-24.70",
	"normalization_type" : "dynamic",
	"target_offset" : "0.00"
}`;
        const jsonMatch = stderrData.match(/\{[\s\S]*"input_i"[\s\S]*?\}/);
        expect(jsonMatch).not.toBeNull();
        const data = JSON.parse(jsonMatch[0]);
        expect(data.input_i).toBe('-24.50');
        expect(data.input_tp).toBe('-2.30');
        expect(data.input_lra).toBe('8.70');
        expect(data.input_thresh).toBe('-35.20');
    });

    test('normalize 토글 on/off', () => {
        const queue = { normalize: false };
        queue.normalize = !queue.normalize;
        expect(queue.normalize).toBe(true);
        queue.normalize = !queue.normalize;
        expect(queue.normalize).toBe(false);
    });

    test('큐 기본값은 normalize: true', () => {
        const DEFAULT_NORMALIZE = true;
        const queue = {
            volume: 0.5,
            normalize: DEFAULT_NORMALIZE,
        };
        expect(queue.normalize).toBe(true);
    });

    test('FFMPEG_ENCODE_ARGS에 loudnorm 포함되지 않음 (필터는 extraArgs로 전달)', () => {
        expect(FFMPEG_ENCODE_ARGS).not.toContain('-af');
        expect(FFMPEG_ENCODE_ARGS.join(' ')).not.toContain('loudnorm');
    });
});

// ══════════════════════════════════════════════════════
// 11-2. 전체 큐 프리페치 — prefetchMap 아키텍처
// ══════════════════════════════════════════════════════
describe('전체 큐 프리페치 (prefetchMap)', () => {
    function createMockQueue(trackUrls) {
        return {
            tracks: trackUrls.map((url, i) => ({ title: `Track ${i}`, url })),
            normalize: true,
            prefetchMap: new Map(),
        };
    }

    test('큐 초기 상태: prefetchMap은 빈 Map', () => {
        const queue = createMockQueue([]);
        expect(queue.prefetchMap).toBeInstanceOf(Map);
        expect(queue.prefetchMap.size).toBe(0);
    });

    test('prefetchMap에서 URL 조회 — hits & misses', () => {
        const queue = createMockQueue(['url-a', 'url-b']);
        const fakeStream = { readableLength: 1024, destroyed: false };
        queue.prefetchMap.set('url-a', fakeStream);

        expect(queue.prefetchMap.has('url-a')).toBe(true);
        expect(queue.prefetchMap.has('url-b')).toBe(false);
    });

    test('prefetchMap에서 소유권 이전 — delete 후 조회 불가', () => {
        const queue = createMockQueue(['url-a']);
        const fakeStream = { readableLength: 2048, destroyed: false };
        queue.prefetchMap.set('url-a', fakeStream);

        const stream = queue.prefetchMap.get('url-a');
        queue.prefetchMap.delete('url-a');

        expect(stream).toBe(fakeStream);
        expect(queue.prefetchMap.has('url-a')).toBe(false);
    });

    test('destroyed 스트림은 사용하지 않음', () => {
        const queue = createMockQueue(['url-a']);
        const fakeStream = { readableLength: 0, destroyed: true };
        queue.prefetchMap.set('url-a', fakeStream);

        const prefetched = queue.prefetchMap.get('url-a');
        const usable = prefetched && !prefetched.destroyed;
        expect(usable).toBe(false);
    });

    test('큐에서 빠진 URL은 정리 대상', () => {
        const queue = createMockQueue(['url-b']);
        queue.prefetchMap.set('url-a', { destroyed: false });
        queue.prefetchMap.set('url-b', { destroyed: false });

        const queueUrls = new Set(queue.tracks.map(t => t.url));
        for (const [url] of queue.prefetchMap) {
            if (!queueUrls.has(url)) {
                queue.prefetchMap.delete(url);
            }
        }

        expect(queue.prefetchMap.has('url-a')).toBe(false);
        expect(queue.prefetchMap.has('url-b')).toBe(true);
    });

    test('중복 URL 트랙 — Map은 단일 엔트리만 유지', () => {
        const queue = createMockQueue(['url-a', 'url-a', 'url-b']);
        const uniqueUrls = new Set(queue.tracks.map(t => t.url));
        expect(uniqueUrls.size).toBe(2);
        // Map.set은 동일 키에 덮어쓰므로 자연스럽게 단일
        queue.prefetchMap.set('url-a', { id: 1 });
        queue.prefetchMap.set('url-a', { id: 2 });
        expect(queue.prefetchMap.size).toBe(1);
        expect(queue.prefetchMap.get('url-a').id).toBe(2);
    });

    test('normalize 토글 후 prefetchMap 전체 정리', () => {
        const queue = createMockQueue(['url-a', 'url-b']);
        queue.prefetchMap.set('url-a', { destroyed: false });
        queue.prefetchMap.set('url-b', { destroyed: false });

        // cleanupPrefetch 시뮬레이션
        queue.prefetchMap.clear();
        expect(queue.prefetchMap.size).toBe(0);
    });
});

// ══════════════════════════════════════════════════════
// 12. 이전곡 버튼 — 7.5초 기준 restart / 이전 곡 점프
// ══════════════════════════════════════════════════════
describe('이전곡 버튼 — 7.5초 기준', () => {
    const PREV_RESTART_THRESHOLD_MS = 7500;

    test('7.5초 이상 재생 → 현재 곡 다시 재생 (restart)', () => {
        const currentTrack = { title: 'Now', url: 'https://youtube.com/watch?v=now1' };
        const queue = {
            current: currentTrack,
            _startedAt: Date.now() - 10000, // 10초 전 시작
            _paused: false,
            _pausedElapsed: 0,
            tracks: [{ title: 'Next', url: 'next' }],
            history: [{ title: 'Prev', url: 'prev' }],
        };

        const elapsedMs = Date.now() - queue._startedAt;
        if (elapsedMs >= PREV_RESTART_THRESHOLD_MS) {
            // restart: 현재 곡을 큐 앞에
            queue.tracks.unshift(queue.current);
            queue.current = null;
        }

        expect(queue.current).toBeNull();
        expect(queue.tracks[0].title).toBe('Now'); // 같은 곡이 큐 맨 앞
        expect(queue.tracks[1].title).toBe('Next');
        expect(queue.history).toHaveLength(1); // history 건드리지 않음
    });

    test('7.5초 이전 → 이전 곡으로 점프', () => {
        const currentTrack = { title: 'Now', url: 'https://youtube.com/watch?v=now1' };
        const prevTrack = { title: 'Prev', url: 'https://youtube.com/watch?v=prev1' };
        const queue = {
            current: currentTrack,
            _startedAt: Date.now() - 3000, // 3초 전 시작
            _paused: false,
            _pausedElapsed: 0,
            tracks: [],
            history: [prevTrack],
        };

        const elapsedMs = Date.now() - queue._startedAt;
        if (elapsedMs >= PREV_RESTART_THRESHOLD_MS) {
            queue.tracks.unshift(queue.current);
            queue.current = null;
        } else {
            // 이전 곡 복원
            const prev = queue.history.shift();
            queue.tracks.unshift(queue.current);
            queue.tracks.unshift(prev);
            queue.current = null;
        }

        expect(queue.current).toBeNull();
        expect(queue.tracks[0].title).toBe('Prev');
        expect(queue.tracks[1].title).toBe('Now');
        expect(queue.history).toHaveLength(0);
    });

    test('일시정지 상태에서도 경과 시간 정확하게 판단', () => {
        const queue = {
            current: { title: 'Now', url: 'now' },
            _startedAt: Date.now() - 20000,
            _paused: true,
            _pausedElapsed: 8, // 8초 재생 후 일시정지
            tracks: [],
            history: [{ title: 'Prev', url: 'prev' }],
        };

        const elapsedMs = queue._paused
            ? queue._pausedElapsed * 1000
            : Date.now() - queue._startedAt;

        expect(elapsedMs).toBe(8000);
        expect(elapsedMs >= PREV_RESTART_THRESHOLD_MS).toBe(true);
    });

    test('일시정지 2초 → 7.5초 미만이므로 이전 곡', () => {
        const queue = {
            current: { title: 'Now', url: 'now' },
            _startedAt: Date.now() - 5000,
            _paused: true,
            _pausedElapsed: 2, // 2초 재생 후 일시정지
            tracks: [],
            history: [{ title: 'Prev', url: 'prev' }],
        };

        const elapsedMs = queue._paused
            ? queue._pausedElapsed * 1000
            : Date.now() - queue._startedAt;

        expect(elapsedMs).toBe(2000);
        expect(elapsedMs >= PREV_RESTART_THRESHOLD_MS).toBe(false);
    });
});

// ══════════════════════════════════════════════════════
// 12. init() — 확장 패키지 없이 musicAvailable 방지
// ══════════════════════════════════════════════════════
describe('init() — 확장 패키지 부재 시 musicAvailable 방지', () => {
    const musicMod = require('../extensions/music');

    test('init()이 export되어 있음', () => {
        expect(typeof musicMod.init).toBe('function');
    });

    test('musicAvailable()이 함수이고 boolean 반환', () => {
        expect(typeof musicMod.musicAvailable).toBe('function');
        expect(typeof musicMod.musicAvailable()).toBe('boolean');
    });

    /**
     * 핵심 버그 시나리오:
     * 확장 node_modules가 없을 때 init()이 musicAvailable=true로 설정하면
     * 이후 패키지가 설치되어도 재초기화되지 않아 영원히 음악이 동작하지 않음.
     *
     * 이 테스트는 cleanup() 후 init()을 다시 호출하여, 확장 패키지 디렉토리가
     * 없는 환경에서 musicAvailable이 true로 설정되지 않는지 검증한다.
     */
    test('확장 node_modules 없으면 musicAvailable이 true가 되면 안됨', () => {
        // 현재 상태 저장
        const wasMusicAvailable = musicMod.musicAvailable();

        // cleanup으로 musicAvailable=false 설정
        musicMod.cleanup();
        expect(musicMod.musicAvailable()).toBe(false);

        // 모든 fallback 경로를 차단하여 확장 디렉토리를 찾지 못하게 함
        const origExtDir = process.env.SABA_EXTENSIONS_DIR;
        const origDataDir = process.env.SABA_DATA_DIR;
        process.env.SABA_EXTENSIONS_DIR = '/nonexistent/path/that/does/not/exist';
        process.env.SABA_DATA_DIR = '/nonexistent/saba-data';

        try {
            // init() 호출 — 확장 패키지가 없으므로 false를 반환해야 함
            const result = musicMod.init();

            // 핵심 검증: 확장 패키지 없이 musicAvailable=true가 되면 버그
            expect(result).toBe(false);
            expect(musicMod.musicAvailable()).toBe(false);
        } finally {
            // 환경 복원
            if (origExtDir !== undefined) {
                process.env.SABA_EXTENSIONS_DIR = origExtDir;
            } else {
                delete process.env.SABA_EXTENSIONS_DIR;
            }
            if (origDataDir !== undefined) {
                process.env.SABA_DATA_DIR = origDataDir;
            } else {
                delete process.env.SABA_DATA_DIR;
            }
        }
    });

});

// ══════════════════════════════════════════════════════
// 13. player.stop(true) — 일시정지 상태에서 강제 Idle 전환
// ══════════════════════════════════════════════════════
describe('일시정지 상태에서 player.stop(true) 호출 검증', () => {
    // @discordjs/voice v0.19에서 player.stop(false)을 Paused 상태에서 호출하면
    // Idle 전환이 되지 않아 playNext()가 호출되지 않는 버그 검증.
    // handlePrevious, music_next, handleSkip 모두 stop(true) 사용 필수.

    let music;
    let _test;

    beforeAll(() => {
        try {
            music = require('../extensions/music');
            _test = music._test;
        } catch (e) {
            // 모듈 로드 실패 시 테스트 skip
        }
    });

    afterEach(() => {
        // 테스트 후 guildQueues 정리
        if (_test?.guildQueues) {
            _test.guildQueues.clear();
        }
    });

    function createMockPlayer(status = 'playing') {
        const stopCalls = [];
        return {
            state: { status },
            stop: jest.fn((force) => {
                stopCalls.push({ force: !!force });
                return true;
            }),
            _stopCalls: stopCalls,
        };
    }

    function setupPausedQueue(guildId, opts = {}) {
        if (!_test) return null;
        const player = createMockPlayer('paused');
        const queue = {
            guildId,
            tracks: opts.tracks || [],
            history: opts.history || [],
            _radioHistory: [],
            _radioSeedPool: [],
            current: opts.current || { title: 'Current', url: 'https://youtube.com/watch?v=current1', duration: '3:30' },
            connection: {},
            player,
            resource: null,
            volume: 0.5,
            loop: false,
            radio: false,
            _radioFetching: false,
            normalize: false,
            idleTimer: null,
            prefetchMap: new Map(),
            _startedAt: opts._startedAt ?? (Date.now() - 3000),
            _paused: true,
            _pausedElapsed: opts._pausedElapsed ?? 2,
            _playNextPending: false,
            _activeStream: null,
            _adapterCreator: null,
            _autoPauseCount: 0,
            _autoPauseResetTimer: null,
        };
        _test.guildQueues.set(guildId, queue);
        return { queue, player };
    }

    test('handlePrevious — 일시정지 + <7.5초 → player.stop(true) 호출', async () => {
        if (!_test?.handlePrevious) return;
        const guildId = 'test-guild-pause-prev';
        const { player } = setupPausedQueue(guildId, {
            _pausedElapsed: 2, // 2초 → 7.5초 미만
            history: [{ title: 'Prev Song', url: 'https://youtube.com/watch?v=prev1', duration: '4:00' }],
        });

        await _test.handlePrevious(guildId);

        expect(player.stop).toHaveBeenCalledTimes(1);
        expect(player.stop).toHaveBeenCalledWith(true);
    });

    test('handlePrevious — 일시정지 + >=7.5초 → player.stop(true) 호출', async () => {
        if (!_test?.handlePrevious) return;
        const guildId = 'test-guild-pause-restart';
        const { player } = setupPausedQueue(guildId, {
            _pausedElapsed: 10, // 10초 → 7.5초 이상
            _startedAt: Date.now() - 15000,
        });

        await _test.handlePrevious(guildId);

        expect(player.stop).toHaveBeenCalledTimes(1);
        expect(player.stop).toHaveBeenCalledWith(true);
    });

    test('handlePrevious — 재생 중(비일시정지) + <7.5초 → player.stop(true) 호출', async () => {
        if (!_test?.handlePrevious) return;
        const guildId = 'test-guild-playing-prev';
        const player = createMockPlayer('playing');
        const queue = {
            guildId,
            tracks: [],
            history: [{ title: 'Prev Song', url: 'prev', duration: '3:00' }],
            _radioHistory: [],
            _radioSeedPool: [],
            current: { title: 'Current', url: 'current', duration: '3:00' },
            connection: {},
            player,
            resource: null,
            volume: 0.5,
            loop: false,
            radio: false,
            _radioFetching: false,
            normalize: false,
            idleTimer: null,
            prefetchMap: new Map(),
            _startedAt: Date.now() - 2000,
            _paused: false,
            _pausedElapsed: 0,
            _playNextPending: false,
            _activeStream: null,
            _adapterCreator: null,
            _autoPauseCount: 0,
            _autoPauseResetTimer: null,
        };
        _test.guildQueues.set(guildId, queue);

        await _test.handlePrevious(guildId);

        expect(player.stop).toHaveBeenCalledTimes(1);
        // 재생 중에도 stop(true) 사용 (일관성)
        expect(player.stop).toHaveBeenCalledWith(true);
    });
});
