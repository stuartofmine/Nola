import test from 'node:test';
import assert from 'node:assert/strict';
import { Script } from 'node:vm';
import { diagramDocument } from './diagramDocument.ts';

test('diagram document keeps hostile labels inert and cannot access parent or network', () => {
    const source = 'flowchart BT\nA["</script><img src=https://evil.invalid onerror=alert(1)>"]';
    const html = diagramDocument(source, 'http://127.0.0.1:5001/static/vendor/mermaid/mermaid.min.js', true);
    assert.equal((html.match(/<script\b/g) ?? []).length, 2);
    assert.ok(!html.includes('<img'));
    assert.ok(html.includes("connect-src 'none'"));
    assert.ok(html.includes("securityLevel:'strict'"));
    assert.ok(html.includes("theme:'dark'"));
    assert.ok(html.includes('\\u003c/script\\u003e'));
    const inline = html.match(/<script>\n([\s\S]*?)<\/script>/)?.[1];
    assert.ok(inline);
    assert.doesNotThrow(() => new Script(inline!));
    assert.throws(() => diagramDocument('graph LR; A-->B', 'javascript:alert(1)', false));
    assert.ok(diagramDocument('graph LR; A-->B', 'https://localhost/library.js', false).includes("theme:'default'"));
});
