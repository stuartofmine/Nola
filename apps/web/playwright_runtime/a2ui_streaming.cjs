const {chromium} = require('playwright-core');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const assert = require('node:assert/strict');
(async()=>{
    const root=path.resolve(__dirname,'../app/static/ui-tests');
    const evidence=fs.mkdtempSync(require('./test_environment.cjs').temporaryPrefix('nola-a2ui-streaming-chrome-'));
    const server=http.createServer((request,response)=>{
        const file=path.resolve(root,'.'+new URL(request.url,'http://localhost').pathname.replace(/^\/review-tests/,''));
        if(!file.startsWith(root+path.sep)||!fs.existsSync(file)||!fs.statSync(file).isFile()){response.writeHead(404).end();return;}
        response.setHeader('Content-Type',file.endsWith('.js')?'application/javascript':file.endsWith('.css')?'text/css':'text/html');
        fs.createReadStream(file).pipe(response);
    });
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    const browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
    const results=[];
    try {
        for(const width of [1280,390]){
            const page=await browser.newPage({viewport:{width,height:900}});
            const errors=[];page.on('pageerror',error=>errors.push(error.message));
            await page.goto(`http://127.0.0.1:${server.address().port}/review-tests/tests/a2ui-streaming.html`);
            const response=page.locator('#response');
            const visibleText=()=>response.innerText();
            assert.match(await visibleText(),/首轮思考/);
            await page.locator('#publish').click();
            await page.locator('[data-ui-surface]').waitFor();
            assert.match(await visibleText(),/公开进度/);
            assert.match(await visibleText(),/后续思考/);
            assert.equal(await response.locator('details.execution-process').getAttribute('open'),null);
            assert.doesNotMatch(await visibleText(),/Publish UI/,'tool details remain collapsed');
            await page.locator('#reason-delta').click();
            assert.match(await visibleText(),/后续思考：新增片段/);
            await page.locator('#private').click();
            assert.doesNotMatch(await response.textContent(),/PRIVATE-CANDIDATE/);
            assert.match(await visibleText(),/后续思考：新增片段/);
            await page.locator('#text-delta').click();
            assert.match(await visibleText(),/最终正文/);
            assert.match(await visibleText(),/公开进度/);
            await page.locator('#text-delta').click();
            assert.match(await visibleText(),/最终正文：新增片段/);
            await page.locator('#finish').click();
            assert.match(await visibleText(),/最终正文：新增片段/);
            await response.locator('details.execution-process > summary').click();
            assert.match(await visibleText(),/后续思考：新增片段/,'thinking remains accessible after completion');
            assert.equal(await response.locator('[data-ui-surface]').count(),1);
            assert.equal(errors.length,0);
            await page.screenshot({path:`${evidence}/${width}.png`,fullPage:true});
            results.push({width,passed:true,errors});await page.close();
        }
        fs.writeFileSync(`${evidence}/result.json`,JSON.stringify({passed:true,results},null,2));
        console.log(JSON.stringify({passed:true,evidence,results}));
    }catch(error){console.error(error);process.exitCode=1;fs.writeFileSync(`${evidence}/result.json`,JSON.stringify({passed:false,error:String(error),results}));}
    finally{await browser.close();await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
