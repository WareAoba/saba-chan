#!/usr/bin/env node
// R2 멀티파트 업로드 (AWS SDK v3 + @aws-sdk/lib-storage)
// 사용법: node scripts/upload-r2.mjs <key> <file>
import { S3Client } from "@aws-sdk/client-s3";
import { Upload } from "@aws-sdk/lib-storage";
import fs from "node:fs";

const [key, file] = process.argv.slice(2);
if (!key || !file) {
    console.error("usage: upload-r2.mjs <key> <file>");
    process.exit(2);
}

const endpoint = process.env.R2_ENDPOINT;
const bucket = process.env.R2_BUCKET;
const accessKeyId = process.env.R2_ACCESS_KEY_ID;
const secretAccessKey = process.env.R2_SECRET_ACCESS_KEY;
if (!endpoint || !bucket || !accessKeyId || !secretAccessKey) {
    console.error("R2_ENDPOINT / R2_BUCKET / R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY 필요");
    process.exit(2);
}

const s3 = new S3Client({
    region: "auto",
    endpoint,
    credentials: { accessKeyId, secretAccessKey },
});

const stream = fs.createReadStream(file);
const size = fs.statSync(file).size;

// 확장자 기반 Content-Type 추론 — 이미지는 브라우저에서 바로 렌더될 수 있어야 한다.
const EXT_TO_MIME = {
    ".png": "image/png",
    ".jpg": "image/jpeg",
    ".jpeg": "image/jpeg",
    ".gif": "image/gif",
    ".svg": "image/svg+xml",
    ".webp": "image/webp",
    ".json": "application/json",
    ".zip": "application/zip",
};
const extMatch = /\.[A-Za-z0-9]+$/.exec(file);
const contentType = (extMatch && EXT_TO_MIME[extMatch[0].toLowerCase()]) || "application/octet-stream";

const upload = new Upload({
    client: s3,
    params: {
        Bucket: bucket,
        Key: key,
        Body: stream,
        ContentType: contentType,
    },
    queueSize: 4,
    partSize: 8 * 1024 * 1024, // 8MB parts
});

let lastPct = -1;
upload.on("httpUploadProgress", (p) => {
    if (p.total) {
        const pct = Math.floor((p.loaded / p.total) * 100);
        if (pct !== lastPct && pct % 10 === 0) {
            lastPct = pct;
            process.stderr.write(`    ${pct}% (${(p.loaded / 1048576).toFixed(1)} / ${(size / 1048576).toFixed(1)} MB)\n`);
        }
    }
});

try {
    await upload.done();
    console.log(`OK ${key}`);
} catch (err) {
    console.error(`FAIL ${key}: ${err?.message ?? err}`);
    process.exit(1);
}
