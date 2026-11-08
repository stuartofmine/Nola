// Deterministic production renderer baseline, not a model/Core/FFI measurement.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
(async () => {
    const root = path.resolve(__dirname, '../app/static/ui-tests');
    const out = fs.mkdtempSync('/tmp/nola-a2ui-accessibility-chrome-');
    const server = http.createServer((request, response) => {
        const file = path.resolve(root, '.' + new URL(request.url, 'http://localhost').pathname.replace(/^\/review-tests/, ''));
        if (!file.startsWith(root + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { response.writeHead(404).end(); return; }
        response.setHeader('Content-Type', file.endsWith('.js') ? 'application/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
        fs.createReadStream(file).pipe(response);
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const assert = require('node:assert/strict');
    const fixture = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../../tests/fixtures/a2ui/form_interaction_v0_9_1.json')));
    const browser = await chromium.launch({executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', headless: true});
    const results = [];
    try {
        for (const width of [1280, 390]) {
            const page = await browser.newPage({viewport: {width, height: 900}});
            const errors = [];
            page.on('pageerror', error => errors.push(error.message));
            await page.goto(`http://127.0.0.1:${server.address().port}/review-tests/tests/a2ui-accessibility.html`);
            const current = page.getByRole('region',{name:'当前卡片',exact:true});
            const history = page.getByRole('region',{name:'历史卡片',exact:true});
            const historicalBefore = await history.getByRole('radio').evaluateAll(nodes=>nodes.map(n=>n.checked));
            const selected = await current.getByRole('radio').evaluateAll(nodes=>nodes.findIndex(n=>n.checked));
            await current.getByRole('radio').nth(selected === 0 ? 1 : 0).check();
            const historicalAfter = await history.getByRole('radio').evaluateAll(nodes=>nodes.map(n=>n.checked));
            const tabs = page.getByRole('tab');
            await tabs.nth(0).focus();
            await page.keyboard.press('ArrowRight');
            const arrow = await tabs.nth(1).evaluate(e=>document.activeElement===e && e.getAttribute('aria-selected')==='true');
            await page.keyboard.press('End');
            const end = await tabs.nth(2).evaluate(e=>document.activeElement===e && e.getAttribute('aria-selected')==='true');
            await page.keyboard.press('ArrowRight');
            const wrap = await tabs.nth(0).evaluate(e=>document.activeElement===e && e.getAttribute('aria-selected')==='true');
            await page.keyboard.press('ArrowLeft');
            await page.keyboard.press('Home');
            const home = await tabs.nth(0).evaluate(e=>document.activeElement===e && e.getAttribute('aria-selected')==='true');
            const singleTabStop = await tabs.evaluateAll(nodes=>nodes.filter(e=>e.tabIndex===0).length===1);
            const panel = page.getByRole('tabpanel');
            const linked = await panel.evaluate(e=>!!e.id && !!e.getAttribute('aria-labelledby') && document.getElementById(e.getAttribute('aria-labelledby'))?.getAttribute('aria-controls')===e.id);
            const literals=page.getByRole('region',{name:'静态值',exact:true});
            const literalValues = {
                text:await literals.getByRole('textbox',{name:'Static text'}).inputValue(),
                checkbox:await literals.getByRole('checkbox',{name:'Static checkbox'}).isChecked(),
                choice:await literals.getByRole('radio',{name:'B',exact:true}).isChecked(),
                slider:await literals.getByRole('slider').inputValue(),
                date:await literals.locator('input[type=datetime-local]').inputValue(),
            };
            assert.deepEqual(literalValues,{text:'静态值',checkbox:true,choice:true,slider:'75',date:'2026-10-21T12:34'});
            assert.equal(await literals.locator('input:enabled,button:enabled,textarea:enabled').count(),0,'literal projections must not gain writable paths');
            const result = {width, historicalRadioUnchanged:JSON.stringify(historicalBefore)===JSON.stringify(historicalAfter), arrowNavigation:arrow,
                panelLinked:linked,endNavigation:end,wrapNavigation:wrap,homeNavigation:home,singleTabStop,literalValues,errors};
            results.push(result);
            await page.screenshot({path:`${out}/${width}.png`,fullPage:true});
            await page.close();
        }
        const passed=results.every(r=>r.historicalRadioUnchanged && r.arrowNavigation && r.panelLinked && r.endNavigation && r.wrapNavigation && r.homeNavigation && r.singleTabStop && !r.errors.length);
        fs.writeFileSync(`${out}/result.json`, JSON.stringify({passed, results}, null, 2));
        console.log(JSON.stringify({passed, evidence: out,results}));
        if(!passed)process.exitCode=1;
    } catch (error) {
        fs.writeFileSync(`${out}/result.json`, JSON.stringify({passed: false, results, error: String(error)}, null, 2));
        console.error(JSON.stringify({passed: false, evidence: out, error: String(error)}));
        process.exitCode = 1;
    } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => {console.error(error); process.exitCode = 1;});
