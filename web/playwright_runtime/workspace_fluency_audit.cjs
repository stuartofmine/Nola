// All routed production pages, isolated browser API fixtures, no user DB/provider.
const http=require('node:http');
const fs=require('node:fs/promises');
const path=require('node:path');
const {spawn}=require('node:child_process');
(async()=>{
    const root=path.resolve(__dirname,'../app/static/ui');
    const index=await fs.readFile(path.join(root,'index.html'));
    const types={'.js':'text/javascript','.css':'text/css','.png':'image/png','.svg':'image/svg+xml','.woff2':'font/woff2','.json':'application/json'};
    const server=http.createServer(async(req,res)=>{
        const url=new URL(req.url,'http://127.0.0.1');
        if(url.pathname.startsWith('/api/')){res.writeHead(503);res.end();return;}
        if(!url.pathname.startsWith('/ui/')){res.setHeader('Content-Type','text/html; charset=utf-8');res.end(index);return;}
        try{
            const file=path.resolve(root,decodeURIComponent(url.pathname.slice(4)));
            if(!file.startsWith(root+path.sep))throw new Error('outside fixture');
            const content=await fs.readFile(file);res.setHeader('Content-Type',(types[path.extname(file)]||'application/octet-stream')+'; charset=utf-8');res.end(content);
        }catch{res.writeHead(404);res.end();}
    });
    await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
    try{
        const status=await new Promise((resolve,reject)=>{
            const child=spawn(process.execPath,[path.join(__dirname,'page_matrix_regression.cjs')],{stdio:'inherit',env:{...process.env,
                UI_FIXTURE_URL:`http://127.0.0.1:${server.address().port}`,UI_AUDIT_MOTION:'1'}});
            child.on('error',reject);child.on('exit',resolve);
        });
        if(status!==0)throw new Error(`Workspace page matrix failed: ${status}`);
    }finally{await new Promise(resolve=>server.close(resolve));}
})().catch(error=>{console.error(error);process.exitCode=1;});
