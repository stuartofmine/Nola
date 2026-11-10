// Deterministic production renderer baseline, not a model/Core/FFI measurement.
const { chromium } = require('playwright-core');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
(async () => {
    const root = path.resolve(__dirname, '../app/static/ui-tests');
    const out = fs.mkdtempSync(require('./test_environment.cjs').temporaryPrefix('nola-a2ui-golden-chrome-'));
    const server = http.createServer((request, response) => {
        const file = path.resolve(root, '.' + new URL(request.url, 'http://localhost').pathname.replace(/^\/review-tests/, ''));
        if (!file.startsWith(root + path.sep) || !fs.existsSync(file) || !fs.statSync(file).isFile()) { response.writeHead(404).end(); return; }
        response.setHeader('Content-Type', file.endsWith('.js') ? 'application/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html');
        fs.createReadStream(file).pipe(response);
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const assert = require('node:assert/strict');
    const fixture = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../../../tests/fixtures/a2ui/form_interaction_v0_9_1.json')));
    const browser = await chromium.launch({...require('./test_environment.cjs').browserOptions(), headless: true});
    const results = [];
    try {
        for (const width of [1280, 390]) {
            const page = await browser.newPage({viewport: {width, height: 900}});
            const errors = [];
            page.on('pageerror', error => errors.push(error.message));
            await page.goto(`http://127.0.0.1:${server.address().port}/review-tests/tests/a2ui-golden.html`);
            await page.getByRole('textbox', {name: 'Name', exact: true}).fill('共同验收');
            await page.getByRole('radio', {name: '中文', exact: true}).check();
            await page.getByRole('checkbox', {name: 'Confirm', exact: true}).check();
            await page.getByRole('button', {name: 'Submit', exact: true}).click();
            await page.waitForFunction(() => document.querySelector('#calls').textContent === '1');
            const actual = JSON.parse(await page.locator('#submitted').innerText());
            const normalize = input => ({...input, edits: [...input.edits].sort((a,b) => a.instance_id.localeCompare(b.instance_id))});
            assert.deepEqual(normalize(actual), normalize(fixture.interaction));
            await page.getByText('共同验收', {exact: true}).waitFor();
            assert.equal(await page.getByRole('textbox', {name: 'Name', exact: true}).inputValue(), '共同验收');
            await page.screenshot({path: `${out}/${width}.png`, fullPage: true});
            await page.locator('#history').click();
            assert.equal(await page.locator('input:not(:disabled)').count(), 0);
            assert.equal(await page.getByRole('textbox',{name:'Name',exact:true}).inputValue(),'共同验收','historical text retains its committed value without writable bindings');
            assert.equal(await page.getByRole('radio',{name:'中文',exact:true}).isChecked(),true,'historical choice retains its committed value');
            assert.equal(await page.getByRole('checkbox',{name:'Confirm',exact:true}).isChecked(),true,'historical checkbox retains its committed value');
            assert.equal(await page.locator('#calls').innerText(), '1');
            assert.deepEqual(errors, []);
            results.push({width, passed: true, exact_core_interaction: true, history_read_only: true, errors});
            await page.close();
        }
        fs.writeFileSync(`${out}/result.json`, JSON.stringify({passed: true, results}, null, 2));
        console.log(JSON.stringify({passed: true, evidence: out}));
    } catch (error) {
        fs.writeFileSync(`${out}/result.json`, JSON.stringify({passed: false, results, error: String(error)}, null, 2));
        console.error(JSON.stringify({passed: false, evidence: out, error: String(error)}));
        process.exitCode = 1;
    } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
})().catch(error => {console.error(error); process.exitCode = 1;});
