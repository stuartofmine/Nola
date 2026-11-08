import { defineConfig } from "@playwright/test";

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
});
