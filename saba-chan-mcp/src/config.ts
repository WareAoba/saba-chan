/**
 * 데몬 접속 설정 해석.
 *
 * 개발 규칙: 경로/포트/토큰을 코드에 하드코딩하지 않는다. 데몬이 상속시키는
 * env var(`DAEMON_API_URL`, `SABA_IPC_PORT`, `SABA_TOKEN_PATH`, `APPDATA`)와
 * 기본 상수만 사용한다. 필수값은 시작 시점에 검증한다.
 */
import { readFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";

/** constants.DEFAULT_IPC_PORT 와 동일해야 함. */
const DEFAULT_IPC_PORT = 57474;

export interface DaemonConfig {
  baseUrl: string;
  /** 인증 비활성화(SABA_AUTH_DISABLED=1)면 null. */
  token: string | null;
}

/**
 * 데몬 베이스 URL 해석.
 * 우선순위: DAEMON_API_URL > SABA_DAEMON_URL > 127.0.0.1:${SABA_IPC_PORT|기본값}
 */
function resolveBaseUrl(): string {
  const explicit = [process.env.DAEMON_API_URL, process.env.SABA_DAEMON_URL]
    .map((value) => value?.trim())
    .find(Boolean);
  const address = explicit ?? `http://127.0.0.1:${process.env.SABA_IPC_PORT || DEFAULT_IPC_PORT}`;
  let url: URL;
  try {
    url = new URL(address);
  } catch {
    throw new Error("데몬 URL이 올바르지 않습니다.");
  }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) {
    throw new Error("데몬 URL은 인증 정보, 쿼리, fragment가 없는 HTTP(S) 주소여야 합니다.");
  }
  return url.href.replace(/\/+$/, "");
}

/**
 * IPC 토큰 파일 경로 해석.
 * 우선순위: SABA_TOKEN_PATH > SABA_DATA_DIR > 운영체제별 앱 데이터 경로.
 */
function resolveTokenPath(): string {
  const explicit = process.env.SABA_TOKEN_PATH;
  if (explicit && explicit.trim().length > 0) {
    return explicit;
  }
  if (process.env.SABA_DATA_DIR) {
    return path.join(process.env.SABA_DATA_DIR, ".ipc_token");
  }
  if (os.platform() === "win32" && process.env.APPDATA) {
    return path.join(process.env.APPDATA, "saba-chan", ".ipc_token");
  }
  if (os.platform() !== "win32" && process.env.HOME) {
    return path.join(process.env.HOME, ".config", "saba-chan", ".ipc_token");
  }
  return path.join(".", "saba-chan", ".ipc_token");
}

/**
 * 데몬 설정을 로드하고 필수값을 검증한다.
 * 인증이 활성화되어 있는데 토큰 파일을 읽을 수 없으면 throw.
 */
export function loadDaemonConfig(): DaemonConfig {
  const baseUrl = resolveBaseUrl();

  if (process.env.SABA_AUTH_DISABLED === "1") {
    return { baseUrl, token: null };
  }

  const tokenPath = resolveTokenPath();
  let token: string;
  try {
    token = readFileSync(tokenPath, "utf-8").trim();
  } catch (err) {
    throw new Error(
      `IPC 토큰 파일을 읽을 수 없습니다: ${tokenPath} (${(err as Error).message}). ` +
        "데몬이 실행 중인지, 경로가 올바른지 확인하세요.",
    );
  }
  if (token.length === 0) {
    throw new Error(`IPC 토큰 파일이 비어 있습니다: ${tokenPath}`);
  }

  return { baseUrl, token };
}
