import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadDaemonConfig } from "../src/config.js";

const directories: string[] = [];

function tokenFile(directory: string, token = "test-token"): string {
  mkdirSync(directory, { recursive: true });
  const file = path.join(directory, ".ipc_token");
  writeFileSync(file, `${token}\n`);
  return file;
}

function temporaryDirectory(): string {
  const directory = mkdtempSync(path.join(os.tmpdir(), "saba-mcp-"));
  directories.push(directory);
  return directory;
}

describe("loadDaemonConfig", () => {
  beforeEach(() => {
    const directory = temporaryDirectory();
    tokenFile(path.join(directory, "saba-chan"), "fallback-token");
    vi.stubEnv("APPDATA", directory);
  });

  afterEach(() => {
    vi.unstubAllEnvs();
    vi.restoreAllMocks();
    for (const directory of directories.splice(0)) {
      rmSync(directory, { recursive: true, force: true });
    }
  });

  it("데몬과 같은 SABA_DATA_DIR에서 토큰을 읽는다", () => {
    const directory = temporaryDirectory();
    tokenFile(directory);
    vi.stubEnv("SABA_AUTH_DISABLED", "0");
    vi.stubEnv("SABA_TOKEN_PATH", "");
    vi.stubEnv("SABA_DATA_DIR", directory);
    expect(loadDaemonConfig().token).toBe("test-token");
  });

  it("Unix에서는 HOME/.config/saba-chan의 토큰을 읽는다", () => {
    const directory = temporaryDirectory();
    tokenFile(path.join(directory, ".config", "saba-chan"));
    vi.spyOn(os, "platform").mockReturnValue("linux");
    vi.stubEnv("HOME", directory);
    vi.stubEnv("SABA_AUTH_DISABLED", "0");
    vi.stubEnv("SABA_TOKEN_PATH", "");
    vi.stubEnv("SABA_DATA_DIR", "");
    expect(loadDaemonConfig().token).toBe("test-token");
  });

  it("공백 URL override 다음의 유효한 URL을 사용한다", () => {
    vi.stubEnv("SABA_AUTH_DISABLED", "1");
    vi.stubEnv("DAEMON_API_URL", " ");
    vi.stubEnv("SABA_DAEMON_URL", " http://127.0.0.1:12345/ ");
    expect(loadDaemonConfig().baseUrl).toBe("http://127.0.0.1:12345");
  });

  it.each(["file:///tmp/daemon", "http://user:password@localhost", "http://localhost?token=x"])(
    "잘못된 데몬 URL을 시작 시 거부한다: %s",
    (url) => {
      vi.stubEnv("SABA_AUTH_DISABLED", "1");
      vi.stubEnv("DAEMON_API_URL", url);
      expect(() => loadDaemonConfig()).toThrow(/URL/);
    },
  );
});
