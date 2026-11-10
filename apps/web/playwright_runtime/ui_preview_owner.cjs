const { execFileSync } = require('node:child_process');
const { realpathSync } = require('node:fs');
const path = require('node:path');

// Never print process environments or subprocess diagnostics: they may contain
// secrets. Any uncertainty means this harness must not write to the listener.
function verifyPreviewOwner(database, dependencies = {}) {
    const run = dependencies.run || ((file, args) => execFileSync(file, args, {
        encoding: 'utf8', timeout: 3000, maxBuffer: 1024 * 1024, stdio: ['ignore', 'pipe', 'pipe'],
    }));
    const canonical = dependencies.canonical || realpathSync;
    const executable = path.resolve(__dirname, '../../../target/debug/nola');
    const deny = () => { throw new Error('preview_listener_owner_unverified'); };
    try {
        if (!/^\/tmp\/nola-web-preview\.[^/\s]+\/state\.db$/.test(database || '')) return deny();
        if (canonical(database) !== path.join(canonical('/tmp'), database.slice('/tmp/'.length))) return deny();
        const listener = () => {
            const output = run('/usr/sbin/lsof', ['-nP', '-iTCP:5001', '-sTCP:LISTEN', '-Fp']);
            const pids = [...new Set(output.split('\n').filter(line => /^p[1-9][0-9]*$/.test(line)).map(line => line.slice(1)))];
            if (pids.length !== 1) return deny();
            return pids[0];
        };
        const pid = listener();
        const ps = (field, environment = false) => run('/bin/ps', [environment ? 'eww' : '-ww', '-p', pid, '-o', `${field}=`]).trim();
        const processExecutable = () => {
            const comm = ps('comm');
            if (path.isAbsolute(comm)) return canonical(comm);
            const directories = run('/usr/sbin/lsof', ['-a', '-p', pid, '-d', 'cwd', '-Fn'])
                .split('\n').filter(line => line.startsWith('n/')).map(line => line.slice(1));
            if (directories.length !== 1) return deny();
            return canonical(path.resolve(directories[0], comm));
        };
        const started = ps('lstart');
        if (!started || processExecutable() !== canonical(executable)) return deny();
        // ps appends environment to the unmodified argv. Remove that exact
        // prefix before looking for an environment key; argv cannot spoof it.
        const command = ps('command');
        const combined = ps('command', true);
        if (!command || !combined.startsWith(`${command} `)) return deny();
        const environment = combined.slice(command.length + 1);
        const bindings = [...environment.matchAll(/(?:^|\s)NOLA_DB_PATH=([^\s]*)(?=\s|$)/g)];
        if (bindings.length !== 1 || bindings[0][1] !== database
            || canonical(bindings[0][1]) !== canonical(database)) return deny();
        if (listener() !== pid || ps('lstart') !== started || processExecutable() !== canonical(executable)) return deny();
        return { pid: Number(pid), database: canonical(database) };
    } catch {
        return deny();
    }
}
module.exports = { verifyPreviewOwner };
