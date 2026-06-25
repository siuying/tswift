import type { APIRoute } from 'astro';

export const prerender = false;

const maxBytes = 24_000;
const timeoutMs = 10_000;

type CommandResult = {
  ok: boolean;
  code: number | null;
  stdout: string;
  stderr: string;
  timedOut: boolean;
  elapsedMs: number;
};

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

function clamp(output: string): string {
  if (output.length <= maxBytes) return output;
  return `${output.slice(0, maxBytes)}\n\n[prototype truncated ${output.length - maxBytes} bytes]`;
}

async function runCargo(args: string[]): Promise<CommandResult> {
  const [{ spawn }, pathModule] = await Promise.all([
    import('node:child_process'),
    import('node:path'),
  ]);
  const repoRoot = pathModule.resolve(process.cwd(), '../..');
  const started = Date.now();

  return new Promise((resolve) => {
    const child = spawn('cargo', args, {
      cwd: repoRoot,
      env: { ...process.env, CARGO_TERM_COLOR: 'never' },
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    let stdout = '';
    let stderr = '';
    let timedOut = false;

    const timer = setTimeout(() => {
      timedOut = true;
      child.kill('SIGKILL');
    }, timeoutMs);

    child.stdout.on('data', (chunk) => {
      stdout = clamp(stdout + chunk.toString());
    });

    child.stderr.on('data', (chunk) => {
      stderr = clamp(stderr + chunk.toString());
    });

    child.on('error', (error) => {
      clearTimeout(timer);
      resolve({
        ok: false,
        code: 1,
        stdout,
        stderr: `${stderr}\n${error.message}`.trim(),
        timedOut,
        elapsedMs: Date.now() - started,
      });
    });

    child.on('close', (code) => {
      clearTimeout(timer);
      resolve({
        ok: code === 0 && !timedOut,
        code,
        stdout,
        stderr: timedOut ? `${stderr}\nTimed out after ${timeoutMs}ms`.trim() : stderr,
        timedOut,
        elapsedMs: Date.now() - started,
      });
    });
  });
}

async function runLocalQswift(source: string): Promise<Response> {
  const [{ mkdtemp, rm, writeFile }, { tmpdir }, pathModule] = await Promise.all([
    import('node:fs/promises'),
    import('node:os'),
    import('node:path'),
  ]);

  const dir = await mkdtemp(pathModule.join(tmpdir(), 'qswift-web-sandbox-'));
  const file = pathModule.join(dir, 'main.swift');

  try {
    await writeFile(file, source, 'utf8');

    const compile = await runCargo(['run', '-q', '-p', 'qswift-cli', '--', 'dump', '--json', file]);
    const run = compile.ok
      ? await runCargo(['run', '-q', '-p', 'qswift-cli', '--', 'run', file])
      : null;

    return json({
      ok: Boolean(compile.ok && run?.ok),
      backend: 'local-cargo',
      compile: {
        ok: compile.ok,
        stderr: compile.stderr,
        astPreview: compile.stdout.slice(0, 6_000),
        elapsedMs: compile.elapsedMs,
      },
      run: run && {
        ok: run.ok,
        stdout: run.stdout,
        stderr: run.stderr,
        elapsedMs: run.elapsedMs,
      },
    });
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
}

async function proxyToRunner(runnerUrl: string, source: string): Promise<Response> {
  const response = await fetch(runnerUrl, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ source }),
  });

  const text = await response.text();
  return new Response(text, {
    status: response.status,
    headers: { 'content-type': response.headers.get('content-type') ?? 'application/json' },
  });
}

export const POST: APIRoute = async ({ request, locals }) => {
  let source = '';

  try {
    const body = await request.json();
    source = String(body?.source ?? '');
  } catch {
    return json({ error: 'Expected JSON body: { source }' }, 400);
  }

  if (source.length > 100_000) {
    return json({ error: 'Prototype limit: source must be <= 100KB' }, 413);
  }

  const runtimeEnv = (locals as { runtime?: { env?: Record<string, string> } })?.runtime?.env ?? {};
  const runnerUrl = runtimeEnv.QSWIFT_RUNNER_URL ?? import.meta.env.QSWIFT_RUNNER_URL;

  if (runnerUrl) {
    return proxyToRunner(runnerUrl, source);
  }

  if (import.meta.env.DEV) {
    return runLocalQswift(source);
  }

  return json(
    {
      ok: false,
      error: 'QSWIFT_RUNNER_URL is required on Cloudflare. Workers cannot spawn cargo/qswift directly.',
    },
    501,
  );
};
