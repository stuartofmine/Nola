// Static proposal rendering only; this does not launch or access Nola.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { chromium } = require('../../../apps/web/playwright_runtime/node_modules/playwright-core');

(async () => {
    const root = require('../../../apps/web/playwright_runtime/test_environment.cjs').artifactPath('ui-direction-20261020');
    const browser = await chromium.launch({
        ...require('../../../apps/web/playwright_runtime/test_environment.cjs').browserOptions(),
        headless: true,
    });
    const findings = [];
    try {
        for (const file of fs.readdirSync(root).filter(file => file.endsWith('.svg'))) {
            const mobile = file.startsWith('mobile');
            const page = await browser.newPage({
                viewport: mobile ? { width: 390, height: 844 } : { width: 1440, height: 960 },
                deviceScaleFactor: 2,
            });
            await page.goto(pathToFileURL(path.join(root, file)).href);
            await page.evaluate(() => document.fonts.ready);
            const overflow = await page.evaluate(() => {
                const svg = document.documentElement;
                const { width, height } = svg.viewBox.baseVal;
                return [...document.querySelectorAll('text')].flatMap(node => {
                    const b = node.getBBox();
                    return b.x < 0 || b.y < 0 || b.x + b.width > width || b.y + b.height > height
                        ? [{ text: node.textContent, x: b.x, y: b.y, width: b.width }] : [];
                });
            });
            findings.push({ file, textOutsideArtboard: overflow });
            await page.screenshot({ path: path.join(root, file.replace('.svg', '.png')) });
            await page.close();
        }
        fs.writeFileSync(path.join(root, 'render-check.json'), JSON.stringify(findings, null, 2));
        if (findings.some(finding => finding.textOutsideArtboard.length)) {
            throw new Error(JSON.stringify(findings));
        }
        console.log(`Rendered ${findings.length} proposals; no text outside artboard.`);
    } finally {
        await browser.close();
    }
})().catch(error => { console.error(error); process.exitCode = 1; });
