import test from 'node:test';
import assert from 'node:assert/strict';
import { toolDisplayName } from './toolDisplayName.ts';

test('tool titles use the viewing language for both current and stored labels', () => {
    assert.equal(toolDisplayName('zh-Hans', 'render_ui', 'render_ui'), '显示交互界面');
    assert.equal(toolDisplayName('de', 'render_ui', '显示交互界面'), 'Interaktive Ansicht anzeigen');
    assert.equal(toolDisplayName('ja', 'run_command', 'Run command'), 'コマンドを実行');
    assert.equal(toolDisplayName('fr', 'write_file', 'Write project report'), 'Write project report');
    assert.equal(toolDisplayName('fr', 'mcp__reports__write_file', 'Custom report'), 'Custom report');
    assert.equal(toolDisplayName('fr', 'unknown-tool'), 'unknown-tool');
    assert.equal(toolDisplayName('es', 'device.files.read', 'device_read_file'), 'Leer archivo');
    assert.equal(toolDisplayName('zh-Hans', undefined, 'render_ui'), '显示交互界面');
    assert.equal(toolDisplayName('fr', 'external', 'render_ui'), 'render_ui');
    assert.equal(toolDisplayName('fr', '__proto__'), '__proto__');
});
