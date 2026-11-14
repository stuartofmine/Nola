// SPDX-License-Identifier: Apache-2.0
// Browser Use owns admitted input; Playwright supplies snapshots and the preview.
'use strict';
const childProcess = require('node:child_process');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');

function runtimePaths() {
  const root = process.env.NOLA_BROWSER_USE_ROOT || path.join(__dirname, 'browser_use');
  return {
    python: process.env.NOLA_BROWSER_USE_PYTHON || path.join(root, 'python', 'bin', 'python3'),
    script: fs.existsSync(path.join(root, 'worker.py')) ? path.join(root, 'worker.py')
      : path.join(__dirname, '..', 'browser_use_runtime', 'worker.py'),
    modules: path.join(root, 'site-packages'),
  };
}

class BrowserUseExecutor {
  static async attach(cdpURL, context) {
    const runtime = runtimePaths();
    if (!fs.existsSync(runtime.python) || !fs.existsSync(runtime.script)) {
      throw new Error('Browser Use runtime is missing. Build the bundled browser runtime, or install apps/web/browser_use_runtime/requirements.lock in Python >=3.11 and set NOLA_BROWSER_USE_PYTHON to that Python executable.');
    }
    // Browser execution never receives provider credentials or a project .env.
    const env = Object.fromEntries(['PATH', 'SYSTEMROOT', 'WINDIR', 'LANG', 'LC_ALL', 'TMPDIR', 'TEMP', 'TMP', 'HOME', 'USER', 'LOGNAME']
      .filter(key => process.env[key]).map(key => [key, process.env[key]]));
    Object.assign(env, { PYTHONUNBUFFERED: '1', PYTHONNOUSERSITE: '1', PYTHON_DOTENV_DISABLED: '1',
      ANONYMIZED_TELEMETRY: 'false', DO_NOT_TRACK: '1' });
    if (fs.existsSync(runtime.modules)) env.PYTHONPATH = runtime.modules;
    const targets = [];
    for (const page of context.pages()) {
      const connection = await context.newCDPSession(page);
      try { targets.push((await connection.send('Target.getTargetInfo')).targetInfo.targetId); }
      finally { await connection.detach(); }
    }
    const child = childProcess.spawn(runtime.python, ['-u', runtime.script], { env, stdio: ['pipe', 'pipe', 'pipe'] });
    const executor = new BrowserUseExecutor(child);
    child.stdin.write(JSON.stringify({ protocol: 1, cdp_url: cdpURL, owned_targets: targets }) + '\n');
    try { await executor.ready; return executor; }
    catch (error) { await executor.close(); throw error; }
  }
  constructor(child) {
    this.child = child;
    this.pending = new Map();
    this.sequence = 0;
    this.ready = new Promise((resolve, reject) => { this.readyResolve = resolve; this.readyReject = reject; });
    this.readyTimer = setTimeout(() => this.fail(new Error('Browser Use startup timed out; no input was dispatched')), 45000);
    const lines = readline.createInterface({ input: child.stdout });
    lines.on('line', line => {
      let reply;
      try { reply = JSON.parse(line); } catch { return; } // third-party startup diagnostics never become tool evidence
      if ('ready' in reply) {
        clearTimeout(this.readyTimer);
        if (reply.ready && reply.protocol === 1 && reply.engine === 'browser-use' && reply.version === '1.0.2') this.readyResolve();
        else this.fail(new Error(reply.error || 'Browser Use version differs from the pinned runtime'));
      } else if (this.pending.has(reply.id)) {
        const waiting = this.pending.get(reply.id);
        this.pending.delete(reply.id); clearTimeout(waiting.timer);
        if (reply.ok) waiting.resolve(); else waiting.reject(new Error(reply.error || 'Browser Use action outcome is unknown'));
      }
    });
    child.stdin.on('error', error => this.fail(error));
    child.stderr.on('data', () => {}); // no credentials/page diagnostics in terminal logs
    child.on('error', error => this.fail(error));
    child.on('exit', () => { clearTimeout(this.forceTimer); this.fail(new Error('Browser Use exited; observe before deciding whether to repeat input')); });
  }
  fail(error) {
    clearTimeout(this.readyTimer);
    this.readyReject(error);
    for (const item of this.pending.values()) { clearTimeout(item.timer); item.reject(error); }
    this.pending.clear();
  }
  async perform(page, operation, args = {}, element = null) {
    const connection = await page.context().newCDPSession(page);
    let binding;
    try {
      const { targetInfo } = await connection.send('Target.getTargetInfo');
      if (element) {
        binding = 'nola_bu_' + crypto.randomUUID().replaceAll('-', '');
        await element.evaluate((node, key) => {
          if (!node.isConnected || node.ownerDocument !== document) throw new Error('stale owned element');
          Object.defineProperty(globalThis, key, { value: node, configurable: true });
        }, binding);
      }
      const id = ++this.sequence;
      await new Promise((resolve, reject) => {
        const timeout = Number.isInteger(args.timeout_ms) && args.timeout_ms > 0 ? Math.min(args.timeout_ms, 30000) : 30000;
        const timer = setTimeout(() => {
          this.pending.delete(id);
          reject(new Error('Browser Use action timed out; observe before deciding whether to repeat input'));
          // An abandoned worker must never execute a late command after its caller continues.
          this.terminate();
        }, timeout);
        this.pending.set(id, { resolve, reject, timer });
        this.child.stdin.write(JSON.stringify({ ...args, id, operation, target_id: targetInfo.targetId, binding }) + '\n', error => {
          if (error) this.fail(error);
        });
      });
    } finally {
      if (binding) await page.evaluate(key => { delete globalThis[key]; }, binding).catch(() => {});
      await connection.detach().catch(() => {});
    }
  }
  terminate() {
    if (this.child.exitCode !== null || this.child.signalCode !== null || this.forceTimer) return;
    // Third-party signal cleanup can itself stall on a dead daemon. Escalation
    // only targets this owned child, and stays alive until that child exits.
    this.forceTimer = setTimeout(() => this.child.kill('SIGKILL'), 1000);
    this.child.kill('SIGTERM');
  }
  async close() {
    this.child.stdin.end();
    if (this.child.exitCode !== null || this.child.signalCode !== null) return;
    await new Promise(resolve => {
      const timer = setTimeout(() => this.terminate(), 5000);
      this.child.once('exit', () => { clearTimeout(timer); resolve(); });
    });
  }

}
module.exports = { BrowserUseExecutor, runtimePaths };
