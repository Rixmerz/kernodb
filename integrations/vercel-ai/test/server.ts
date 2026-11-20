/**
 * Starts the real kerno-server binary for the tests: `$KERNO_SERVER_BIN` when that is set, else the
 * one `cargo build -p kerno-server` makes. A missing binary fails the tests: a client tested
 * against nothing proves nothing.
 */

import { type ChildProcess, spawn, spawnSync } from 'node:child_process';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

import { KernoClient } from 'kerno-client';

const repo = fileURLToPath(new URL('../../../', import.meta.url));
let binary: string | undefined;

function serverBinary(): string {
  if (binary !== undefined) return binary;
  if (process.env.KERNO_SERVER_BIN) return (binary = process.env.KERNO_SERVER_BIN);
  const build = spawnSync('cargo', ['build', '--quiet', '-p', 'kerno-server', '--message-format=json'], {
    cwd: repo,
    encoding: 'utf8',
    maxBuffer: 64 << 20,
  });
  if (build.status !== 0) throw new Error(`cargo build -p kerno-server failed:\n${build.stderr}`);
  for (const line of build.stdout.split('\n')) {
    if (!line.startsWith('{')) continue;
    const message = JSON.parse(line);
    if (message.reason === 'compiler-artifact' && message.target.name === 'kerno-server' && message.executable) {
      return (binary = message.executable as string);
    }
  }
  throw new Error('cargo did not report the kerno-server executable');
}

export interface Running {
  url: string;
  db: string;
  process: ChildProcess;
  /** Resolves with the exit code once the process is gone. */
  exited: Promise<number | null>;
}

export async function startServer(db: string): Promise<Running> {
  const child = spawn(serverBinary(), ['--db', db, '--listen', '127.0.0.1:0'], { stdio: ['ignore', 'pipe', 'inherit'] });
  const exited = new Promise<number | null>((resolve) => child.once('exit', (code) => resolve(code)));
  let seen = '';
  const url = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`the server did not start: ${seen}`)), 30_000);
    child.stdout!.on('data', (chunk: Buffer) => {
      seen += chunk.toString();
      const match = /listening on (http:\/\/\S+)/.exec(seen);
      if (match) {
        clearTimeout(timer);
        resolve(match[1]!);
      }
    });
    void exited.then((code) => reject(new Error(`the server exited with ${code} before it listened: ${seen}`)));
  });
  return { url, db, process: child, exited };
}

/** Asks the server to stop as a service manager does (SIGTERM) and waits for its exit code. */
export async function stopServer(server: Running): Promise<number | null> {
  server.process.kill('SIGTERM');
  return server.exited;
}

/** Runs `body` against a fresh server on a fresh database, and cleans up after it. */
export async function withServer(body: (kerno: KernoClient, server: Running) => Promise<void>): Promise<void> {
  const dir = await mkdtemp(join(tmpdir(), 'kerno-ai-sdk-'));
  // Two levels that do not exist yet: the server must create both.
  const server = await startServer(join(dir, 'data', 'db'));
  try {
    await body(new KernoClient(server.url), server);
  } finally {
    server.process.kill('SIGKILL');
    await server.exited;
    await rm(dir, { recursive: true, force: true });
  }
}

/** Every value of an async iterable. */
export async function collect<T>(values: AsyncIterable<T>): Promise<T[]> {
  const out: T[] = [];
  for await (const value of values) out.push(value);
  return out;
}
