// SPDX-License-Identifier: Apache-2.0
import { lstat, open } from "node:fs/promises";
import path from "node:path";

const readyPrefix = /^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z \[state\] \{"kind":"ready"/;
const readyRecord = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z) \[state\] \{"kind":"ready","origin":"http:\/\/127\.0\.0\.1:([1-9]\d{0,4})"\}\r?$/;

function readyPort(line: string, complete: boolean): number | undefined {
    if (!readyPrefix.test(line)) return undefined;
    const match = complete ? readyRecord.exec(line) : null;
    if (!match) return 0;
    const timestamp = new Date(match[1]);
    const port = Number(match[2]);
    return Number.isFinite(timestamp.getTime()) && timestamp.toISOString() === match[1] && port <= 65535 ? port : 0;
}

/** Recover the last ready origin emitted by this Electron profile's old state logger. */
export async function previousHostPort(portFile: string): Promise<number> {
    const directory = path.join(path.dirname(portFile), "logs");
    const file = path.join(directory, "nola-windows.log");
    try {
        if (!(await lstat(directory)).isDirectory()) return 0;
        const info = await lstat(file);
        if (!info.isFile()) return 0;
        const log = await open(file, "r");
        try {
            const opened = await log.stat();
            if (!opened.isFile() || opened.dev !== info.dev || opened.ino !== info.ino) return 0;
            const chunk = Buffer.allocUnsafe(64 * 1024);
            let end = opened.size;
            let suffix = "";
            let complete = false;
            // Read newest first. A state record fits in 128 bytes, even when an
            // adjacent diagnostic line is several megabytes. Never log its contents.
            while (end > 0) {
                const size = Math.min(end, chunk.length);
                end -= size;
                const { bytesRead } = await log.read(chunk, 0, size, end);
                if (bytesRead !== size) return 0;
                const lines = chunk.toString("latin1", 0, size).split("\n");
                lines[lines.length - 1] += suffix;
                for (let index = lines.length - 1; index > 0; index--) {
                    const port = readyPort(lines[index], complete);
                    if (port !== undefined) return port;
                    complete = true;
                }
                suffix = lines[0].slice(0, 128);
            }
            return readyPort(suffix, complete) ?? 0;
        } finally {
            await log.close();
        }
    } catch {
        // Missing, inaccessible, or unsupported logs leave previous storage untouched.
        return 0;
    }
}
