// SPDX-License-Identifier: Apache-2.0
// The shipped Windows startup renderer, without starting Tauri or its Host.
const {chromium} = require('playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const {startupPageHtml} = require('../../windows/dist/startupPage.js');
const {stringsFor} = require('../../windows/dist/i18n.js');

const ALL_LOCALES = ['en', 'zh-Hans', 'zh-Hant', 'ja', 'ko', 'de', 'es', 'fr', 'ru'];

function parseRgb(value) {
    const match = String(value).match(/rgba?\((\d+),\s*(\d+),\s*(\d+)/);
    if (!match) return null;
    return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function relativeLuminance([r, g, b]) {
    const [lr, lg, lb] = [r, g, b].map((c) => {
        const s = c / 255;
        return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * lr + 0.7152 * lg + 0.0722 * lb;
}

function contrastRatio(rgbA, rgbB) {
    const lumA = relativeLuminance(rgbA);
    const lumB = relativeLuminance(rgbB);
    return (Math.max(lumA, lumB) + 0.05) / (Math.min(lumA, lumB) + 0.05);
}

(async () => {
    const output = process.env.UI_EVIDENCE_DIR || require('./test_environment.cjs').artifactPath('nola-windows-startup');
    await fs.mkdir(output, {recursive: true});
    const browser = await chromium.launch({...require('./test_environment.cjs').browserOptions(), headless: true});
    try {
        const detail = '<script>window.untrustedMarkup=true</script> ' + 'Long fixture path 中文 '.repeat(30);
        for (const colorScheme of ['light', 'dark']) {
            const expectedBodyBg = colorScheme === 'dark' ? 'rgb(19, 19, 18)' : 'rgb(247, 246, 243)';
            const expectedCardBg = colorScheme === 'dark' ? 'rgb(27, 27, 25)' : 'rgb(255, 255, 255)';
            for (const width of [320, 1440]) {
                const context = await browser.newContext({colorScheme, viewport: {width, height: 900}, reducedMotion: 'no-preference'});
                await context.route('**/*', route => route.abort());
                const page = await context.newPage();
                const errors = [];
                page.on('pageerror', e => errors.push(e.message));
                try {
                    for (const locale of ALL_LOCALES) {
                        const strings = stringsFor(locale);
                        for (const mode of ['starting', 'failed']) {
                            await page.emulateMedia({reducedMotion: 'no-preference'});
                            await page.setContent(startupPageHtml(strings, mode, detail));
                            assert.equal(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), true, `${locale} ${mode} ${width} ${colorScheme} overflow`);
                            assert.equal(await page.evaluate(() => getComputedStyle(document.body).backgroundColor), expectedBodyBg);
                            assert.equal(await page.evaluate(() => getComputedStyle(document.querySelector('.card')).backgroundColor), expectedCardBg);
                            assert.equal(await page.locator('h1').textContent(), mode === 'failed' ? strings.failedTitle : strings.startingTitle);

                            const contrastCheck = await page.evaluate(() => {
                                const cardBg = getComputedStyle(document.querySelector('.card')).backgroundColor;
                                const h1Color = getComputedStyle(document.querySelector('h1')).color;
                                const pColor = getComputedStyle(document.querySelector('p')).color;
                                return {cardBg, h1Color, pColor};
                            });
                            const bgRgb = parseRgb(contrastCheck.cardBg);
                            assert.ok(contrastRatio(parseRgb(contrastCheck.h1Color), bgRgb) >= 4.5, `${locale} h1 contrast`);
                            assert.ok(contrastRatio(parseRgb(contrastCheck.pColor), bgRgb) >= 4.5, `${locale} p contrast`);

                            if (mode === 'failed') {
                                assert.equal(await page.locator('script').count(), 0);
                                assert.equal(await page.locator('.detail').textContent(), detail);
                                assert.equal(await page.locator('.actions').evaluate(e => e.getBoundingClientRect().bottom <= innerHeight), true, 'Recovery actions remain visible with long details');
                                if (locale === 'zh-Hans' || locale === 'en') {
                                    await page.locator('.detail').focus();
                                    await page.keyboard.press('PageDown');
                                    await page.waitForFunction(() => document.querySelector('.detail').scrollTop > 0);
                                    assert.equal(await page.locator('.detail').evaluate(e => e.scrollTop > 0), true);
                                    await page.locator('#nola-retry-host').focus();
                                    assert.equal(await page.locator('#nola-retry-host').evaluate(e => e === document.activeElement), true);
                                }
                            } else if (locale === 'zh-Hans' || locale === 'en') {
                                await page.emulateMedia({reducedMotion: 'reduce'});
                                assert.equal(await page.locator('.pulse').evaluate(e => getComputedStyle(e, '::after').animationName), 'none');
                            }

                            if (locale === 'zh-Hans') {
                                await page.screenshot({path: path.join(output, `${mode}-${width}-${colorScheme}.png`)});
                            }
                        }
                    }
                    assert.deepEqual(errors, []);
                } finally {
                    await context.close();
                }
            }
        }
        console.log(`PASS: Windows startup/failure HTML verified across ${ALL_LOCALES.length} locales, Light (#f7f6f3) / Dark (#131312), 320px/1440px, XSS escaping, keyboard focus, WCAG AA contrast, and reduced motion`);
    } finally {
        await browser.close();
    }
})().catch(e => {
    console.error(e);
    process.exitCode = 1;
});
