import { ChildProcessByStdio, spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { existsSync } from "node:fs";
import path from "node:path";
import type { Readable } from "node:stream";

type WebHostProcess = ChildProcessByStdio<null, Readable, Readable>;

export type HostState =
    | { kind: "starting"; attempt: number }
    | { kind: "ready"; origin: string }
    | { kind: "stopped" }
    | { kind: "failed"; detail: string };

export type HostControllerOptions = {
    cliPath: string;
    webRoot: string;
    workspaceRoot: string;
    llamaServerPath?: string;
    llamaBackend?: string;
    debug?: boolean;
    log: (line: string) => void;
    onState: (state: HostState) => void;
};

const delay = (milliseconds: number) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds));

async function availablePort(): Promise<number> {
    return new Promise((resolve, reject) => {
        const server = createServer();
        server.unref();
        server.once("error", reject);
        server.listen(0, "127.0.0.1", () => {
            const address = server.address();
            if (!address || typeof address === "string") {
                server.close();
                reject(new Error("Could not allocate a loopback port"));
                return;
            }
            const port = address.port;
            server.close((error) => error ? reject(error) : resolve(port));
        });
    });
}

async function waitUntilHealthy(origin: string, child: WebHostProcess): Promise<void> {
    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
        if (child.exitCode !== null) throw new Error(`Agent Host exited with code ${child.exitCode}`);
        try {
            const response = await fetch(`${origin}/api/health`, { signal: AbortSignal.timeout(1_250) });
            if (response.ok) return;
        } catch {
            // The listener may be bound before the Web state is ready. Keep polling.
        }
        await delay(250);
    }
    throw new Error("Timed out waiting for the local Agent Host");
}

export class HostController {
    private child?: WebHostProcess;
    private explicitStop = false;
    private generation = 0;

    constructor(private readonly options: HostControllerOptions) {}

    async start(): Promise<string> {
        if (this.child && this.child.exitCode === null) throw new Error("Agent Host is already running");
        if (!existsSync(this.options.cliPath)) throw new Error(`Nola CLI was not found at ${this.options.cliPath}`);
        if (!existsSync(this.options.webRoot)) throw new Error(`Web resources were not found at ${this.options.webRoot}`);

        this.explicitStop = false;
        const generation = ++this.generation;
        let lastError: unknown;

        for (let attempt = 1; attempt <= 3; attempt += 1) {
            this.options.onState({ kind: "starting", attempt });
            try {
                const port = await availablePort();
                const origin = `http://127.0.0.1:${port}`;
                const environment: NodeJS.ProcessEnv = {
                    ...process.env,
                    NOLA_DESKTOP_PLATFORM: "windows",
                    NOLA_WEB_HOST: "127.0.0.1",
                    NOLA_WEB_PORT: String(port),
                };
                if (this.options.llamaServerPath) environment.NOLA_LLAMA_SERVER = this.options.llamaServerPath;
                if (this.options.llamaBackend) environment.NOLA_LLAMA_BACKEND = this.options.llamaBackend;

                if (process.platform === "win32") {
                    const architecture = process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : "unsupported";
                    environment.NOLA_UPDATE_TARGET = `windows-${architecture}-${this.options.llamaBackend ?? "unsupported"}`;
                }
                const arguments_ = ["web", "--host", "127.0.0.1", "--port", String(port), "--web-root", this.options.webRoot];
                if (this.options.debug) arguments_.unshift("--debug");
                const child = spawn(
                    this.options.cliPath,
                    arguments_,
                    {
                        cwd: this.options.workspaceRoot,
                        env: environment,
                        windowsHide: true,
                        stdio: ["ignore", "pipe", "pipe"],
                    },
                );
                this.child = child;
                let hasBecomeReady = false;
                child.stdout.on("data", (value: Buffer) => this.record("stdout", value));
                child.stderr.on("data", (value: Buffer) => this.record("stderr", value));
                child.once("error", (error) => this.options.log(`[process] ${error.stack ?? error.message}`));
                child.once("exit", (code, signal) => {
                    if (generation !== this.generation || this.child !== child) return;
                    this.child = undefined;
                    const detail = `Agent Host exited (code=${code ?? "none"}, signal=${signal ?? "none"})`;
                    this.options.log(`[process] ${detail}`);
                    if (this.explicitStop) {
                        this.options.onState({ kind: "stopped" });
                    } else if (hasBecomeReady) {
                        this.options.onState({ kind: "failed", detail });
                    }
                });

                await waitUntilHealthy(origin, child);
                if (generation !== this.generation) throw new Error("Agent Host start was superseded");
                hasBecomeReady = true;
                this.options.onState({ kind: "ready", origin });
                return origin;
            } catch (error) {
                lastError = error;
                this.options.log(`[startup:${attempt}] ${error instanceof Error ? error.stack ?? error.message : String(error)}`);
                this.terminateChild();
                if (attempt < 3) await delay(attempt * 500);
            }
        }

        const detail = lastError instanceof Error ? lastError.message : String(lastError);
        this.options.onState({ kind: "failed", detail });
        throw new Error(detail);
    }

    async restart(): Promise<string> {
        await this.stop();
        return this.start();
    }

    async stop(): Promise<void> {
        this.explicitStop = true;
        this.generation += 1;
        this.terminateChild();
        this.options.onState({ kind: "stopped" });
    }

    private terminateChild(): void {
        const child = this.child;
        this.child = undefined;
        if (!child || child.exitCode !== null) return;
        if (process.platform === "win32" && child.pid) {
            // taskkill /T terminates llama-server and any document helpers owned by the Host.
            spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
                windowsHide: true,
                stdio: "ignore",
            });
        } else {
            child.kill("SIGTERM");
        }
    }

    private record(channel: string, value: Buffer): void {
        const text = value.toString("utf8");
        for (const line of text.split(/\r?\n/)) {
            if (line.trim()) this.options.log(`[${channel}] ${line}`);
        }
    }
}

export function resolveExecutable(options: {
    override?: string;
    packagedCandidate: string;
    developmentCandidates: string[];
}): string {
    const candidates = [options.override, options.packagedCandidate, ...options.developmentCandidates]
        .filter((candidate): candidate is string => Boolean(candidate))
        .map((candidate) => path.resolve(candidate));
    return candidates.find(existsSync) ?? candidates[0];
}
