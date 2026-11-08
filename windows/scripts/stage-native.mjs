import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const windowsRoot = path.resolve(here, "..");
const repoRoot = path.resolve(windowsRoot, "..");
const requestedArch = process.argv.includes("--arch") ? process.argv[process.argv.indexOf("--arch") + 1] : "x64";
const rustTarget = requestedArch === "arm64" ? "aarch64-pc-windows-msvc" : "x86_64-pc-windows-msvc";
const destination = path.join(windowsRoot, "resources", "bin");

function firstFile(label, environmentName, override, candidates) {
    const found = [override, ...candidates].filter(Boolean).map((entry) => path.resolve(entry)).find(existsSync);
    if (!found) {
        throw new Error(`${label} was not found. Build it first or set ${environmentName} to its absolute path.`);
    }
    return found;
}

const cli = firstFile("nola.exe", "NOLA_WINDOWS_CLI_PATH", process.env.NOLA_WINDOWS_CLI_PATH, [
    path.join(repoRoot, "target", rustTarget, "release", "nola.exe"),
]);
const agentHost = firstFile("nola-agent-host.exe", "NOLA_WINDOWS_AGENT_HOST_PATH", process.env.NOLA_WINDOWS_AGENT_HOST_PATH, [
    path.join(repoRoot, "target", rustTarget, "release", "nola-agent-host.exe"),
]);
const llamaServer = firstFile("llama-server.exe", "NOLA_LLAMA_SERVER", process.env.NOLA_LLAMA_SERVER, [
    path.join(repoRoot, "vendor", "llama.cpp", "build", "bin", "Release", "llama-server.exe"),
]);

mkdirSync(destination, { recursive: true });
for (const entry of readdirSync(destination)) {
    if (/\.(exe|dll|pdb)$/i.test(entry)) rmSync(path.join(destination, entry));
}
copyFileSync(cli, path.join(destination, "nola.exe"));
copyFileSync(agentHost, path.join(destination, "nola-agent-host.exe"));
copyFileSync(llamaServer, path.join(destination, "llama-server.exe"));

const llamaDirectory = path.dirname(llamaServer);
for (const entry of readdirSync(llamaDirectory)) {
    if (/\.dll$/i.test(entry)) copyFileSync(path.join(llamaDirectory, entry), path.join(destination, entry));
}

const licenseDirectory = path.join(windowsRoot, "resources", "licenses");
mkdirSync(licenseDirectory, { recursive: true });
const llamaSource = process.env.NOLA_LLAMA_CPP_SOURCE;
if (llamaSource) {
    const llamaLicense = path.join(path.resolve(llamaSource), "LICENSE");
    if (existsSync(llamaLicense)) copyFileSync(llamaLicense, path.join(licenseDirectory, "llama.cpp.txt"));
}

const nativeLock = JSON.parse(readFileSync(path.join(windowsRoot, "native-lock.json"), "utf8"));
writeFileSync(path.join(windowsRoot, "resources", "native-runtime.json"), `${JSON.stringify({
    architecture: requestedArch,
    backend: process.env.NOLA_LLAMA_BACKEND || "external",
    llamaCppRelease: nativeLock.llamaCpp.release,
    llamaCppCommit: process.env.NOLA_LLAMA_COMMIT || nativeLock.llamaCpp.commitPrefix,
}, null, 2)}\n`);

process.stdout.write(`Staged ${requestedArch} native runtime in ${destination}\n`);
