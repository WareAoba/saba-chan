#!/usr/bin/env node
/**
 * 모의 릴레이 업데이트 서버 — 로컬에서 업데이터 확인/다운로드/적용 E2E 시험용
 *
 * 운영 릴레이의 업데이트 API와 같은 모양으로 응답한다 (updater/src/relay.rs):
 *   GET  /v1/updates/check?channel&platform&current&client_id
 *   GET  /v1/updates/manifest/:version   ← 서명된 manifest 봉투 {manifest, signature, keyId}
 *   GET  /download/:file                 ← R2 공개 URL 대신
 *
 * manifest는 공개된 고정 모의 키(keyId "saba-updater-mock")로 서명한다. 이 키는
 * mock-relay 기능으로 만든 개발용 데몬만 믿는다 (signature::MOCK_RELAY_KEY):
 *   pwsh scripts/build-core.ps1 -Debug -MockRelay   (또는 cargo build -p saba-core --features mock-relay)
 *
 * ## 사용법
 *   node scripts/mock-release-server.js [--port 9876] [--version 0.2.0] [--bin-dir target/release]
 *   그다음 업데이트 설정: source = "relay", relay_url = "http://127.0.0.1:9876"
 *
 * 실행 파일 컴포넌트(saba-core, cli, updater, gui)는 --bin-dir(기본 target/release)에 실제 빌드가 있으면
 * 그 파일을 담는다. 그래야 적용 뒤 데몬·GUI가 다시 떠서 "재시작"까지 시험할 수 있다.
 * 없으면 글자 파일을 담고 경고한다 (확인·다운로드·적용까지만 시험 가능).
 *
 * --zips <폴더>를 주면 그 폴더에 있는 실제 릴리즈 zip(package-linux.py 출력 등)을 그대로 싣는다:
 * 실행 권한·아이콘 같은 zip 안 구성까지 설치기·업데이터로 시험한다 (리눅스 데스크톱 설치 실측).
 * 설치기(`saba-chan-cli install`)는 mock-relay로 빌드하면 SABA_RELAY_URL을 따른다.
 *
 * 에셋은 scripts/.mock-assets/ 에 만들어진다 (git에 넣지 않는다).
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

// ─── CLI 인자 ───────────────────────────────────────
const args = process.argv.slice(2);
function getArg(name, defaultVal) {
    const idx = args.indexOf(name);
    return idx >= 0 && args[idx + 1] ? args[idx + 1] : defaultVal;
}

const PORT = parseInt(getArg('--port', '9876'), 10);
const MOCK_VERSION = getArg('--version', '0.2.0');
const ASSETS_DIR = path.join(__dirname, '.mock-assets');
const BIN_DIR = path.resolve(getArg('--bin-dir', path.join(__dirname, '..', 'target', 'release')));
const ZIPS_DIR = args.includes('--zips') ? path.resolve(getArg('--zips', '')) : null;
if (ZIPS_DIR && !getArg('--zips', '')) {
    console.error('--zips needs a folder');
    process.exit(2);
}

// ─── 서명 ───────────────────────────────────────────
// 공개된 시험용 seed. 운영 키가 아니다: 릴리즈 빌드는 이 키를 믿지 않는다
const MOCK_KEY_ID = 'saba-updater-mock';
const MOCK_SEED = Buffer.from('saba-mock-relay-not-for-release!');
const PRIVATE_KEY = crypto.createPrivateKey({
    key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), MOCK_SEED]),
    format: 'der',
    type: 'pkcs8',
});

/** updater signature::canonical_json 과 같은 정규형: 키 정렬, 공백 없음 */
function canonicalJson(value) {
    if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
    if (value && typeof value === 'object') {
        return `{${Object.keys(value).sort().map((k) => `${JSON.stringify(k)}:${canonicalJson(value[k])}`).join(',')}}`;
    }
    return JSON.stringify(value);
}

function signManifest(manifest) {
    const signature = crypto.sign(null, Buffer.from(canonicalJson(manifest)), PRIVATE_KEY).toString('base64');
    return { manifest, signature, keyId: MOCK_KEY_ID };
}

// ─── 가짜 ZIP 생성 ─────────────────────────────────
// 진짜 ZIP은 아니지만, 업데이터의 zip::ZipArchive가 파싱할 수 있어야 합니다.
// 최소한의 유효한 ZIP 파일을 생성합니다 (PK 매직넘버 + 단일 파일).

function createMiniZip(innerFileName, innerContent) {
    // ZIP local file header + central directory + end-of-central-directory
    const fnBuf = Buffer.from(innerFileName, 'utf-8');
    const dataBuf = Buffer.isBuffer(innerContent) ? innerContent : Buffer.from(innerContent, 'utf-8');
    const crc = crc32(dataBuf);

    // Local file header
    const lfh = Buffer.alloc(30 + fnBuf.length);
    lfh.writeUInt32LE(0x04034b50, 0);   // signature
    lfh.writeUInt16LE(20, 4);           // version needed
    lfh.writeUInt16LE(0, 6);            // flags
    lfh.writeUInt16LE(0, 8);            // compression: stored
    lfh.writeUInt16LE(0, 10);           // mod time
    lfh.writeUInt16LE(0, 12);           // mod date
    lfh.writeUInt32LE(crc, 14);         // crc32
    lfh.writeUInt32LE(dataBuf.length, 18); // compressed size
    lfh.writeUInt32LE(dataBuf.length, 22); // uncompressed size
    lfh.writeUInt16LE(fnBuf.length, 26);   // filename length
    lfh.writeUInt16LE(0, 28);              // extra field length
    fnBuf.copy(lfh, 30);

    const lfhSize = lfh.length;
    const dataOffset = lfhSize;

    // Central directory header
    const cdh = Buffer.alloc(46 + fnBuf.length);
    cdh.writeUInt32LE(0x02014b50, 0);   // signature
    cdh.writeUInt16LE(20, 4);           // version made by
    cdh.writeUInt16LE(20, 6);           // version needed
    cdh.writeUInt16LE(0, 8);            // flags
    cdh.writeUInt16LE(0, 10);           // compression
    cdh.writeUInt16LE(0, 12);           // mod time
    cdh.writeUInt16LE(0, 14);           // mod date
    cdh.writeUInt32LE(crc, 16);         // crc32
    cdh.writeUInt32LE(dataBuf.length, 20); // compressed size
    cdh.writeUInt32LE(dataBuf.length, 24); // uncompressed size
    cdh.writeUInt16LE(fnBuf.length, 28);   // filename length
    cdh.writeUInt16LE(0, 30);              // extra field length
    cdh.writeUInt16LE(0, 32);              // comment length
    cdh.writeUInt16LE(0, 34);              // disk start
    cdh.writeUInt16LE(0, 36);              // internal attrs
    cdh.writeUInt32LE(0, 38);              // external attrs
    cdh.writeUInt32LE(0, 42);              // local header offset
    fnBuf.copy(cdh, 46);

    const cdOffset = lfhSize + dataBuf.length;

    // End of central directory
    const eocd = Buffer.alloc(22);
    eocd.writeUInt32LE(0x06054b50, 0);    // signature
    eocd.writeUInt16LE(0, 4);              // disk number
    eocd.writeUInt16LE(0, 6);              // cd start disk
    eocd.writeUInt16LE(1, 8);              // entries on this disk
    eocd.writeUInt16LE(1, 10);             // total entries
    eocd.writeUInt32LE(cdh.length, 12);    // cd size
    eocd.writeUInt32LE(cdOffset, 16);      // cd offset
    eocd.writeUInt16LE(0, 20);             // comment length

    return Buffer.concat([lfh, dataBuf, cdh, eocd]);
}

// CRC32 (ZIP 형식에 필요)
function crc32(buf) {
    let crc = 0xFFFFFFFF;
    for (let i = 0; i < buf.length; i++) {
        crc ^= buf[i];
        for (let j = 0; j < 8; j++) {
            crc = (crc >>> 1) ^ ((crc & 1) ? 0xEDB88320 : 0);
        }
    }
    return (crc ^ 0xFFFFFFFF) >>> 0;
}

// ─── 에셋 ───────────────────────────────────────────

// manifest 컴포넌트 키 → (에셋 이름, 안에 든 파일, 설치 폴더, 실제 빌드 파일 이름)
function componentSpecs(platform) {
    const exe = platform.startsWith('windows') ? '.exe' : '';
    return {
        'saba-core': { asset: `saba-core-${platform}.zip`, inner: `saba-core${exe}`, install_dir: '.', bin: `saba-core${exe}` },
        cli: { asset: `saba-chan-cli-${platform}.zip`, inner: `saba-chan-cli${exe}`, install_dir: '.', bin: `saba-chan-cli${exe}` },
        updater: { asset: `saba-chan-updater-${platform}.zip`, inner: `saba-chan-updater${exe}`, install_dir: '.', bin: `saba-chan-updater${exe}` },
        gui: { asset: `saba-chan-gui-${platform}.zip`, inner: `saba-chan-gui${exe}`, install_dir: '.', bin: `saba-chan-gui${exe}` },
        'module-minecraft': { asset: 'module-minecraft.zip', inner: 'module.toml', install_dir: 'modules/minecraft' },
    };
}

/** 에셋 안에 담을 내용: 실제 빌드가 있으면 그 바이트, 없으면 글자 */
function assetContent(key, spec) {
    if (spec.inner === 'module.toml') {
        return `[module]\nname = "minecraft"\nversion = "${MOCK_VERSION}"\n`;
    }
    const built = spec.bin && path.join(BIN_DIR, spec.bin);
    if (built && fs.existsSync(built)) {
        return fs.readFileSync(built);
    }
    if (spec.bin) {
        console.warn(`  ! ${spec.bin} not found in ${BIN_DIR}: shipping a text placeholder (it cannot restart after apply)`);
    }
    return `MOCK ${key} v${MOCK_VERSION}`;
}

/** 플랫폼별 에셋을 만들고 manifest를 돌려준다 (요청이 처음 올 때 한 번) */
const manifests = new Map();
function manifestFor(platform, baseUrl) {
    if (manifests.has(platform)) return manifests.get(platform);
    fs.mkdirSync(ASSETS_DIR, { recursive: true });
    const components = {};
    for (const [key, spec] of Object.entries(componentSpecs(platform))) {
        const real = ZIPS_DIR && path.join(ZIPS_DIR, spec.asset);
        const zip = real && fs.existsSync(real) ? fs.readFileSync(real) : createMiniZip(spec.inner, assetContent(key, spec));
        fs.writeFileSync(path.join(ASSETS_DIR, spec.asset), zip);
        components[key] = {
            version: MOCK_VERSION,
            asset: spec.asset,
            size_bytes: zip.length,
            sha256: crypto.createHash('sha256').update(zip).digest('hex'),
            url: `${baseUrl}/download/${spec.asset}`,
            install_dir: spec.install_dir,
        };
    }
    const manifest = {
        version: MOCK_VERSION,
        channel: 'stable',
        platform,
        released_at: new Date().toISOString(),
        is_critical: false,
        components,
    };
    manifests.set(platform, manifest);
    console.log(`  ✓ ${platform}: ${Object.keys(components).length} components`);
    return manifest;
}

function newer(a, b) {
    const pa = String(a).split(/[.-]/).map(Number);
    const pb = String(b).split(/[.-]/).map(Number);
    for (let i = 0; i < 3; i++) {
        if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) > (pb[i] || 0);
    }
    return false;
}

// ─── 서버 ───────────────────────────────────────────

function send(res, status, body) {
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify(body));
}

const server = http.createServer((req, res) => {
    const baseUrl = `http://127.0.0.1:${PORT}`;
    const url = new URL(req.url, baseUrl);
    const pathname = url.pathname;
    console.log(`[${new Date().toISOString().slice(11, 19)}] ${req.method} ${pathname}${url.search}`);

    if (req.method === 'GET' && pathname === '/v1/updates/check') {
        const platform = url.searchParams.get('platform') || 'windows-x64';
        const current = url.searchParams.get('current') || '';
        manifestFor(platform, baseUrl);
        const latest = {
            version: MOCK_VERSION,
            channel: 'stable',
            isCritical: false,
            releasedAt: manifests.get(platform).released_at,
            manifestUrl: `${baseUrl}/v1/updates/manifest/${MOCK_VERSION}?platform=${encodeURIComponent(platform)}`,
            notes: { ko: `모의 릴리즈 v${MOCK_VERSION}`, en: `Mock release v${MOCK_VERSION}` },
        };
        return send(res, 200, { upToDate: current !== '' && !newer(MOCK_VERSION, current), latest });
    }

    const manifestMatch = pathname.match(/^\/v1\/updates\/manifest\/([^/]+)$/);
    if (req.method === 'GET' && manifestMatch) {
        if (decodeURIComponent(manifestMatch[1]) !== MOCK_VERSION) return send(res, 404, { error: 'not_found' });
        const platform = url.searchParams.get('platform') || 'windows-x64';
        return send(res, 200, signManifest(manifestFor(platform, baseUrl)));
    }

    const downloadMatch = pathname.match(/^\/download\/([^/]+)$/);
    if (req.method === 'GET' && downloadMatch) {
        const file = path.join(ASSETS_DIR, path.basename(decodeURIComponent(downloadMatch[1])));
        if (!fs.existsSync(file)) return send(res, 404, { error: 'not_found' });
        const stat = fs.statSync(file);
        res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Length': stat.size });
        return fs.createReadStream(file).pipe(res);
    }

    send(res, 404, { error: 'not_found', path: pathname });
});

server.listen(PORT, '127.0.0.1', () => {
    console.log(`[Mock Relay] Listening on http://127.0.0.1:${PORT} (release v${MOCK_VERSION}, key ${MOCK_KEY_ID})`);
    console.log('  Needs a daemon built with --features mock-relay; set update source=relay, relay_url to this address.');
});
