// SPDX-License-Identifier: Apache-2.0
// A real npm package served only on loopback so the desktop acceptance is reproducible.
const fs = require('node:fs');
const path = require('node:path');
const {execFileSync} = require('node:child_process');
const {createServer} = require('node:http');

async function desktopPluginRegistry(home) {
    const name = '@nola-ui-qa/board';
    const version = '1.0.0';
    const pkg = path.join(home, 'board-fixture');
    fs.mkdirSync(pkg);
    const manifest = {
        name, version,
        pi: {extensions:['./extension.js']}, type:'module',
        nola: {pages:[{id:'board', title:'工作看板', entry:'board.html'}]}
    };
    fs.writeFileSync(path.join(pkg, 'package.json'), JSON.stringify(manifest));
    fs.writeFileSync(path.join(pkg, 'extension.js'), 'export default function activate() {}\n');
    fs.writeFileSync(path.join(pkg, 'board.html'), `<!doctype html><html lang="zh-CN"><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>本周工作</title>
<style>
:root{color-scheme:light dark;font:16px/1.6 system-ui}body{margin:0;padding:clamp(1rem,4vw,3.25rem)}
header{border-bottom:1px solid #8886;margin-bottom:1.75rem}h1{font-size:2rem;margin:0 0 .75rem}
h2{font-size:.9rem;font-weight:500;color:GrayText;margin:0 0 .6rem}
section{display:grid;gap:1.25rem;grid-template-columns:repeat(auto-fit,minmax(10rem,1fr))}
article{border:1px solid #8885;border-radius:10px;padding:1rem}
button{font:inherit;border:1px solid #8887;border-radius:.5rem;padding:.35em .8em;cursor:pointer}
</style><header><h1>本周工作</h1></header><section><article><h2>待处理</h2>整理访谈笔记</article>
<article><h2>进行中</h2><p>校对研究报告</p><button>完成校对</button></article><article><h2>已完成</h2><p id="done">归档来源</p></article></section>
<script>document.querySelector('button').onclick = event => { document.querySelector('#done').textContent = '研究报告已校对'; event.target.disabled = true; };</script></html>`);
    const tarball = execFileSync('npm', ['pack','--silent','--offline','--ignore-scripts'], {cwd:pkg, encoding:'utf8'}).trim();
    const bytes = fs.readFileSync(path.join(pkg, tarball));
    const server = createServer((request, response) => {
        if (decodeURIComponent(request.url).toLowerCase() === '/' + name) {
            const url = 'http://127.0.0.1:' + server.address().port + '/board.tgz';
            response.writeHead(200, {'Content-Type':'application/json'});
            response.end(JSON.stringify({name,'dist-tags':{latest:version},versions:{[version]:{...manifest,dist:{tarball:url}}}}));
        } else if (request.url === '/board.tgz') {
            response.writeHead(200, {'Content-Type':'application/octet-stream'});
            response.end(bytes);
        } else {
            response.writeHead(404);
            response.end();
        }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    fs.writeFileSync(path.join(home,'.npmrc'), '@nola-ui-qa:registry=http://127.0.0.1:' + server.address().port + '/\n');
    return {name, close:() => new Promise(resolve=>server.close(resolve))};
}
module.exports = {desktopPluginRegistry};
