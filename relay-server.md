# Saba-chan Relay Server — 구현 청사진 v2

> **v2 변경점**: 5모듈 봇 아키텍처 반영, SQLite → PostgreSQL 17 + Drizzle ORM, 음악 익스텐션 로컬 전용 명시, 메타데이터 동기화 추가, Phase 6 전면 재작성

## 개요

사바쨩 Discord 봇의 명령어를 중앙 서버를 경유하여 방장의 로컬 사바쨩 프로세스에 전달하는 릴레이 시스템.
중앙 서버는 유저 검증 후 payload를 그대로 전달만 한다. 명령어 해석, 인스턴스 관리, 모듈 로직은 모두 로컬 사바쨩이 담당.

### 핵심 원칙

- 중앙 서버는 payload를 해석하지 않는다 (택배 기사)
- 방장 1명 = Discord 계정 1개 = 로컬 사바쨩 프로세스 1개 = 노드 1개
- `local` / `cloud` 모드 토글로 기존 로직 100% 유지
- 노드가 서버에 접속 (Pull 모델), IP 저장 절대 없음
- **음악 익스텐션은 로컬 모드 전용** — Voice 연결이 필요하므로 릴레이 불가

### 현재 봇 아키텍처 (5모듈 구조)

```
discord_bot/
├── index.js            진입점 · 프로세스 관리
├── core/
│   ├── ipc.js          IPC 통신 (토큰, axios, API 래퍼)      ← 클라우드 모드 분기점
│   ├── resolver.js     별명/매핑 통합 (botConfig, moduleMetadata)
│   ├── processor.js    명령어 해석 · 디스패치              ← 변경 불필요
│   └── handler.js      봇 자체 기능 (익스텐션 파이프라인)  ← 모드 인식 필터링
├── extensions/
│   ├── music.js        🎵 음악 재생 (로컬 전용)
│   ├── easter_eggs.js  🥚 이스터 에그 (양쪽 모드)
│   └── rps.js          ✊ 가위바위보 (양쪽 모드)
├── utils/
│   └── aliasResolver.js
├── i18n.js
└── bot-config.json
```

**명령어 흐름**: `messageCreate` → `processor.process()` → ① `handler.handle()` (익스텐션) → ② help/list → ③ `handleModuleCommand()` → `ipc.*()` API 호출

클라우드 모드의 **핵심 분기점은 `core/ipc.js`**. processor.js는 ipc/resolver/handler 추상화만 사용하므로 변경 불필요.

### 기술 스택

- 중앙 서버: Node.js 22+, Fastify 5, TypeScript 5, **PostgreSQL 17**, **Drizzle ORM**, discord.js 14
- 노드 에이전트: Node.js 22+, TypeScript 5
- 인증: argon2id (토큰 해싱), HMAC-SHA256 (요청 서명), 자체 토큰 (`sbn_` prefix)
- 인프라: VPS 1대, Cloudflare (DNS + SSL + DDoS 방어)

### DB 선정 근거: PostgreSQL 17 + Drizzle ORM

**왜 SQLite가 아닌가?**

| 관점 | SQLite | PostgreSQL |
|------|--------|------------|
| 동시 쓰기 | WAL로도 단일 writer | 완전 MVCC 병렬 쓰기 |
| 실시간 알림 | 불가 | `LISTEN/NOTIFY` 네이티브 |
| JSON 처리 | json1 확장 | `JSONB` 인덱싱 포함 |
| 커넥션 | 단일 프로세스 | 멀티 커넥션 풀 |
| 운영 | 파일 1개 | Docker 컨테이너 1개 추가 |

릴레이 서버는 다수의 노드가 동시에 long-poll하고, Discord 봇이 동시에 명령을 큐잉하는 구조. 진정한 동시성과 `LISTEN/NOTIFY`(PollWaiters 내부 트리거)가 결정적 이점.

**Drizzle ORM**: TypeScript 생태계에서 가장 최신의 타입 안전 ORM. SQL-like 문법, 제로 오버헤드, 마이그레이션 내장.

> 💡 **경량 대안**: VPS 리소스가 제한적이라면 [Turso (libSQL)](https://turso.tech/) — SQLite 포크에 HTTP 접근 + 리플리케이션 추가. 스키마 거의 동일하게 유지 가능.

---

## Phase 1: relay-server 프로젝트 초기화

### Task 1-1: 프로젝트 스캐폴딩

`relay-server/` 디렉토리 생성 및 초기 파일 구성.

생성할 파일:

- `relay-server/package.json`
- `relay-server/tsconfig.json`
- `relay-server/.env.example`
- `relay-server/src/index.ts` (Fastify 진입점, 빈 서버)

`package.json` 의존성:

```json
{
  "type": "module",
  "dependencies": {
    "fastify": "^5",
    "@fastify/helmet": "^12",
    "@fastify/rate-limit": "^10",
    "@fastify/cors": "^10",
    "discord.js": "^14",
    "drizzle-orm": "^0.44",
    "postgres": "^3",
    "zod": "^3",
    "argon2": "^0.41",
    "nanoid": "^5",
    "pino": "^9"
  },
  "devDependencies": {
    "typescript": "^5",
    "drizzle-kit": "^0.31",
    "@types/node": "^22",
    "tsx": "^4"
  },
  "scripts": {
    "dev": "tsx watch src/index.ts",
    "build": "tsc",
    "start": "node dist/index.js",
    "db:generate": "drizzle-kit generate",
    "db:migrate": "drizzle-kit migrate",
    "db:studio": "drizzle-kit studio"
  }
}
```

`tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "declaration": true
  },
  "include": ["src"]
}
```

`.env.example`:

```
PORT=3000
DATABASE_URL=postgresql://saba:saba@localhost:5432/saba_relay
DISCORD_TOKEN=
DISCORD_APP_ID=
```

`drizzle.config.ts`:

```typescript
import { defineConfig } from 'drizzle-kit';

export default defineConfig({
    schema: './src/db/schema.ts',
    out: './drizzle',
    dialect: 'postgresql',
    dbCredentials: {
        url: process.env.DATABASE_URL!,
    },
});
```

`src/index.ts` 초기 구현:

```typescript
import Fastify from 'fastify';

const app = Fastify({ logger: true });

app.get('/info', async () => ({
  name: 'saba-chan-relay',
  apiVersion: 2,
  minAgentVersion: '2.0.0',
}));

const port = parseInt(process.env.PORT ?? '3000');
await app.listen({ port, host: '0.0.0.0' });
```

### Task 1-2: 데이터베이스 (PostgreSQL 17 + Drizzle ORM)

생성할 파일:

- `relay-server/src/db/schema.ts`
- `relay-server/src/db/index.ts`

`src/db/schema.ts` — Drizzle 스키마 정의 (8개 테이블):

```typescript
import {
    pgTable, text, integer, boolean, timestamp,
    jsonb, uniqueIndex, index, primaryKey, serial,
} from 'drizzle-orm/pg-core';
import { sql } from 'drizzle-orm';

// ── 사용자 ──
export const users = pgTable('users', {
    discordId:   text('discord_id').primaryKey(),
    username:    text('username').notNull(),
    displayName: text('display_name'),
    isBanned:    boolean('is_banned').notNull().default(false),
    createdAt:   timestamp('created_at').notNull().defaultNow(),
    lastSeen:    timestamp('last_seen'),
});

// ── 방장 (노드) ──
export const hosts = pgTable('hosts', {
    id:            text('id').primaryKey(),
    discordId:     text('discord_id').notNull().unique()
                       .references(() => users.discordId),
    name:          text('name').notNull(),
    tokenHash:     text('token_hash').notNull(),
    status:        text('status').notNull().default('offline'),
    lastHeartbeat: timestamp('last_heartbeat'),
    agentVersion:  text('agent_version'),
    metadata:      jsonb('metadata'),        // ★ 신규: 모듈 메타데이터 캐시
    createdAt:     timestamp('created_at').notNull().defaultNow(),
    updatedAt:     timestamp('updated_at').notNull().defaultNow(),
}, (t) => [
    index('idx_hosts_discord').on(t.discordId),
]);

// ── 개별 사용자 권한 ──
export const permissions = pgTable('permissions', {
    id:             serial('id').primaryKey(),
    hostId:         text('host_id').notNull()
                        .references(() => hosts.id, { onDelete: 'cascade' }),
    userDiscordId:  text('user_discord_id').notNull()
                        .references(() => users.discordId, { onDelete: 'cascade' }),
    permissionLevel: integer('permission_level').notNull().default(1),
    grantedBy:      text('granted_by').references(() => users.discordId),
    grantedAt:      timestamp('granted_at').notNull().defaultNow(),
}, (t) => [
    uniqueIndex('uq_perm_host_user').on(t.hostId, t.userDiscordId),
    index('idx_permissions_host').on(t.hostId),
    index('idx_permissions_user').on(t.userDiscordId),
]);

// ── 역할 권한 ──
export const rolePermissions = pgTable('role_permissions', {
    id:              serial('id').primaryKey(),
    hostId:          text('host_id').notNull()
                         .references(() => hosts.id, { onDelete: 'cascade' }),
    guildId:         text('guild_id').notNull(),
    discordRoleId:   text('discord_role_id').notNull(),
    permissionLevel: integer('permission_level').notNull().default(1),
}, (t) => [
    uniqueIndex('uq_role_perm').on(t.hostId, t.guildId, t.discordRoleId),
    index('idx_role_perms_host').on(t.hostId),
]);

// ── 길드-호스트 연결 ──
export const guildHosts = pgTable('guild_hosts', {
    guildId:  text('guild_id').notNull(),
    hostId:   text('host_id').notNull()
                  .references(() => hosts.id, { onDelete: 'cascade' }),
    guildName: text('guild_name'),
    linkedAt: timestamp('linked_at').notNull().defaultNow(),
    linkedBy: text('linked_by').references(() => users.discordId),
}, (t) => [
    primaryKey({ columns: [t.guildId, t.hostId] }),
    index('idx_guild_hosts_guild').on(t.guildId),
]);

// ── 명령어 큐 ──
export const commandQueue = pgTable('command_queue', {
    id:              text('id').primaryKey(),
    hostId:          text('host_id').notNull()
                         .references(() => hosts.id, { onDelete: 'cascade' }),
    payload:         jsonb('payload').notNull(),       // ★ JSONB로 변경
    requestedBy:     text('requested_by').notNull(),
    guildId:         text('guild_id'),
    channelId:       text('channel_id'),
    interactionToken: text('interaction_token'),
    status:          text('status').notNull().default('pending'),
    createdAt:       timestamp('created_at').notNull().defaultNow(),
    deliveredAt:     timestamp('delivered_at'),
    completedAt:     timestamp('completed_at'),
    expiresAt:       timestamp('expires_at').notNull(),
    result:          jsonb('result'),                  // ★ JSONB로 변경
}, (t) => [
    index('idx_queue_host_status').on(t.hostId, t.status),
    index('idx_queue_expires').on(t.expiresAt),
]);

// ── 감사 로그 ──
export const auditLogs = pgTable('audit_logs', {
    id:             serial('id').primaryKey(),
    timestamp:      timestamp('timestamp').notNull().defaultNow(),
    userDiscordId:  text('user_discord_id').notNull(),
    hostId:         text('host_id'),
    guildId:        text('guild_id'),
    action:         text('action').notNull(),
    detail:         jsonb('detail'),                   // ★ JSONB로 변경
    result:         text('result').notNull().default('success'),
}, (t) => [
    index('idx_audit_time').on(t.timestamp),
]);
```

> ★ v1 대비 변경: `hosts.metadata` 컬럼 추가 (노드 에이전트가 하트비트 시 모듈 메타데이터 동기화), payload/result/detail을 JSONB로 전환하여 인덱싱 및 부분 조회 가능.

`src/db/index.ts`:

```typescript
import { drizzle } from 'drizzle-orm/postgres-js';
import postgres from 'postgres';
import * as schema from './schema.js';

export function initDatabase(databaseUrl: string) {
    const client = postgres(databaseUrl, {
        max: 10,           // 커넥션 풀
        idle_timeout: 20,
    });

    const db = drizzle(client, { schema });

    return { db, client };
}

export type DB = ReturnType<typeof initDatabase>['db'];
```

Drizzle Kit이 마이그레이션을 자동 관리하므로, 별도 schema.sql이나 수동 마이그레이션 코드 불필요.

```bash
# 스키마 변경 시
npm run db:generate   # drizzle/ 폴더에 SQL 생성
npm run db:migrate    # PostgreSQL에 적용
```

### Task 1-3: Fastify 플러그인 연결

`src/index.ts`를 확장하여 DB, 보안 미들웨어, rate limit 연결.

수정할 파일:

- `relay-server/src/index.ts`

생성할 파일:

- `relay-server/src/middleware/rateLimit.ts`

`src/index.ts` 전체:

```typescript
import Fastify from 'fastify';
import helmet from '@fastify/helmet';
import cors from '@fastify/cors';
import { initDatabase, type DB } from './db/index.js';
import { setupRateLimit } from './middleware/rateLimit.js';
import { PollWaiters } from './services/pollWaiters.js';

// Fastify 타입 확장
declare module 'fastify' {
    interface FastifyInstance {
        db: DB;
        pollWaiters: PollWaiters;
        discordAppId: string;
        authenticateNode: typeof import('./middleware/auth.js').authenticateNode;
    }
}

const app = Fastify({ logger: true });

// 보안
await app.register(helmet);
await app.register(cors, { origin: false }); // API 서버, 브라우저 접근 불필요

// DB (PostgreSQL + Drizzle)
const { db, client: pgClient } = initDatabase(
    process.env.DATABASE_URL ?? 'postgresql://saba:saba@localhost:5432/saba_relay',
);
app.decorate('db', db);
app.decorate('discordAppId', process.env.DISCORD_APP_ID ?? '');

// Poll Waiters
app.decorate('pollWaiters', new PollWaiters());

// Rate Limit
await setupRateLimit(app);

// 라우트 등록 (이후 Task에서 구현)
// await app.register(commandRoutes);
// await app.register(pollRoutes);
// await app.register(resultRoutes);
// await app.register(heartbeatRoutes);
// await app.register(hostRoutes);

app.get('/info', async () => ({
    name: 'saba-chan-relay',
    apiVersion: 2,
    minAgentVersion: '2.0.0',
}));

// 종료 시 PG 커넥션 풀 닫기
app.addHook('onClose', () => pgClient.end());

const port = parseInt(process.env.PORT ?? '3000');
await app.listen({ port, host: '0.0.0.0' });
```

`src/middleware/rateLimit.ts`:

```typescript
import rateLimit from '@fastify/rate-limit';
import { FastifyInstance } from 'fastify';

export async function setupRateLimit(app: FastifyInstance) {
    await app.register(rateLimit, {
        global: true,
        max: 100,
        timeWindow: 60000,
    });
}
```

---

## Phase 2: 인증 시스템

### Task 2-1: 노드 토큰 서비스

생성할 파일:

- `relay-server/src/services/nodeToken.ts`

기능:

- `generateNodeToken(nodeId: string)` → `{ raw, hash }`. raw는 `sbn_{nodeId}.{nanoid(48)}` 형식. hash는 argon2id.
- `parseToken(token: string)` → `{ nodeId, secret, raw } | null`. 정규식 `/^sbn_([A-Za-z0-9_-]+)\.(.+)$/`.
- `verifyNodeToken(raw: string, storedHash: string)` → `boolean`. argon2.verify.

```typescript
import { nanoid } from 'nanoid';
import * as argon2 from 'argon2';

export interface TokenParts {
    nodeId: string;
    secret: string;
    raw: string;
}

export async function generateNodeToken(nodeId: string): Promise<{ raw: string; hash: string }> {
    const secret = nanoid(48);
    const raw = `sbn_${nodeId}.${secret}`;
    const hash = await argon2.hash(raw, {
        type: argon2.argon2id,
        memoryCost: 19456,
        timeCost: 2,
        parallelism: 1,
    });
    return { raw, hash };
}

export function parseToken(token: string): TokenParts | null {
    const m = token.match(/^sbn_([A-Za-z0-9_-]+)\.(.+)$/);
    if (!m) return null;
    return { nodeId: m[1], secret: m[2], raw: token };
}

export async function verifyNodeToken(raw: string, storedHash: string): Promise<boolean> {
    try {
        return await argon2.verify(storedHash, raw);
    } catch {
        return false;
    }
}
```

### Task 2-2: 노드 인증 미들웨어

생성할 파일:

- `relay-server/src/middleware/auth.ts`

기능:

- `authenticateNode` — Fastify preHandler.
- Bearer 토큰 추출 → parseToken → DB 해시 비교 (5분 캐시) → HMAC 서명 검증 → 타임스탬프 ±30초 → `req.node = { id }` 첨부.
- 실패 시 1초 딜레이 후 401.
- 검증 순서: 토큰 형식 → DB 존재 → argon2 해시 → 타임스탬프 → HMAC.

```typescript
import { FastifyRequest, FastifyReply } from 'fastify';
import { parseToken, verifyNodeToken } from '../services/nodeToken.js';
import { createHmac, timingSafeEqual } from 'crypto';
import { eq } from 'drizzle-orm';
import { hosts } from '../db/schema.js';

const TOKEN_CACHE = new Map<string, { hash: string; verifiedAt: number }>();
const CACHE_TTL = 300;

export async function authenticateNode(req: FastifyRequest, reply: FastifyReply) {
    const authHeader = req.headers['authorization'];
    if (!authHeader?.startsWith('Bearer ')) {
        return reply.code(401).send({ error: 'MISSING_TOKEN' });
    }

    const rawToken = authHeader.slice(7);
    const parsed = parseToken(rawToken);
    if (!parsed) {
        return reply.code(401).send({ error: 'INVALID_TOKEN_FORMAT' });
    }

    const now = Math.floor(Date.now() / 1000);

    // DB 해시 검증 (캐시)
    let cached = TOKEN_CACHE.get(parsed.nodeId);
    if (!cached || now - cached.verifiedAt > CACHE_TTL) {
        const host = await req.server.db.query.hosts.findFirst({
            where: eq(hosts.id, parsed.nodeId),
            columns: { id: true, tokenHash: true },
        });

        if (!host) {
            await sleep(1000);
            return reply.code(401).send({ error: 'NODE_NOT_FOUND' });
        }

        const valid = await verifyNodeToken(rawToken, host.tokenHash);
        if (!valid) {
            await sleep(1000);
            return reply.code(401).send({ error: 'INVALID_TOKEN' });
        }

        cached = { hash: host.tokenHash, verifiedAt: now };
        TOKEN_CACHE.set(parsed.nodeId, cached);
    }

    // 타임스탬프
    const ts = parseInt(req.headers['x-request-timestamp'] as string);
    if (!ts || Math.abs(now - ts) > 30) {
        return reply.code(401).send({ error: 'TIMESTAMP_EXPIRED' });
    }

    // HMAC 서명
    const sig = req.headers['x-request-signature'] as string;
    if (!sig) {
        return reply.code(401).send({ error: 'MISSING_SIGNATURE' });
    }

    const payload = [
        req.method.toUpperCase(),
        req.url.split('?')[0],
        ts.toString(),
        req.body ? JSON.stringify(req.body) : '',
    ].join('\n');
    const expected = createHmac('sha256', parsed.secret)
        .update(payload)
        .digest('hex');

    const sigBuf = Buffer.from(sig, 'hex');
    const expBuf = Buffer.from(expected, 'hex');
    if (sigBuf.length !== expBuf.length || !timingSafeEqual(sigBuf, expBuf)) {
        return reply.code(401).send({ error: 'INVALID_SIGNATURE' });
    }

    // online 갱신
    await req.server.db.update(hosts)
        .set({ status: 'online', lastHeartbeat: new Date(), updatedAt: new Date() })
        .where(eq(hosts.id, parsed.nodeId));

    (req as any).node = { id: parsed.nodeId };
}

function sleep(ms: number) {
    return new Promise(r => setTimeout(r, ms));
}
```

`src/index.ts`에서 등록:

```typescript
import { authenticateNode } from './middleware/auth.js';
app.decorate('authenticateNode', authenticateNode);
```

### Task 2-3: TOKEN_CACHE 외부 접근

`auth.ts`에서 TOKEN_CACHE를 export하여 토큰 갱신 시 캐시 무효화:

```typescript
export { TOKEN_CACHE };
```

---

## Phase 3: 핵심 서비스

### Task 3-1: PollWaiters 서비스

생성할 파일:

- `relay-server/src/services/pollWaiters.ts`

기능:

- `wait(hostId, timeoutMs)` → Promise. 타임아웃 또는 wake 시 resolve.
- `wake(hostId)` → 대기 중인 poll을 즉시 깨움.
- `activeCount` getter.
- 이전 waiter가 있으면 자동 cancel.

```typescript
export class PollWaiters {
    private waiters = new Map<string, {
        resolve: () => void;
        timer: NodeJS.Timeout;
    }>();

    wait(hostId: string, timeoutMs: number): Promise<void> {
        this.cancel(hostId);
        return new Promise<void>((resolve) => {
            const timer = setTimeout(() => {
                this.waiters.delete(hostId);
                resolve();
            }, timeoutMs);
            this.waiters.set(hostId, { resolve, timer });
        });
    }

    wake(hostId: string): void {
        const w = this.waiters.get(hostId);
        if (w) {
            clearTimeout(w.timer);
            this.waiters.delete(hostId);
            w.resolve();
        }
    }

    private cancel(hostId: string): void {
        const w = this.waiters.get(hostId);
        if (w) {
            clearTimeout(w.timer);
            this.waiters.delete(hostId);
        }
    }

    get activeCount(): number {
        return this.waiters.size;
    }
}
```

### Task 3-2: ACL 서비스

생성할 파일:

- `relay-server/src/services/acl.ts`

권한 레벨:

- 0 = NONE (접근 불가)
- 1 = USER (명령 전송 가능, 세부 제어는 로컬)
- 2 = ADMIN (권한 관리 가능)
- 방장 본인은 별도 체크로 무조건 ADMIN

기능:

- `resolve({ userDiscordId, hostId, guildId?, memberRoleIds? })` → PermLevel.
  - 해석 순서: banned → 방장 본인 → permissions 직접 → role_permissions → 최댓값.
- `grant(hostId, targetDiscordId, level, grantedBy)` → boolean. 호출자가 ADMIN이어야 함. UPSERT.
- `revoke(hostId, targetDiscordId, revokedBy)` → boolean. 호출자가 ADMIN이어야 함. DELETE.

```typescript
import { eq, and, inArray, sql } from 'drizzle-orm';
import { users, hosts, permissions, rolePermissions } from '../db/schema.js';
import type { DB } from '../db/index.js';

export enum PermLevel {
    NONE  = 0,
    USER  = 1,
    ADMIN = 2,
}

export class AclService {
    constructor(private db: DB) {}

    async resolve(p: {
        userDiscordId: string;
        hostId: string;
        guildId?: string;
        memberRoleIds?: string[];
    }): Promise<PermLevel> {
        // 1. 밴 체크
        const user = await this.db.query.users.findFirst({
            where: eq(users.discordId, p.userDiscordId),
            columns: { isBanned: true },
        });
        if (!user || user.isBanned) return PermLevel.NONE;

        // 2. 방장 본인
        const host = await this.db.query.hosts.findFirst({
            where: eq(hosts.id, p.hostId),
            columns: { discordId: true },
        });
        if (host?.discordId === p.userDiscordId) return PermLevel.ADMIN;

        let max = PermLevel.NONE;

        // 3. 직접 권한
        const direct = await this.db.query.permissions.findFirst({
            where: and(
                eq(permissions.hostId, p.hostId),
                eq(permissions.userDiscordId, p.userDiscordId),
            ),
            columns: { permissionLevel: true },
        });
        if (direct) max = Math.max(max, direct.permissionLevel);

        // 4. 역할 권한
        if (p.guildId && p.memberRoleIds?.length) {
            const roles = await this.db.select({
                maxLevel: sql<number>`MAX(${rolePermissions.permissionLevel})`,
            })
                .from(rolePermissions)
                .where(and(
                    eq(rolePermissions.hostId, p.hostId),
                    eq(rolePermissions.guildId, p.guildId),
                    inArray(rolePermissions.discordRoleId, p.memberRoleIds),
                ));
            if (roles[0]?.maxLevel != null) max = Math.max(max, roles[0].maxLevel);
        }

        return max as PermLevel;
    }

    async grant(hostId: string, target: string, level: PermLevel, by: string): Promise<boolean> {
        if (await this.resolve({ userDiscordId: by, hostId }) < PermLevel.ADMIN) return false;
        await this.db.insert(permissions)
            .values({ hostId, userDiscordId: target, permissionLevel: level, grantedBy: by })
            .onConflictDoUpdate({
                target: [permissions.hostId, permissions.userDiscordId],
                set: { permissionLevel: level, grantedBy: by, grantedAt: new Date() },
            });
        return true;
    }

    async revoke(hostId: string, target: string, by: string): Promise<boolean> {
        if (await this.resolve({ userDiscordId: by, hostId }) < PermLevel.ADMIN) return false;
        await this.db.delete(permissions)
            .where(and(
                eq(permissions.hostId, hostId),
                eq(permissions.userDiscordId, target),
            ));
        return true;
    }
}
```

### Task 3-3: 정리 스케줄러

생성할 파일:

- `relay-server/src/services/cleanup.ts`

기능:

- 6시간마다 실행.
- audit_logs 30일 초과 삭제.
- command_queue 중 완료/만료 7일 초과 삭제.
- 24시간 이상 heartbeat 없는 호스트 → offline.

```typescript
import { lt, and, inArray, sql } from 'drizzle-orm';
import { auditLogs, commandQueue, hosts } from '../db/schema.js';
import type { DB } from '../db/index.js';

export function scheduleCleanup(db: DB) {
    const run = async () => {
        const now = new Date();
        const thirtyDaysAgo = new Date(now.getTime() - 30 * 86400_000);
        const sevenDaysAgo = new Date(now.getTime() - 7 * 86400_000);
        const oneDayAgo = new Date(now.getTime() - 86400_000);

        // 감사 로그 30일 초과 삭제
        await db.delete(auditLogs)
            .where(lt(auditLogs.timestamp, thirtyDaysAgo));

        // 완료/만료된 큐 7일 초과 삭제
        await db.delete(commandQueue)
            .where(and(
                inArray(commandQueue.status, ['completed', 'timeout', 'error']),
                lt(commandQueue.createdAt, sevenDaysAgo),
            ));

        // 만료된 pending/delivered → timeout
        await db.update(commandQueue)
            .set({ status: 'timeout' })
            .where(and(
                inArray(commandQueue.status, ['pending', 'delivered']),
                lt(commandQueue.expiresAt, now),
            ));

        // 24시간 무응답 호스트 → offline
        await db.update(hosts)
            .set({ status: 'offline' })
            .where(and(
                sql`${hosts.status} = 'online'`,
                lt(hosts.lastHeartbeat, oneDayAgo),
            ));
    };

    run(); // 즉시 1회
    setInterval(run, 6 * 60 * 60 * 1000);
}
```

`src/index.ts`에서 호출:

```typescript
import { scheduleCleanup } from './services/cleanup.js';
scheduleCleanup(db);
```

---

## Phase 4: API 라우트

### Task 4-1: 방장 등록/관리 라우트

생성할 파일:

- `relay-server/src/routes/host.ts`

엔드포인트:

- `POST /api/hosts/register` — 새 방장 등록. body: `{ discordId, name, username? }`. 내부 전용 (Discord 봇이 호출). users 테이블에 upsert 후 hosts에 INSERT. 토큰 생성. 이미 등록된 경우 409. 응답에 평문 토큰 한 번만 포함.
- `POST /api/hosts/:hostId/rotate-token` — 토큰 재발급. body: `{ discordId }`. 방장 본인만. 기존 토큰 즉시 무효화 + TOKEN_CACHE 삭제.
- `GET /api/hosts/:hostId` — 상태 조회 (id, name, status, last_heartbeat, agent_version, created_at). 공개 정보만.
- `GET /api/hosts/:hostId/metadata` — ★ 신규. 노드가 동기화한 모듈 메타데이터 조회. 클라우드 모드 봇의 resolver가 사용.

```typescript
import { FastifyInstance } from 'fastify';
import { nanoid } from 'nanoid';
import { eq } from 'drizzle-orm';
import { generateNodeToken } from '../services/nodeToken.js';
import { TOKEN_CACHE } from '../middleware/auth.js';
import { users, hosts, auditLogs } from '../db/schema.js';

export async function hostRoutes(app: FastifyInstance) {
    app.post('/api/hosts/register', async (req, reply) => {
        const { discordId, name, username } = req.body as any;

        // users upsert
        await app.db.insert(users)
            .values({ discordId, username: username ?? discordId })
            .onConflictDoUpdate({
                target: users.discordId,
                set: { username: username ?? discordId, lastSeen: new Date() },
            });

        // 이미 방장 등록 확인
        const existing = await app.db.query.hosts.findFirst({
            where: eq(hosts.discordId, discordId),
            columns: { id: true },
        });
        if (existing) {
            return reply.code(409).send({
                error: 'ALREADY_REGISTERED',
                hostId: existing.id,
            });
        }

        const hostId = nanoid(12);
        const { raw, hash } = await generateNodeToken(hostId);

        await app.db.insert(hosts)
            .values({ id: hostId, discordId, name, tokenHash: hash });

        await app.db.insert(auditLogs)
            .values({
                userDiscordId: discordId,
                hostId,
                action: 'register',
                detail: { name },
            });

        return {
            hostId,
            token: raw,
            warning: '이 토큰은 다시 표시되지 않습니다.',
        };
    });

    app.post('/api/hosts/:hostId/rotate-token', async (req, reply) => {
        const { hostId } = req.params as any;
        const { discordId } = req.body as any;

        const host = await app.db.query.hosts.findFirst({
            where: eq(hosts.id, hostId),
            columns: { discordId: true },
        });
        if (!host || host.discordId !== discordId) {
            return reply.code(403).send({ error: 'FORBIDDEN' });
        }

        const { raw, hash } = await generateNodeToken(hostId);
        await app.db.update(hosts)
            .set({ tokenHash: hash, updatedAt: new Date() })
            .where(eq(hosts.id, hostId));

        TOKEN_CACHE.delete(hostId);

        await app.db.insert(auditLogs)
            .values({ userDiscordId: discordId, hostId, action: 'rotate_token' });

        return {
            hostId,
            token: raw,
            warning: '이 토큰은 다시 표시되지 않습니다.',
        };
    });

    app.get('/api/hosts/:hostId', async (req) => {
        const { hostId } = req.params as any;
        const host = await app.db.query.hosts.findFirst({
            where: eq(hosts.id, hostId),
            columns: {
                id: true, name: true, status: true,
                lastHeartbeat: true, agentVersion: true, createdAt: true,
            },
        });
        return host ?? { error: 'NOT_FOUND' };
    });

    // ★ 신규: 메타데이터 조회 (클라우드 모드 봇용)
    app.get('/api/hosts/:hostId/metadata', async (req, reply) => {
        const { hostId } = req.params as any;
        const host = await app.db.query.hosts.findFirst({
            where: eq(hosts.id, hostId),
            columns: { metadata: true },
        });
        if (!host) return reply.code(404).send({ error: 'NOT_FOUND' });
        return host.metadata ?? {};
    });
}
```

### Task 4-2: 명령어 큐 라우트

생성할 파일:

- `relay-server/src/routes/command.ts`

엔드포인트:

- `POST /api/command` — Discord 봇이 호출.
  - body: `{ hostId, userDiscordId, guildId?, memberRoleIds?, payload, interactionToken?, channelId? }`
  - payload는 JSONB 그대로 저장, 해석하지 않음.
  - ACL 체크 → 호스트 online 확인 → command_queue INSERT → pollWaiters.wake.
  - expires_at = now + 60초.
  - 응답: `{ requestId, status: 'queued' }`.

```typescript
import { FastifyInstance } from 'fastify';
import { nanoid } from 'nanoid';
import { eq } from 'drizzle-orm';
import { AclService, PermLevel } from '../services/acl.js';
import { hosts, commandQueue, auditLogs } from '../db/schema.js';

export async function commandRoutes(app: FastifyInstance) {
    const acl = new AclService(app.db);

    app.post('/api/command', async (req, reply) => {
        const body = req.body as {
            hostId: string;
            userDiscordId: string;
            guildId?: string;
            memberRoleIds?: string[];
            payload: Record<string, unknown>;
            interactionToken?: string;
            channelId?: string;
        };

        // ACL
        const level = await acl.resolve({
            userDiscordId: body.userDiscordId,
            hostId: body.hostId,
            guildId: body.guildId,
            memberRoleIds: body.memberRoleIds,
        });
        if (level === PermLevel.NONE) {
            return reply.code(403).send({ error: 'FORBIDDEN' });
        }

        // 호스트 온라인 확인
        const host = await app.db.query.hosts.findFirst({
            where: eq(hosts.id, body.hostId),
            columns: { status: true },
        });
        if (!host) {
            return reply.code(404).send({ error: 'HOST_NOT_FOUND' });
        }
        if (host.status !== 'online') {
            return reply.code(503).send({ error: 'HOST_OFFLINE' });
        }

        // 큐 INSERT
        const id = nanoid();
        const now = new Date();
        const expiresAt = new Date(now.getTime() + 60_000);

        await app.db.insert(commandQueue).values({
            id,
            hostId: body.hostId,
            payload: body.payload,
            requestedBy: body.userDiscordId,
            guildId: body.guildId ?? null,
            channelId: body.channelId ?? null,
            interactionToken: body.interactionToken ?? null,
            status: 'pending',
            createdAt: now,
            expiresAt,
        });

        // 감사 로그
        await app.db.insert(auditLogs).values({
            userDiscordId: body.userDiscordId,
            hostId: body.hostId,
            guildId: body.guildId ?? null,
            action: 'command',
            detail: { requestId: id },
        });

        // poll 대기 깨우기
        app.pollWaiters.wake(body.hostId);

        return { requestId: id, status: 'queued' };
    });
}
```

### Task 4-3: Poll 라우트

생성할 파일:

- `relay-server/src/routes/poll.ts`

엔드포인트:

- `GET /poll` — 노드 에이전트가 호출.
  - preHandler: authenticateNode.
  - hostId는 인증 토큰에서 추출 (`req.node.id`), URL 파라미터에 넣지 않음.
  - pending 명령 조회 (최대 10개, created_at ASC).
  - 있으면 즉시 응답 + status를 delivered로 변경.
  - 없으면 pollWaiters.wait(25초) 후 재확인.
  - 그래도 없으면 204.

```typescript
import { FastifyInstance } from 'fastify';
import { eq, and, gt, asc, inArray } from 'drizzle-orm';
import { commandQueue } from '../db/schema.js';

const POLL_TIMEOUT = 25000;

export async function pollRoutes(app: FastifyInstance) {
    app.get('/poll', {
        preHandler: [app.authenticateNode],
    }, async (req, reply) => {
        const hostId = (req as any).node.id;
        const now = new Date();

        const fetchPending = () => app.db
            .select({ id: commandQueue.id, payload: commandQueue.payload })
            .from(commandQueue)
            .where(and(
                eq(commandQueue.hostId, hostId),
                eq(commandQueue.status, 'pending'),
                gt(commandQueue.expiresAt, now),
            ))
            .orderBy(asc(commandQueue.createdAt))
            .limit(10);

        const markDelivered = async (ids: string[]) => {
            if (ids.length === 0) return;
            await app.db.update(commandQueue)
                .set({ status: 'delivered', deliveredAt: new Date() })
                .where(inArray(commandQueue.id, ids));
        };

        // 즉시 확인
        let pending = await fetchPending();
        if (pending.length > 0) {
            await markDelivered(pending.map(c => c.id));
            return { commands: pending };
        }

        // 대기
        try {
            await app.pollWaiters.wait(hostId, POLL_TIMEOUT);
        } catch {
            // 타임아웃 → 정상
        }

        // 재확인
        pending = await fetchPending();
        if (pending.length > 0) {
            await markDelivered(pending.map(c => c.id));
            return { commands: pending };
        }

        return reply.code(204).send();
    });
}
```

### Task 4-4: Result 라우트

생성할 파일:

- `relay-server/src/routes/result.ts`

엔드포인트:

- `POST /result/:requestId` — 노드 에이전트가 호출.
  - preHandler: authenticateNode.
  - body: `{ success, data }`.
  - command_queue를 completed로 업데이트.
  - host_id 일치 확인 (다른 노드의 결과 반환 방지).
  - 이미 completed면 409.
  - interaction_token이 있으면 Discord webhook으로 메시지 편집 (PATCH).

```typescript
import { FastifyInstance } from 'fastify';
import { eq } from 'drizzle-orm';
import { commandQueue } from '../db/schema.js';

export async function resultRoutes(app: FastifyInstance) {
    app.post<{
        Params: { requestId: string };
    }>('/result/:requestId', {
        preHandler: [app.authenticateNode],
    }, async (req, reply) => {
        const { requestId } = req.params;
        const body = req.body as { success: boolean; data: any };
        const hostId = (req as any).node.id;

        // 큐 조회
        const cmd = await app.db.query.commandQueue.findFirst({
            where: eq(commandQueue.id, requestId),
            columns: {
                hostId: true,
                interactionToken: true,
                status: true,
            },
        });

        if (!cmd || cmd.hostId !== hostId) {
            return reply.code(404).send({ error: 'NOT_FOUND' });
        }
        if (cmd.status === 'completed') {
            return reply.code(409).send({ error: 'ALREADY_COMPLETED' });
        }

        // 상태 업데이트
        await app.db.update(commandQueue)
            .set({
                status: 'completed',
                completedAt: new Date(),
                result: body,
            })
            .where(eq(commandQueue.id, requestId));

        // Discord followup
        if (cmd.interactionToken && app.discordAppId) {
            try {
                const message = body.success
                    ? `✅ ${body.data?.message ?? JSON.stringify(body.data).slice(0, 1900)}`
                    : `❌ ${body.data?.error ?? '실패'}`;

                await fetch(
                    `https://discord.com/api/v10/webhooks/${app.discordAppId}/${cmd.interactionToken}/messages/@original`,
                    {
                        method: 'PATCH',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({
                            content: message.slice(0, 2000),
                        }),
                    },
                );
            } catch (e: any) {
                app.log.warn(`Discord followup failed: ${e.message}`);
            }
        }

        return { status: 'ok' };
    });
}
```

### Task 4-5: Heartbeat 라우트

생성할 파일:

- `relay-server/src/routes/heartbeat.ts`

엔드포인트:

- `POST /heartbeat` — 노드 에이전트가 30초마다 호출.
  - preHandler: authenticateNode.
  - body: `{ agentVersion?, os?, metadata? }`.
  - hosts 테이블의 status, last_heartbeat, agent_version, **metadata** 갱신.
  - ★ `metadata`에 모듈 목록, 서버 목록, 명령어 정의가 포함됨. 클라우드 봇의 resolver가 이 데이터를 사용.
  - agentVersion이 minAgentVersion 미만이면 응답에 `{ warning: 'UPDATE_REQUIRED', minVersion }` 포함.

```typescript
import { FastifyInstance } from 'fastify';
import { eq } from 'drizzle-orm';
import { hosts } from '../db/schema.js';

const MIN_AGENT_VERSION = '2.0.0';

export async function heartbeatRoutes(app: FastifyInstance) {
    app.post('/heartbeat', {
        preHandler: [app.authenticateNode],
    }, async (req) => {
        const hostId = (req as any).node.id;
        const body = req.body as {
            agentVersion?: string;
            os?: string;
            metadata?: {
                modules: string[];
                servers: Array<{ id: string; name: string; module: string; status: string }>;
                moduleDetails: Record<string, any>;
                botConfig?: { prefix: string; moduleAliases: Record<string, string> };
            };
        };

        const updateData: Record<string, any> = {
            status: 'online',
            lastHeartbeat: new Date(),
            agentVersion: body.agentVersion ?? null,
            updatedAt: new Date(),
        };

        // ★ 메타데이터 동기화: 노드가 보내면 DB에 캐시
        if (body.metadata) {
            updateData.metadata = body.metadata;
        }

        await app.db.update(hosts)
            .set(updateData)
            .where(eq(hosts.id, hostId));

        const response: any = { status: 'ok' };

        if (body.agentVersion && body.agentVersion < MIN_AGENT_VERSION) {
            response.warning = 'UPDATE_REQUIRED';
            response.minVersion = MIN_AGENT_VERSION;
        }

        return response;
    });
}
```

### Task 4-6: 라우트 등록 통합

수정할 파일:

- `relay-server/src/index.ts`

주석 처리된 라우트 등록을 활성화:

```typescript
import { authenticateNode } from './middleware/auth.js';
import { hostRoutes } from './routes/host.js';
import { commandRoutes } from './routes/command.js';
import { pollRoutes } from './routes/poll.js';
import { resultRoutes } from './routes/result.js';
import { heartbeatRoutes } from './routes/heartbeat.js';
import { scheduleCleanup } from './services/cleanup.js';

app.decorate('authenticateNode', authenticateNode);

await app.register(hostRoutes);
await app.register(commandRoutes);
await app.register(pollRoutes);
await app.register(resultRoutes);
await app.register(heartbeatRoutes);

scheduleCleanup(db);
```

---

## Phase 5: node-agent (메타데이터 동기화 추가)

### Task 5-1: 프로젝트 스캐폴딩

`node-agent/` 디렉토리 생성.

생성할 파일:

- `node-agent/package.json`
- `node-agent/tsconfig.json`
- `node-agent/src/config.ts`

`package.json` — 의존성 없음, Node.js 내장 `crypto`, `fetch`만 사용:

```json
{
  "name": "saba-chan-node-agent",
  "version": "2.0.0",
  "type": "module",
  "dependencies": {},
  "devDependencies": {
    "typescript": "^5",
    "@types/node": "^22",
    "tsx": "^4"
  },
  "scripts": {
    "dev": "tsx watch src/index.ts",
    "build": "tsc",
    "start": "node dist/index.js"
  }
}
```

`tsconfig.json`:

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "outDir": "dist",
    "rootDir": "src",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "declaration": true
  },
  "include": ["src"]
}
```

`src/config.ts`:

```typescript
import { readFileSync, existsSync } from 'fs';

export interface AgentConfig {
    serverUrl: string;
    nodeToken: string;
    daemonBase: string;
}

export function loadConfig(): AgentConfig {
    // 환경변수 우선
    if (process.env.RELAY_SERVER_URL && process.env.NODE_TOKEN) {
        return {
            serverUrl: process.env.RELAY_SERVER_URL,
            nodeToken: process.env.NODE_TOKEN,
            daemonBase: process.env.DAEMON_BASE ?? 'http://127.0.0.1:57474',
        };
    }

    // agent.json 폴백
    const configPath = process.env.AGENT_CONFIG ?? './agent.json';
    if (!existsSync(configPath)) {
        console.error(`설정 파일이 없습니다: ${configPath}`);
        process.exit(1);
    }

    const raw = JSON.parse(readFileSync(configPath, 'utf-8'));
    return {
        serverUrl: raw.serverUrl ?? raw.server_url,
        nodeToken: raw.nodeToken ?? raw.node_token,
        daemonBase: raw.daemonBase ?? raw.daemon_base ?? 'http://127.0.0.1:57474',
    };
}
```

### Task 5-2: 인증 요청 헬퍼

생성할 파일:

- `node-agent/src/auth.ts`

기능:

- `signRequest({ method, path, body?, secret, timestamp })` → HMAC-SHA256 hex string.
- `authenticatedFetch(url, nodeToken, options?)` → fetch with Authorization, X-Request-Timestamp, X-Request-Signature headers.
- secret은 토큰의 `.` 이후 부분.

```typescript
import { createHmac } from 'crypto';

export function signRequest(p: {
    method: string;
    path: string;
    body?: string;
    secret: string;
    timestamp: number;
}): string {
    const payload = [
        p.method.toUpperCase(),
        p.path,
        p.timestamp.toString(),
        p.body ?? '',
    ].join('\n');
    return createHmac('sha256', p.secret).update(payload).digest('hex');
}

export async function authenticatedFetch(
    url: string,
    nodeToken: string,
    options: RequestInit = {},
): Promise<Response> {
    const parsed = new URL(url);
    const timestamp = Math.floor(Date.now() / 1000);
    const body = options.body as string | undefined;

    // sbn_{nodeId}.{secret} → secret 부분 추출
    const dotIndex = nodeToken.indexOf('.');
    const secret = dotIndex >= 0 ? nodeToken.slice(dotIndex + 1) : nodeToken;

    const sig = signRequest({
        method: options.method ?? 'GET',
        path: parsed.pathname,
        body,
        secret,
        timestamp,
    });

    return fetch(url, {
        ...options,
        headers: {
            ...(options.headers as Record<string, string> ?? {}),
            'Authorization': `Bearer ${nodeToken}`,
            'X-Request-Timestamp': timestamp.toString(),
            'X-Request-Signature': sig,
            ...(body ? { 'Content-Type': 'application/json' } : {}),
        },
    });
}
```

### Task 5-3: Poller

생성할 파일:

- `node-agent/src/poller.ts`

기능:

- 무한 루프: `GET {serverUrl}/poll` (timeout 30초).
- 204 → 즉시 재루프.
- 200 → commands 배열 순차 실행. 각 payload를 로컬 데몬에 POST. 결과를 `POST {serverUrl}/result/{id}`로 보고.
- 네트워크 에러 시 3초 대기 후 재시도.
- payload에 instance_id가 있으면 `/api/instance/{id}/command`, 없으면 `/api/command`.

```typescript
import { authenticatedFetch } from './auth.js';

export class Poller {
    private running = false;

    constructor(
        private serverUrl: string,
        private nodeToken: string,
        private daemonBase: string,
    ) {}

    async start() {
        this.running = true;
        while (this.running) {
            try {
                await this.pollOnce();
            } catch (e: any) {
                console.error(`[Poll] ${e.message}`);
                await sleep(3000);
            }
        }
    }

    stop() {
        this.running = false;
    }

    private async pollOnce() {
        const res = await authenticatedFetch(
            `${this.serverUrl}/poll`,
            this.nodeToken,
            { signal: AbortSignal.timeout(30000) },
        );

        if (res.status === 204) return;
        if (!res.ok) throw new Error(`Poll ${res.status}`);

        const { commands } = await res.json() as {
            commands: { id: string; payload: string }[];
        };

        for (const cmd of commands) {
            await this.executeAndReport(cmd);
        }
    }

    private async executeAndReport(cmd: { id: string; payload: string }) {
        let result: any;
        let success = false;

        try {
            const payload = JSON.parse(cmd.payload);
            const endpoint = payload.instance_id
                ? `${this.daemonBase}/api/instance/${payload.instance_id}/command`
                : `${this.daemonBase}/api/command`;

            const res = await fetch(endpoint, {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: cmd.payload,
            });
            result = await res.json();
            success = res.ok;
        } catch (e: any) {
            result = { error: e.message };
        }

        await authenticatedFetch(
            `${this.serverUrl}/result/${cmd.id}`,
            this.nodeToken,
            {
                method: 'POST',
                body: JSON.stringify({ success, data: result }),
            },
        );
    }
}

function sleep(ms: number) {
    return new Promise(r => setTimeout(r, ms));
}
```

### Task 5-4: Heartbeat 서비스 (★ 메타데이터 동기화)

생성할 파일:

- `node-agent/src/heartbeat.ts`

기능:

- 30초 interval, 즉시 1회 실행.
- `POST {serverUrl}/heartbeat` with `{ agentVersion, metadata }`.
- ★ **메타데이터 동기화**: 로컬 사바쨩 데몬에서 모듈/서버/명령어 정보를 수집하여 릴레이 서버에 전송.
- 클라우드 모드 Discord 봇의 `resolver.js`가 이 데이터를 사용하여 별명 해석, 도움말 생성 등을 수행.
- 응답에 `warning: 'UPDATE_REQUIRED'`가 있으면 콘솔 경고.

```typescript
import { authenticatedFetch } from './auth.js';
import { readFileSync } from 'fs';

const INTERVAL = 30000;

export class HeartbeatService {
    private timer?: NodeJS.Timeout;
    private agentVersion: string;

    constructor(
        private serverUrl: string,
        private nodeToken: string,
        private daemonBase: string,
    ) {
        try {
            this.agentVersion = JSON.parse(
                readFileSync(
                    new URL('../package.json', import.meta.url),
                    'utf-8',
                ),
            ).version;
        } catch {
            this.agentVersion = 'unknown';
        }
    }

    start() {
        this.tick();
        this.timer = setInterval(() => this.tick(), INTERVAL);
    }

    stop() {
        if (this.timer) clearInterval(this.timer);
    }

    /**
     * ★ 로컬 사바쨩 데몬에서 메타데이터 수집.
     * 클라우드 봇의 resolver가 이 데이터를 사용:
     *   - modules: 설치된 모듈 목록
     *   - servers: 인스턴스 목록 (id, name, module, status)
     *   - moduleDetails: 각 모듈의 toml 정보 (명령어 정의 포함)
     *   - botConfig: bot-config.json의 prefix, moduleAliases, commandAliases
     */
    private async collectMetadata(): Promise<Record<string, any> | null> {
        try {
            // 서버(인스턴스) 목록
            const serversRes = await fetch(`${this.daemonBase}/api/servers`);
            const { servers } = await serversRes.json() as any;

            // 모듈 목록
            const modulesRes = await fetch(`${this.daemonBase}/api/modules`);
            const { modules } = await modulesRes.json() as any;

            // 각 모듈 상세 (명령어 정의)
            const moduleDetails: Record<string, any> = {};
            for (const mod of modules) {
                try {
                    const detailRes = await fetch(`${this.daemonBase}/api/module/${mod}`);
                    const { toml } = await detailRes.json() as any;
                    moduleDetails[mod] = toml;
                } catch { /* 개별 모듈 실패는 무시 */ }
            }

            return { modules, servers, moduleDetails };
        } catch (e: any) {
            console.warn(`[Heartbeat] 메타데이터 수집 실패: ${e.message}`);
            return null;
        }
    }

    private async tick() {
        try {
            const metadata = await this.collectMetadata();

            const res = await authenticatedFetch(
                `${this.serverUrl}/heartbeat`,
                this.nodeToken,
                {
                    method: 'POST',
                    body: JSON.stringify({
                        agentVersion: this.agentVersion,
                        ...(metadata ? { metadata } : {}),
                    }),
                },
            );
            const data = await res.json() as any;
            if (data.warning === 'UPDATE_REQUIRED') {
                console.warn(
                    `[Agent] 에이전트 업데이트 필요. 최소 버전: ${data.minVersion}`,
                );
            }
        } catch (e: any) {
            console.error(`[Heartbeat] ${e.message}`);
        }
    }
}
```

### Task 5-5: 에이전트 메인

생성할 파일:

- `node-agent/src/index.ts`

기능:

- loadConfig → 서버 /info 호출 (연결 확인 + minAgentVersion) → HeartbeatService 시작 → Poller 시작 (무한 루프).
- SIGINT/SIGTERM 시 graceful shutdown.

```typescript
import { loadConfig } from './config.js';
import { Poller } from './poller.js';
import { HeartbeatService } from './heartbeat.js';

async function main() {
    const config = loadConfig();
    console.log(`[Agent] 서버: ${config.serverUrl}`);
    console.log(`[Agent] 데몬: ${config.daemonBase}`);

    // 서버 연결 확인
    const infoRes = await fetch(`${config.serverUrl}/info`);
    if (!infoRes.ok) {
        console.error('[Agent] 서버 연결 실패');
        process.exit(1);
    }
    const info = await infoRes.json() as { minAgentVersion: string };
    console.log(`[Agent] 서버 연결 확인. 최소 에이전트 버전: ${info.minAgentVersion}`);

    // ★ HeartbeatService에 daemonBase 전달 (메타데이터 수집용)
    const heartbeat = new HeartbeatService(config.serverUrl, config.nodeToken, config.daemonBase);
    const poller = new Poller(config.serverUrl, config.nodeToken, config.daemonBase);

    heartbeat.start();

    const shutdown = () => {
        console.log('[Agent] 종료 중...');
        poller.stop();
        heartbeat.stop();
        process.exit(0);
    };
    process.on('SIGINT', shutdown);
    process.on('SIGTERM', shutdown);

    await poller.start(); // 무한 루프
}

main().catch((e) => {
    console.error(e);
    process.exit(1);
});
```

> 💡 **향후 고려**: node-agent를 saba-core Rust 바이너리에 내장하는 방안. `src/relay_client/mod.rs`로 구현하면 localhost HTTP 호출 없이 내부 API를 직접 사용 가능. 현재는 별도 TS 프로젝트로 시작하고, 안정화 후 Rust 통합 검토.

---

## Phase 6: Discord 봇 하이브리드 모드 (5모듈 아키텍처 기반)

> **v2 핵심 변경**: 구 청사진은 모놀리식 index.js 1곳만 분기했으나, 현재 봇은 5개 모듈로 분리됨.
> 변경이 필요한 모듈과 불필요한 모듈이 명확히 구분됨.

### 모듈별 클라우드 모드 영향 분석

| 모듈 | 변경 필요 | 이유 |
|------|-----------|------|
| `core/ipc.js` | ✅ **핵심 분기점** | 모든 데몬 API 호출의 관문. 클라우드 모드에서 릴레이 API로 전환 |
| `core/handler.js` | ✅ 필터링 추가 | 음악 익스텐션 스킵 (로컬 전용) |
| `core/resolver.js` | ✅ 메타데이터 소스 변경 | 로컬 IPC 대신 릴레이 API에서 메타데이터 로드 |
| `index.js` | ✅ 모드 초기화 | GuildVoiceStates 제거, 모드 전달 |
| `core/processor.js` | ❌ **변경 불필요** | ipc/resolver/handler 추상화만 사용, 모드 무관 |
| `extensions/music.js` | ❌ 변경 불필요 | 로컬 전용. handler.js에서 스킵 |
| `extensions/easter_eggs.js` | ❌ 변경 불필요 | IPC 미사용, 양쪽 모드 동작 |
| `extensions/rps.js` | ❌ 변경 불필요 | IPC 미사용, 양쪽 모드 동작 |

> 💡 `processor.js`가 변경 불필요한 것이 5모듈 아키텍처의 핵심 이점. 명령어 흐름의 비즈니스 로직이 인프라(IPC 전송 방식)와 완전히 분리됨.

### Task 6-1: 모드 설정 추가

수정할 파일:

- `config/global.toml`
- `discord_bot/bot-config.json`

`config/global.toml` 추가:

```toml
[discord]
mode = "local"  # "local" 또는 "cloud"
token = ""

[discord.cloud]
relay_url = ""
host_id = ""
node_token = ""   # 슬래시 커맨드에서 릴레이 API 직접 호출 시 사용
```

`discord_bot/bot-config.json`에 모드 필드 추가:

```json
{
  "prefix": "사바쨩",
  "mode": "local",
  "cloud": {
    "relayUrl": "",
    "hostId": ""
  },
  "moduleAliases": { ... },
  "commandAliases": { ... }
}
```

환경변수로도 오버라이드 가능:
- `BOT_MODE=cloud`
- `RELAY_URL=https://saba-relay.example.com`
- `HOST_ID=abc123`

### Task 6-2: `core/ipc.js` — 클라우드 트랜스포트 레이어

수정할 파일:

- `discord_bot/core/ipc.js`

**변경 전략**: ipc.js가 외부에 노출하는 API(`getServers`, `startServer`, `sendRcon` 등)의 **시그니처는 유지**하되, 내부에서 모드에 따라 전송 경로를 분기.

- `local` 모드: 기존 그대로 `axios.get/post(IPC_BASE + ...)`.
- `cloud` 모드: `axios.post(RELAY_URL + '/api/command', { hostId, payload })` + 응답 대기.

processor.js는 `ipc.getServers()`, `ipc.sendRcon()` 등만 호출하므로 **일체 수정 불필요**.

```javascript
// discord_bot/core/ipc.js (클라우드 모드 추가)

const axios = require('axios');
const fs = require('fs');
const path = require('path');
const i18n = require('../i18n');

const IPC_BASE = process.env.IPC_BASE || 'http://127.0.0.1:57474';

// ── 모드 설정 ──
let _mode = process.env.BOT_MODE || 'local';     // 'local' | 'cloud'
let _relayUrl = process.env.RELAY_URL || '';
let _hostId = process.env.HOST_ID || '';
let _cachedMetadata = null;                        // 클라우드 모드 메타데이터 캐시

// ... (기존 토큰 관리 코드 유지)

// ── 모드 설정 주입 ──
function setMode(mode, options = {}) {
    _mode = mode;
    if (options.relayUrl) _relayUrl = options.relayUrl;
    if (options.hostId) _hostId = options.hostId;
    console.log(`[IPC] Mode: ${_mode}` + (_mode === 'cloud' ? ` → ${_relayUrl}` : ''));
}

function getMode() { return _mode; }

// ── 클라우드 모드: 릴레이를 통한 명령 전송 ──

/**
 * 릴레이 서버에 명령을 전송하고 결과를 폴링합니다.
 * 릴레이가 결과를 돌려주는 방식: command → queue → node-agent → daemon → result
 * 봇은 requestId를 받고, 결과가 올 때까지 짧은 폴링.
 */
async function relayCommand(payload, message) {
    const res = await axios.post(`${_relayUrl}/api/command`, {
        hostId: _hostId,
        userDiscordId: message?.author?.id ?? 'system',
        guildId: message?.guildId ?? null,
        memberRoleIds: message?.member?.roles?.cache?.map(r => r.id) ?? [],
        payload,
        channelId: message?.channelId ?? null,
    });

    const { requestId } = res.data;

    // 결과 폴링 (최대 30초, 1초 간격)
    for (let i = 0; i < 30; i++) {
        await new Promise(r => setTimeout(r, 1000));
        try {
            const resultRes = await axios.get(`${_relayUrl}/api/command/${requestId}/status`);
            if (resultRes.data.status === 'completed') {
                return resultRes.data.result;
            }
        } catch { /* 아직 완료되지 않음 */ }
    }

    throw new Error(i18n.t('bot:errors.timeout'));
}

// ── API 래퍼 (모드 분기) ──

async function getServers() {
    if (_mode === 'cloud') {
        // 클라우드: 릴레이에 캐시된 메타데이터에서 가져오기
        const metadata = await getCloudMetadata();
        return metadata?.servers || [];
    }
    const res = await axios.get(`${IPC_BASE}/api/servers`);
    return res.data.servers || [];
}

async function getModules() {
    if (_mode === 'cloud') {
        const metadata = await getCloudMetadata();
        return metadata?.modules || [];
    }
    const res = await axios.get(`${IPC_BASE}/api/modules`);
    return res.data.modules || [];
}

async function getModuleDetail(moduleName) {
    if (_mode === 'cloud') {
        const metadata = await getCloudMetadata();
        return metadata?.moduleDetails?.[moduleName] || {};
    }
    const res = await axios.get(`${IPC_BASE}/api/module/${moduleName}`);
    return res.data.toml || {};
}

async function startServer(serverId, serverName, serverModule, useManaged) {
    if (_mode === 'cloud') {
        return relayCommand({
            action: 'start',
            instance_id: serverId,
            server_name: serverName,
            module: serverModule,
            managed: useManaged,
        });
    }
    if (useManaged) {
        return axios.post(`${IPC_BASE}/api/instance/${serverId}/managed/start`, {});
    }
    return axios.post(`${IPC_BASE}/api/server/${serverName}/start`, {
        module: serverModule, config: {},
    });
}

async function stopServer(serverName) {
    if (_mode === 'cloud') {
        return relayCommand({ action: 'stop', server_name: serverName });
    }
    return axios.post(`${IPC_BASE}/api/server/${serverName}/stop`, { force: false });
}

async function sendStdin(serverId, command) {
    if (_mode === 'cloud') {
        return relayCommand({ action: 'stdin', instance_id: serverId, command });
    }
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/stdin`, { command });
}

async function sendRcon(serverId, command) {
    if (_mode === 'cloud') {
        return relayCommand({ action: 'rcon', instance_id: serverId, command });
    }
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/rcon`, {
        command, instance_id: serverId,
    });
}

async function sendRestCommand(serverId, endpoint, httpMethod, body, serverOpts) {
    if (_mode === 'cloud') {
        return relayCommand({
            action: 'rest', instance_id: serverId,
            endpoint, method: httpMethod, body,
            rest_host: serverOpts.rest_host, rest_port: serverOpts.rest_port,
            username: serverOpts.rest_username, password: serverOpts.rest_password,
        });
    }
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/rest`, {
        endpoint, method: httpMethod, body,
        instance_id: serverId,
        rest_host: serverOpts.rest_host || '127.0.0.1',
        rest_port: serverOpts.rest_port || 8212,
        username: serverOpts.rest_username || 'admin',
        password: serverOpts.rest_password || '',
    });
}

async function sendModuleCommand(serverId, commandName, body) {
    if (_mode === 'cloud') {
        return relayCommand({
            action: 'module_command', instance_id: serverId,
            command: commandName, args: body,
        });
    }
    return axios.post(`${IPC_BASE}/api/instance/${serverId}/command`, {
        command: commandName, args: body, instance_id: serverId,
    });
}

// ── 클라우드 메타데이터 캐시 ──

async function getCloudMetadata() {
    if (_cachedMetadata && Date.now() - _cachedMetadata._fetchedAt < 30000) {
        return _cachedMetadata;
    }
    try {
        const res = await axios.get(`${_relayUrl}/api/hosts/${_hostId}/metadata`);
        _cachedMetadata = { ...res.data, _fetchedAt: Date.now() };
        return _cachedMetadata;
    } catch (e) {
        console.warn('[IPC] Cloud metadata fetch failed:', e.message);
        return _cachedMetadata; // 이전 캐시 사용
    }
}

module.exports = {
    init,
    setMode,
    getMode,
    getServers,
    getModules,
    getModuleDetail,
    startServer,
    stopServer,
    sendStdin,
    sendRcon,
    sendRestCommand,
    sendModuleCommand,
    formatResponse,
    getCloudMetadata,
};
```

### Task 6-3: `core/handler.js` — 모드 인식 익스텐션 필터링

수정할 파일:

- `discord_bot/core/handler.js`

**변경**: 클라우드 모드에서 음악 익스텐션(Music, Music:Shortcut)을 스킵.
Voice 연결이 릴레이를 통해 불가능하므로, cloud 모드에서는 아예 시도하지 않음.

```javascript
const musicExtension = require('../extensions/music');
const easterEggsExtension = require('../extensions/easter_eggs');
const rpsExtension = require('../extensions/rps');
const ipc = require('./ipc');   // ★ 모드 조회용

const extensions = [
    {
        name: 'Music:Shortcut',
        localOnly: true,   // ★ 로컬 전용 마커
        handler: (msg, args, cfg) => musicExtension.handleMusicShortcut(msg, args, cfg),
    },
    {
        name: 'Music',
        localOnly: true,   // ★ 로컬 전용 마커
        handler: (msg, args, cfg) => musicExtension.handleMusicMessage(msg, args, cfg),
    },
    {
        name: 'EasterEgg',
        localOnly: false,
        handler: (msg, args, _cfg) => easterEggsExtension.handleEasterEgg(msg, args),
    },
    {
        name: 'RPS',
        localOnly: false,
        handler: (msg, args, _cfg) => rpsExtension.handleRPS(msg, args),
    },
];

async function handle(message, args, botConfig) {
    const mode = ipc.getMode();

    for (const ext of extensions) {
        // ★ 클라우드 모드에서 로컬 전용 익스텐션 스킵
        if (ext.localOnly && mode === 'cloud') continue;

        try {
            const handled = await ext.handler(message, args, botConfig);
            if (handled) return true;
        } catch (e) {
            console.error(`[${ext.name}] Extension error:`, e.message);
        }
    }
    return false;
}

module.exports = { handle };
```

### Task 6-4: `core/resolver.js` — 클라우드 메타데이터 로드

수정할 파일:

- `discord_bot/core/resolver.js`

**변경**: `loadModuleMetadata()`가 모드에 따라 데이터 소스를 분기.
- `local`: 기존 `ipc.getModules()` + `ipc.getModuleDetail()`.
- `cloud`: 릴레이에 캐시된 메타데이터에서 로드 (node-agent가 heartbeat으로 동기화).

`ipc.js`의 `getModules()`/`getModuleDetail()`가 이미 내부적으로 mode 분기하므로, **resolver.js 코드 변경은 최소**.
단, bot-config.json 로드에 cloud 설정을 반영.

```javascript
// resolver.js 수정 부분 (init 함수)

async function init() {
    console.log('[Resolver] Config path:', configPath);
    loadConfig();

    // ★ 클라우드 모드 설정 반영
    if (botConfig.mode === 'cloud' && botConfig.cloud) {
        const ipc = require('./ipc');
        ipc.setMode('cloud', {
            relayUrl: botConfig.cloud.relayUrl || process.env.RELAY_URL,
            hostId: botConfig.cloud.hostId || process.env.HOST_ID,
        });
    }

    console.log('[Resolver] Loading module metadata…');
    await loadModuleMetadata();
    // ipc.getModules() / ipc.getModuleDetail()이 이미 모드별로 분기하므로
    // loadModuleMetadata() 함수 자체는 변경 불필요

    const ma = getModuleAliases();
    const ca = getCommandAliases();
    console.log(`[Resolver] Module aliases: ${JSON.stringify(ma)}`);
    console.log(`[Resolver] Command aliases: ${JSON.stringify(ca)}`);
}
```

### Task 6-5: `index.js` — 모드 초기화

수정할 파일:

- `discord_bot/index.js`

**변경**:
1. 클라우드 모드에서 `GuildVoiceStates` 인텐트 제거 (음악 미사용).
2. 모드 로그 출력.

```javascript
const { Client, GatewayIntentBits } = require('discord.js');
const ipc = require('./core/ipc');
const resolver = require('./core/resolver');
const processor = require('./core/processor');

// ── 모드 결정 ──
const botMode = process.env.BOT_MODE || 'local';

// ── Discord 클라이언트 ──
const intents = [
    GatewayIntentBits.Guilds,
    GatewayIntentBits.GuildMessages,
    GatewayIntentBits.MessageContent,
];

// ★ 로컬 모드에서만 Voice 인텐트 (음악 재생용)
if (botMode === 'local') {
    intents.push(GatewayIntentBits.GuildVoiceStates);
}

const client = new Client({ intents });

// ... (기존 에러 핸들링, 이벤트 등록 동일)

client.once('ready', async () => {
    console.log(`[Bot] Logged in as ${client.user.tag}`);
    console.log(`[Bot] Mode: ${botMode}`);   // ★ 모드 표시

    ipc.init();
    await resolver.init();   // resolver.init()이 모드 설정 적용

    const cfg = resolver.getConfig();
    console.log(`[Bot] Prefix: ${cfg.prefix}`);
    console.log('[Bot] Ready');
});

client.login(process.env.DISCORD_TOKEN);
```

### Task 6-6: GUI 모드 토글

수정할 파일:

- `saba-chan-gui/src/components/Modals/DiscordBotModal.jsx`

변경 내용:

기존 토큰 입력 UI에 "모드 선택" 라디오/토글 추가.
- `local` 선택 시: 기존 UI 그대로 (토큰 입력, 음악 토글).
- `cloud` 선택 시: relay_url, host_id 입력 필드 표시. 음악 토글 비활성화 (로컬 전용 안내 표시).

GUI에서 봇 프로세스 spawn 시 환경변수에 `BOT_MODE`, `RELAY_URL`, `HOST_ID` 주입:

```javascript
// saba-chan-gui/main.js — spawnBot() 내부
const botEnv = {
    DISCORD_TOKEN: token,
    BOT_MODE: config.mode || 'local',
};

if (config.mode === 'cloud') {
    botEnv.RELAY_URL = config.cloud?.relayUrl || '';
    botEnv.HOST_ID = config.cloud?.hostId || '';
}
```

### Task 6-7: CLI 모드 토글

수정할 파일:

- `saba-chan-cli/src/tui/commands.rs`

변경 내용:

`bot` 커맨드에 `--mode local|cloud` 플래그 추가. cloud 모드 시 환경변수 `BOT_MODE=cloud`, `RELAY_URL`, `HOST_ID`를 설정하여 discord_bot 프로세스를 시작.

### 데이터 흐름 요약

```
[ 로컬 모드 — 기존 그대로 ]
Discord → messageCreate → processor.process()
  → handler.handle() (Music ✅, EasterEgg, RPS)
  → ipc.getServers() → localhost:57474/api/servers
  → ipc.sendRcon()   → localhost:57474/api/instance/{id}/rcon

[ 클라우드 모드 — 릴레이 경유 ]
Discord → messageCreate → processor.process()
  → handler.handle() (Music ❌ 스킵, EasterEgg ✅, RPS ✅)
  → ipc.getServers() → RELAY_URL/api/hosts/{id}/metadata (캐시)
  → ipc.sendRcon()   → RELAY_URL/api/command → 큐 → node-agent → localhost:57474
```

---

## Phase 7: 길드 연동 Discord 명령어

### Task 7-1: 관리 슬래시 커맨드

수정할 파일:

- `discord_bot/index.js` (또는 별도 commands 파일)

cloud 모드에서만 활성화되는 슬래시 커맨드 추가:

| 커맨드 | 기능 | API 호출 |
|--------|------|----------|
| `/사바쨩 등록` | 방장 등록, DM으로 토큰 전달 | `POST /api/hosts/register` |
| `/사바쨩 연결` | 현재 길드를 방장의 노드에 연결 | guild_hosts INSERT |
| `/사바쨩 권한부여 @유저 [레벨]` | 유저에게 권한 부여 | permissions UPSERT |
| `/사바쨩 권한해제 @유저` | 유저 권한 해제 | permissions DELETE |
| `/사바쨩 역할권한 @역할 [레벨]` | Discord 역할에 권한 매핑 | role_permissions UPSERT |
| `/사바쨩 상태` | 노드 온라인/오프라인, last_heartbeat | `GET /api/hosts/:hostId` |

---

## Phase 8: 배포

### Task 8-1: Dockerfile

생성할 파일:

- `relay-server/Dockerfile`

```dockerfile
FROM node:22-alpine AS build
WORKDIR /app
COPY package*.json ./
RUN npm ci
COPY tsconfig.json ./
COPY drizzle.config.ts ./
COPY src ./src
RUN npm run build

FROM node:22-alpine
WORKDIR /app
COPY package*.json ./
RUN npm ci --omit=dev
COPY --from=build /app/dist ./dist
COPY drizzle ./drizzle
EXPOSE 3000
CMD ["node", "dist/index.js"]
```

### Task 8-2: docker-compose (★ PostgreSQL 추가)

생성할 파일:

- `relay-server/docker-compose.yml`

```yaml
services:
  postgres:
    image: postgres:17-alpine
    environment:
      POSTGRES_USER: saba
      POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:-saba_secret}
      POSTGRES_DB: saba_relay
    volumes:
      - pg-data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U saba -d saba_relay"]
      interval: 5s
      timeout: 5s
      retries: 5
    restart: unless-stopped

  relay:
    build: .
    ports:
      - "3000:3000"
    environment:
      - PORT=3000
      - DATABASE_URL=postgresql://saba:${POSTGRES_PASSWORD:-saba_secret}@postgres:5432/saba_relay
      - DISCORD_TOKEN=${DISCORD_TOKEN}
      - DISCORD_APP_ID=${DISCORD_APP_ID}
    depends_on:
      postgres:
        condition: service_healthy
    restart: unless-stopped

volumes:
  pg-data:
```

### Task 8-3: Cloudflare 설정 가이드

생성할 파일:

- `relay-server/DEPLOY.md`

내용:

1. `.env` 파일 작성 (DISCORD_TOKEN, DISCORD_APP_ID, POSTGRES_PASSWORD)
2. `docker compose up -d`로 PostgreSQL + 릴레이 서버 시작
3. `npm run db:migrate`로 스키마 적용 (첫 실행 시)
4. DNS A 레코드 → VPS IP (Cloudflare proxy 활성화)
5. SSL: Full (Strict)
6. VPS 방화벽: 포트 3000은 Cloudflare IP 대역만 허용
7. Cloudflare에서 `saba-relay.example.com` → VPS:3000 프록시

---

## 구현 순서 요약

```
Phase 1: relay-server 초기화
  1-1  스캐폴딩 (package.json, tsconfig, Drizzle 설정, 빈 서버)
  1-2  DB 스키마 (PostgreSQL 17 + Drizzle ORM, hosts.metadata 컬럼 추가)
  1-3  Fastify 플러그인 연결

Phase 2: 인증
  2-1  노드 토큰 서비스 (생성/파싱/검증)
  2-2  인증 미들웨어 (Bearer + HMAC + timestamp, Drizzle 쿼리)
  2-3  TOKEN_CACHE export

Phase 3: 핵심 서비스
  3-1  PollWaiters
  3-2  ACL 서비스 (Drizzle 쿼리)
  3-3  정리 스케줄러 (Drizzle 쿼리)

Phase 4: API 라우트
  4-1  방장 등록/관리 + ★ 메타데이터 조회 엔드포인트
  4-2  명령어 큐 (JSONB payload)
  4-3  Poll
  4-4  Result
  4-5  Heartbeat + ★ 메타데이터 동기화 수신
  4-6  라우트 통합

Phase 5: node-agent + ★ 메타데이터 동기화
  5-1  스캐폴딩
  5-2  인증 헬퍼
  5-3  Poller
  5-4  Heartbeat + ★ 데몬에서 모듈/서버/명령어 메타데이터 수집 · 릴레이 전송
  5-5  메인

Phase 6: Discord 봇 하이브리드 (5모듈 아키텍처 기반)
  6-1  모드 설정 추가 (global.toml, bot-config.json)
  6-2  core/ipc.js — 클라우드 트랜스포트 레이어 (★ 핵심 분기점)
  6-3  core/handler.js — 모드 인식 익스텐션 필터링 (music = 로컬 전용)
  6-4  core/resolver.js — 클라우드 메타데이터 로드
  6-5  index.js — 모드 초기화 + GuildVoiceStates 조건부
  6-6  GUI 토글 (DiscordBotModal.jsx)
  6-7  CLI 토글 (commands.rs)

Phase 7: 길드 연동 커맨드
  7-1  관리 슬래시 커맨드

Phase 8: 배포
  8-1  Dockerfile (Node.js 22)
  8-2  docker-compose (★ PostgreSQL 17 컨테이너 추가)
  8-3  Cloudflare 가이드
```

---

## 파일 생성/수정 목록

### 신규 생성

| 파일 | Phase |
|------|-------|
| `relay-server/package.json` | 1-1 |
| `relay-server/tsconfig.json` | 1-1 |
| `relay-server/drizzle.config.ts` | 1-1 |
| `relay-server/.env.example` | 1-1 |
| `relay-server/src/index.ts` | 1-1, 1-3, 4-6 |
| `relay-server/src/db/schema.ts` | 1-2 |
| `relay-server/src/db/index.ts` | 1-2 |
| `relay-server/src/middleware/rateLimit.ts` | 1-3 |
| `relay-server/src/services/nodeToken.ts` | 2-1 |
| `relay-server/src/middleware/auth.ts` | 2-2 |
| `relay-server/src/services/pollWaiters.ts` | 3-1 |
| `relay-server/src/services/acl.ts` | 3-2 |
| `relay-server/src/services/cleanup.ts` | 3-3 |
| `relay-server/src/routes/host.ts` | 4-1 |
| `relay-server/src/routes/command.ts` | 4-2 |
| `relay-server/src/routes/poll.ts` | 4-3 |
| `relay-server/src/routes/result.ts` | 4-4 |
| `relay-server/src/routes/heartbeat.ts` | 4-5 |
| `node-agent/package.json` | 5-1 |
| `node-agent/tsconfig.json` | 5-1 |
| `node-agent/src/index.ts` | 5-5 |
| `node-agent/src/config.ts` | 5-1 |
| `node-agent/src/auth.ts` | 5-2 |
| `node-agent/src/poller.ts` | 5-3 |
| `node-agent/src/heartbeat.ts` | 5-4 |
| `relay-server/Dockerfile` | 8-1 |
| `relay-server/docker-compose.yml` | 8-2 |
| `relay-server/DEPLOY.md` | 8-3 |

### 수정 (5모듈 아키텍처 기반)

| 파일 | Phase | 변경 |
|------|-------|------|
| `config/global.toml` | 6-1 | `[discord]` 섹션에 mode, cloud 설정 추가 |
| `discord_bot/bot-config.json` | 6-1 | mode, cloud 필드 추가 |
| `discord_bot/core/ipc.js` | 6-2 | ★ 클라우드 트랜스포트 (릴레이 API 분기, 메타데이터 캐시) |
| `discord_bot/core/handler.js` | 6-3 | localOnly 마커, 클라우드 모드에서 음악 스킵 |
| `discord_bot/core/resolver.js` | 6-4 | 클라우드 모드 설정 적용 (ipc.setMode 호출) |
| `discord_bot/index.js` | 6-5, 7-1 | 모드 초기화, GuildVoiceStates 조건부, 관리 슬래시 커맨드 |
| `discord_bot/core/processor.js` | — | **변경 없음** (추상화 계층 덕분) |
| `discord_bot/extensions/music.js` | — | **변경 없음** (handler.js에서 스킵) |
| `discord_bot/extensions/easter_eggs.js` | — | **변경 없음** (양쪽 모드 동작) |
| `discord_bot/extensions/rps.js` | — | **변경 없음** (양쪽 모드 동작) |
| `saba-chan-gui/src/components/Modals/DiscordBotModal.jsx` | 6-6 | 모드 토글 UI + 환경변수 주입 |
| `saba-chan-gui/main.js` | 6-6 | 봇 spawn 시 BOT_MODE, RELAY_URL, HOST_ID 환경변수 |
| `saba-chan-cli/src/tui/commands.rs` | 6-7 | bot 커맨드에 --mode 플래그 |
