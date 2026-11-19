import { ChildProcessByStdio, spawn, spawnSync } from "node:child_process";
import { createServer } from "node:net";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import type { Readable } from "node:stream";
import { previousHostPort } from "./previousHostPort";

type WebHostProcess = ChildProcessByStdio<null, Readable, Readable>;

export type HostState =
    | { kind: "starting"; attempt: number }
    | { kind: "ready"; origin: string }
    | { kind: "stopped" }
    | { kind: "failed"; detail: string };

export type HostControllerOptions = {
    cliPath: string;
    webRoot: string;
    portFile?: string;
    workspaceRoot: string;
    llamaServerPath?: string;
    llamaBackend?: string;
    debug?: boolean;
    maxAttempts?: number;
    log: (line: string) => void;
    onState: (state: HostState) => void;
};

const delay = (milliseconds: number) => new Promise<void>((resolve) => setTimeout(resolve, milliseconds));

async function availablePort(preferred: number): Promise<number> {
    return new Promise((resolve, reject) => {
        const server = createServer();
        server.unref();
        server.once("error", (error: NodeJS.ErrnoException) => {
            if (preferred && error.code === "EADDRINUSE") {
                reject(new Error(`Desktop Host port ${preferred} is busy. Close the other listener and retry to reopen your workspace.`));
            } else {
                reject(error);
            }
        });
        server.listen(preferred, "127.0.0.1", () => {
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

async function waitForHost(origin: string, child: WebHostProcess): Promise<void> {
    // Read readiness from our own child before accessing a persistent browser origin.
    const lines = createInterface({ input: child.stdout });
    try {
        await new Promise<void>((resolve, reject) => {
            const finish = (error?: Error) => {
                clearTimeout(timer);
                child.off("exit", exited);
                child.off("error", failed);
                if (error) reject(error); else resolve();
            };
            const exited = (code: number | null, signal: NodeJS.Signals | null) =>
                finish(new Error(`Agent Host exited with code ${code ?? signal}`));
            const failed = (error: Error) => finish(error);
            const timer = setTimeout(() => finish(new Error("Timed out waiting for the local Agent Host")), 45_000);
            child.once("exit", exited);
            child.once("error", failed);
            lines.on("line", line => {
                let message;
                try { message = JSON.parse(line); } catch { return; }
                if (message?.type !== "server.started") return;
                finish(message.origin === origin && message.loopback === true && message.protocol_version === 1
                    ? undefined : new Error("Unexpected local Host startup handshake"));
            });
        });
    } finally {
        lines.close();
        // The handshake includes an access token. Never echo raw stdout into the shell logs.
        child.stdout.resume();
    }
    for (const route of ["/api/health", "/chat"]) {
        const response = await fetch(origin + route, { signal: AbortSignal.timeout(5_000), redirect: "error" });
        const body = await response.text();
        if (!response.ok || (route === "/chat" && !body.includes('id="app"'))) {
            throw new Error("The local Host cannot serve " + route);
        }
    }
}

export class HostController {
    private child?: WebHostProcess;
    private preferredPort = 0;
    private startup?: Promise<string>;
    private generation = 0;

    constructor(private readonly options: HostControllerOptions) {}

    async start(): Promise<string> {
        if (this.startup || (this.child && this.child.exitCode === null)) throw new Error("Agent Host is already running");
        if (!existsSync(this.options.cliPath)) {
            const detail = `Nola CLI was not found at ${this.options.cliPath}`;
            this.options.onState({ kind: "failed", detail });
            throw new Error(detail);
        }
        if (!existsSync(this.options.webRoot)) {
            const detail = `Web resources were not found at ${this.options.webRoot}`;
            this.options.onState({ kind: "failed", detail });
            throw new Error(detail);
        }

        this.startup = this.startAttempts();
        try {
            return await this.startup;
        } finally {
            this.startup = undefined;
        }
    }

    private async startAttempts(): Promise<string> {
        const generation = ++this.generation;
        const maxAttempts = Math.max(1, this.options.maxAttempts ?? 3);
        let lastError: unknown;

        for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
            if (generation !== this.generation) throw new Error("Agent Host start was superseded");
            this.options.onState({ kind: "starting", attempt });
            try {
                const file = this.options.portFile;
                const hasPortFile = file && existsSync(file);
                if (file && hasPortFile) {
                    const saved = Number(readFileSync(file, "utf8").trim());
                    if (!Number.isInteger(saved) || saved < 1 || saved > 65535) throw new Error("Invalid desktop Host port in " + file);
                    this.preferredPort = saved;
                } else if (file && !this.preferredPort) {
                    this.preferredPort = await previousHostPort(file);
                }
                const port = await availablePort(this.preferredPort);
                if (generation !== this.generation) throw new Error("Agent Host start was superseded");
                if (file && !hasPortFile) writeFileSync(file, String(port), { flag: "wx", mode: 0o600 });
                this.preferredPort = port;
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
                const arguments_ = ["web", "--host", "127.0.0.1", "--port", String(port), "--web-root", this.options.webRoot, "--json"];
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
                child.stderr.on("data", (value: Buffer) => this.record("stderr", value));
                child.once("error", (error) => this.options.log(`[process] ${error.stack ?? error.message}`));
                child.once("exit", (code, signal) => {
                    if (generation !== this.generation || this.child !== child) return;
                    this.child = undefined;
                    const detail = `Agent Host exited (code=${code ?? "none"}, signal=${signal ?? "none"})`;
                    this.options.log(`[process] ${detail}`);
                    if (hasBecomeReady) {
                        this.options.onState({ kind: "failed", detail });
                    }
                });

                await waitForHost(origin, child);
                if (generation !== this.generation || this.child !== child) throw new Error("Agent Host start was interrupted");
                hasBecomeReady = true;
                this.options.onState({ kind: "ready", origin });
                return origin;
            } catch (error) {
                if (generation !== this.generation) throw error;
                lastError = error;
                this.options.log(`[startup:${attempt}] ${error instanceof Error ? error.stack ?? error.message : String(error)}`);
                await this.terminateChild();
                if (attempt < maxAttempts) await delay(attempt * 500);
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
        this.generation += 1;
        const pending = this.startup;
        await this.terminateChild();
        await pending?.catch(() => undefined);
        this.options.onState({ kind: "stopped" });
    }

    private async terminateChild(): Promise<void> {
        const child = this.child;
        this.child = undefined;
        if (!child || child.exitCode !== null || child.signalCode !== null || child.pid === undefined) return;
        const closed = new Promise<void>(resolve => child.once("close", () => resolve()));
        const force = setTimeout(() => child.kill("SIGKILL"), 2_000);
        force.unref();
        if (process.platform === "win32" && child.pid) {
            // taskkill /T terminates llama-server and any document helpers owned by the Host.
            spawnSync("taskkill.exe", ["/PID", String(child.pid), "/T", "/F"], {
                windowsHide: true,
                stdio: "ignore",
            });
        } else {
            child.kill("SIGTERM");
        }
        try { await closed; } finally { clearTimeout(force); }
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
