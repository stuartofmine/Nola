// SPDX-License-Identifier: Apache-2.0
// UI contracts use controlled metadata; cryptographic/download validation lives
// in nola_update_host. No production feed, credentials or model is contacted.
const {chromium}=require('playwright-core');
const assert=require('node:assert/strict');
const path=require('node:path');
const fs=require('node:fs');
const {pathToFileURL}=require('node:url');
(async()=>{
    const root=path.resolve(__dirname,'../frontend');
    const {createServer}=await import(pathToFileURL(path.join(root,'node_modules/vite/dist/node/index.js')).href);
    const fixture='\0software-update-fixture';
    const server=await createServer({root,server:{host:'127.0.0.1',port:0},plugins:[{
        name:'owned-update-fixture',resolveId(id){if(id.endsWith('/__update-entry.js'))return fixture},
        load(id){if(id===fixture)return `import {mount} from 'svelte';import '/src/theme.css';import Settings from '/src/components/SoftwareUpdateSettings.svelte';import {locale} from '/src/lib/i18n.ts';locale.set('en');mount(Settings,{target:document.querySelector('main')});`;},
        configureServer(vite){vite.middlewares.use(async(req,res,next)=>{
            if(!req.url?.split('?')[0].endsWith('/__update'))return next();
            const html='<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><style>body{margin:16px}main{max-width:760px;margin:auto}button,a{min-height:44px}</style></head><body><main></main><script type="module" src="/ui/__update-entry.js"></script></body></html>';
            res.setHeader('Content-Type','text/html');res.end(await vite.transformIndexHtml(req.url,html));
        });}
    }]});
    let browser;const cases=[],out=process.env.NOLA_UPDATE_UI_OUT||require('./test_environment.cjs').artifactPath('nola-update-ui');fs.mkdirSync(out,{recursive:true});
    try {
        await server.listen();const base='http://127.0.0.1:'+server.httpServer.address().port;
        browser=await chromium.launch({...require('./test_environment.cjs').browserOptions(),headless:true});
        for(const [width,scheme]of[[1000,'dark'],[320,'light'],[320,'dark']]){
            const context=await browser.newContext({viewport:{width,height:740},colorScheme:scheme,acceptDownloads:true});
            let scenario='disabled',checks=0;const errors=[];
            const snapshot={status:'disabled',current_version:'1.0.0',current_build:1,channel:'internal',target:'cli-macos-aarch64',release:null};
            await context.route('**/api/software-update**',async route=>{
                const request=route.request(),url=new URL(request.url());
                if(url.pathname==='/api/software-update/package'){
                    assert.equal(url.searchParams.get('release_id'),'a'.repeat(64));
                    return route.fulfill({status:200,headers:{'content-type':'application/octet-stream','content-disposition':'attachment; filename="Nola-fixture.zip"'},body:'fixture'});
                }
                if(url.pathname.endsWith('/check')){
                    checks++;assert.equal(request.method(),'POST');assert.deepEqual(JSON.parse(request.postData()),{});
                    if(scenario==='error')return route.fulfill({status:502,json:{error:'Update server unavailable',code:'network_error'}});
                    if(scenario==='available')return route.fulfill({json:{...snapshot,status:'available',release:{release_id:'a'.repeat(64),version:'1.0.1',build:2,notes:'<img src=x onerror="window.injected=true">',artifact:{kind:'archive',url:'https://publisher.example/Nola.zip',filename:'Nola.zip',sha256:'b'.repeat(64),size:7}}}});
                    if(scenario==='incompatible')return route.fulfill({json:{...snapshot,status:'incompatible_data'}});
                }
                return route.fulfill({json:snapshot});
            });
            const page=await context.newPage();page.on('pageerror',e=>errors.push(e.message));
            await page.goto(base+'/ui/__update');
            const check=page.getByRole('button',{name:'Check for updates',exact:true});
            await page.getByText('Update publishing is not configured',{exact:true}).waitFor();assert.equal(checks,0);
            scenario='error';await check.click();await page.getByRole('alert').waitFor();assert.equal(await check.isEnabled(),true);
            assert.equal(await page.getByText('You have the latest published version',{exact:true}).count(),0);
            scenario='available';await check.click();const link=page.getByRole('link',{name:'Download update',exact:true});await link.waitFor();
            assert.ok((await link.getAttribute('href')).startsWith('/api/software-update/package?release_id='));
            assert.equal(await page.locator('img').count(),0);assert.equal(await page.evaluate(()=>window.injected),undefined);
            const [download]=await Promise.all([page.waitForEvent('download'),link.click()]);
            await download.saveAs(path.join(out,`fixture-${width}-${scheme}.zip`));assert.equal(fs.readFileSync(await download.path(),'utf8'),'fixture');
            assert.equal(await link.getAttribute('aria-disabled'),'true');
            await page.getByText('Download requested.',{exact:false}).waitFor();
            scenario='incompatible';await check.click();await page.getByText('The new data format differs.',{exact:false}).waitFor();
            assert.equal(await page.getByRole('link',{name:'Download update',exact:true}).count(),0);
            assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);
            await page.screenshot({path:path.join(out,`settings-${width}-${scheme}.png`)});
            assert.deepEqual(errors,[]);cases.push({width,scheme,checks,passed:true});await context.close();
        }
        fs.writeFileSync(path.join(out,'receipt.json'),JSON.stringify({passed:true,cases},null,2));console.log(JSON.stringify({passed:true,cases,out}));
    }finally{if(browser)await browser.close();await server.close();}
})().catch(error=>{console.error(error);process.exitCode=1});
