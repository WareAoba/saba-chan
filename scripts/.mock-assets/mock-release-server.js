#!/usr/bin/env node
/**
 * Mock GitHub Release Server — 로컬에서 업데이터 다운로드/적용 E2E 테스트용
 *
 * GitHub REST API와 동일한 엔드포인트 형식으로 응답합니다:
 *   GET /repos/{owner}/{repo}/releases?per_page=N
 *   GET /repos/{owner}/{repo}/releases/latest
 *   GET /download/{filename}  ← asset download_url 대체
 *
 * ## 사용법
 *   node scripts/mock-release-server.js [--port 9876] [--version 0.2.0]
 *
 * ## 생성되는 파일
 *   scripts/.mock-assets/
 *     manifest.json
 *     saba-core-windows-x64.zip
 *     saba-chan-cli-windows-x64.zip
 *     saba-chan-gui-windows-x64.zip
 *     module-minecraft.zip
 *
 * ## 업데이터에서 연결
 *   UpdateConfig에 api_base_url = "http://127.0.0.1:9876" 설정
 */

const http = require('http');
const fs = require('fs');
const path = require('path');
const { createWriteStream } = require('fs');

// ─── CLI 인자 파싱 ──────────────────────────────────
const args = process.argv.slice(2);
function getArg(name, defaultVal) {
    const idx = args.indexOf(name);
    return idx >= 0 && args[idx + 1] ? args[idx + 1] : defaultVal;
}

const PORT = parseInt(getArg('--port', '9876'), 10);
const MOCK_VERSION = getArg('--version', '0.2.0');
const ASSETS_DIR = path.join(__dirname, '.mock-assets');
const OWNER = 'test-owner';
const REPO = 'saba-chan';

// ─── 가짜 ZIP 생성 ─────────────────────────────────
// 진짜 ZIP은 아니지만, 업데이터의 zip::ZipArchive가 파싱할 수 있어야 합니다.
// 최소한의 유효한 ZIP 파일을 생성합니다 (PK 매직넘버 + 단일 파일).

function createMiniZip(innerFileName, innerContent) {
    // ZIP local file header + central directory + end-of-central-directory
    const fnBuf = Buffer.from(innerFileName, 'utf-8');
    const dataBuf = Buffer.from(innerContent, 'utf-8');
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

// ─── 에셋 파일 생성 ──────────────────────────────────

function generateAssets() {
    fs.mkdirSync(ASSETS_DIR, { recursive: true });

    const components = {
        'saba-core-windows-x64.zip': {
            innerFile: 'saba-core.exe',
            content: `MOCK_SABA_CORE_v${MOCK_VERSION}_${Date.now()}`,
        },
        'saba-chan-cli-windows-x64.zip': {
            innerFile: 'saba-chan-cli.exe',
            content: `MOCK_CLI_v${MOCK_VERSION}_${Date.now()}`,
        },
        'saba-chan-gui-windows-x64.zip': {
            innerFile: 'index.html',
            content: `<html><body>MOCK GUI v${MOCK_VERSION}</body></html>`,
        },
        'module-minecraft.zip': {
            innerFile: 'module.toml',
            content: `name = "minecraft"\nversion = "${MOCK_VERSION}"\ntype = "java"\n`,
        },
        'discord-bot.zip': {
            innerFile: 'package.json',
            content: JSON.stringify({ name: "saba-chan-discord-bot", version: MOCK_VERSION, main: "index.js" }, null, 2),
        },
    };

    // manifest.json — 본체 전용 (모듈은 별도 레포에서 관리)
    const manifest = {
        release_version: MOCK_VERSION,
        components: {
            'saba-core': {
                version: MOCK_VERSION,
                asset: 'saba-core-windows-x64.zip',
                sha256: null,
                install_dir: '.',
            },
            cli: {
                version: MOCK_VERSION,
                asset: 'saba-chan-cli-windows-x64.zip',
                sha256: null,
                install_dir: '.',
            },
            gui: {
                version: MOCK_VERSION,
                asset: 'saba-chan-gui-windows-x64.zip',
                sha256: null,
                install_dir: 'saba-chan-gui',
            },
            discord_bot: {
                version: MOCK_VERSION,
                asset: 'discord-bot.zip',
                sha256: null,
                install_dir: 'discord_bot',
            },
        },
    };
    fs.writeFileSync(path.join(ASSETS_DIR, 'manifest.json'), JSON.stringify(manifest, null, 2));

    // ZIP 에셋 생성
    for (const [filename, info] of Object.entries(components)) {
        const zipBuf = createMiniZip(info.innerFile, info.content);
        fs.writeFileSync(path.join(ASSETS_DIR, filename), zipBuf);
        console.log(`  ✓ ${filename} (${zipBuf.length} bytes)`);
    }

    console.log(`  ✓ manifest.json`);

    // 모듈별 릴리스 정보 (별도 레포 시뮬레이션)
    const moduleReleases = {
        'saba-chan-module-minecraft': {
            tag_name: `v${MOCK_VERSION}`,
            name: `Minecraft Module v${MOCK_VERSION} (Mock)`,
            body: `Mock module release`,
            prerelease: false,
            draft: false,
            published_at: new Date().toISOString(),
            assets: [{
                name: 'module-minecraft.zip',
                size: fs.statSync(path.join(ASSETS_DIR, 'module-minecraft.zip')).size,
            }],
        },
    };

    return { manifest, assetFiles: Object.keys(components), moduleReleases };
}

// ─── GitHub API 응답 작성 ───────────────────────────

function buildReleaseResponse(manifest, assetFiles, baseUrl) {
    const allAssets = ['manifest.json', ...assetFiles];

    return {
        tag_name: `v${MOCK_VERSION}`,
        name: `Saba-chan v${MOCK_VERSION} (Mock)`,
        body: `This is a mock release for local testing.\n\nGenerated at ${new Date().toISOString()}`,
        prerelease: false,
        draft: false,
        published_at: new Date().toISOString(),
        html_url: `${baseUrl}/releases/tag/v${MOCK_VERSION}`,
        assets: allAssets.map((name) => {
            const filePath = path.join(ASSETS_DIR, name);
            const size = fs.existsSync(filePath) ? fs.statSync(filePath).size : 0;
            return {
                name,
                size,
                browser_download_url: `${baseUrl}/download/${name}`,
                content_type: name.endsWith('.json') ? 'application/json' : 'application/zip',
            };
        }),
    };
}

// ─── HTTP 서버 ──────────────────────────────────────

function startServer(manifest, assetFiles, moduleReleases) {
    const server = http.createServer((req, res) => {
        const url = new URL(req.url, `http://127.0.0.1:${PORT}`);
        const pathname = url.pathname;
        const baseUrl = `http://127.0.0.1:${PORT}`;

        console.log(`[${new Date().toISOString().slice(11, 19)}] ${req.method} ${pathname}`);

        // GET /repos/{owner}/{repo}/releases — repo에 따라 응답 분기
        const releasesMatch = pathname.match(/^\/repos\/[^/]+\/([^/]+)\/releases$/);
        if (releasesMatch && req.method === 'GET') {
            const repoName = releasesMatch[1];

            // 모듈 레포 체크
            if (moduleReleases && moduleReleases[repoName]) {
                const modInfo = moduleReleases[repoName];
                const release = {
                    ...modInfo,
                    html_url: `${baseUrl}/releases/tag/${modInfo.tag_name}`,
                    assets: modInfo.assets.map((a) => ({
                        ...a,
                        browser_download_url: `${baseUrl}/download/${a.name}`,
                        content_type: 'application/zip',
                    })),
                };
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify([release]));
                return;
            }

            // 본체 레포 (기본)
            const release = buildReleaseResponse(manifest, assetFiles, baseUrl);
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify([release]));
            return;
        }

        // GET /repos/{owner}/{repo}/releases/latest
        const latestMatch = pathname.match(/^\/repos\/[^/]+\/([^/]+)\/releases\/latest$/);
        if (latestMatch && req.method === 'GET') {
            const repoName = latestMatch[1];

            if (moduleReleases && moduleReleases[repoName]) {
                const modInfo = moduleReleases[repoName];
                const release = {
                    ...modInfo,
                    html_url: `${baseUrl}/releases/tag/${modInfo.tag_name}`,
                    assets: modInfo.assets.map((a) => ({
                        ...a,
                        browser_download_url: `${baseUrl}/download/${a.name}`,
                        content_type: 'application/zip',
                    })),
                };
                res.writeHead(200, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify(release));
                return;
            }

            const release = buildReleaseResponse(manifest, assetFiles, baseUrl);
            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify(release));
            return;
        }

        // GET /download/{filename} — 에셋 다운로드
        const downloadMatch = pathname.match(/^\/download\/(.+)$/);
        if (downloadMatch && req.method === 'GET') {
            const filename = decodeURIComponent(downloadMatch[1]);
            const filePath = path.join(ASSETS_DIR, filename);

            if (!fs.existsSync(filePath)) {
                res.writeHead(404, { 'Content-Type': 'application/json' });
                res.end(JSON.stringify({ error: 'Asset not found', file: filename }));
                return;
            }

            const stat = fs.statSync(filePath);
            const contentType = filename.endsWith('.json') ? 'application/json' : 'application/octet-stream';
            res.writeHead(200, {
                'Content-Type': contentType,
                'Content-Length': stat.size,
            });
            fs.createReadStream(filePath).pipe(res);
            return;
        }

        // 404
        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'Not found', path: pathname }));
    });

    server.listen(PORT, '127.0.0.1', () => {
        console.log('');
        console.log('═══════════════════════════════════════════════════');
        console.log(`  Mock GitHub Release Server — v${MOCK_VERSION}`);
        console.log(`  http://127.0.0.1:${PORT}`);
        console.log('═══════════════════════════════════════════════════');
        console.log('');
        console.log('  업데이터 설정:');
        console.log(`    api_base_url = "http://127.0.0.1:${PORT}"`);
        console.log(`    github_owner = "${OWNER}"`);
        console.log(`    github_repo  = "${REPO}"`);
        console.log('');
        console.log('  엔드포인트:');
        console.log(`    GET /repos/${OWNER}/${REPO}/releases`);
        console.log(`    GET /repos/${OWNER}/${REPO}/releases/latest`);
        console.log(`    GET /download/{filename}`);
        console.log('');
        console.log('  Ctrl+C로 종료');
        console.log('═══════════════════════════════════════════════════');
        console.log('');
    });

    return server;
}

// ─── 메인 ───────────────────────────────────────────

console.log(`[Mock Server] Generating mock assets (v${MOCK_VERSION})...`);
const { manifest, assetFiles, moduleReleases } = generateAssets();
startServer(manifest, assetFiles, moduleReleases);
