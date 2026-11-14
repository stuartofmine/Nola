/** Render model diagrams in an opaque sandbox, using only the bundled library.
 * Source never becomes executable HTML; no network or parent-page access. */
export function diagramDocument(source: string, libraryUrl: string, dark: boolean): string {
    const literal = JSON.stringify(source).replaceAll('<', '\\u003c').replaceAll('>', '\\u003e');
    const script = new URL(libraryUrl);
    if (!['http:', 'https:'].includes(script.protocol)) throw new Error('Invalid diagram library URL');
    const url = script.href.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;');
    return `<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src ${script.origin} 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; connect-src 'none'; base-uri 'none'; form-action 'none'">
<style>html,body{margin:0;background:transparent;color:${dark ? '#e7e9ee' : '#16181d'};font:16px system-ui}#diagram{padding:12px;width:max-content;min-width:calc(100% - 24px)}svg{max-width:none!important;height:auto}a{pointer-events:none}</style>
<script src="${url}"></script></head><body><div id="diagram"></div><script>
const source=${literal};
const post=(status)=>parent.postMessage({type:'nola-diagram',status,height:Math.ceil(document.documentElement.scrollHeight)},'*');
try {
mermaid.initialize({startOnLoad:false,securityLevel:'strict',theme:'${dark ? 'dark' : 'default'}',suppressErrorRendering:true,flowchart:{htmlLabels:false}});
mermaid.render('nola-graph',source).then(result=>{
document.getElementById('diagram').innerHTML=result.svg;
post('ready');
}).catch(()=>post('failed'));
} catch { post('failed'); }
</script></body></html>`;
}
