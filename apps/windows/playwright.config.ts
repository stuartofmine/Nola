import { defineConfig } from "@playwright/test";
import path from "node:path";

const repoRoot = path.resolve(__dirname, "../..");
const chromePath = process.env.CHROME_PATH?.trim();

export default defineConfig({
    testDir: "./tests",
    timeout: 90_000,
    fullyParallel: false,
    workers: 1,
    reporter: [["list"], ["html", { outputFolder: "test-results/report", open: "never" }]],
    outputDir: "test-results/artifacts",
    use: {
        trace: "retain-on-failure",
        screenshot: "only-on-failure",
    },
    projects: [
        {
            name: "chromium-desktop-shell",
            testMatch: /startupAndDesktopShell\.spec\.ts/,
            use: chromePath
                ? { launchOptions: { executablePath: path.resolve(repoRoot, chromePath) } }
                : { channel: "chrome" },
        },
        {
            name: "windows-electron",
            testMatch: /electron\.e2e\.ts/,
        },
    ],
});
