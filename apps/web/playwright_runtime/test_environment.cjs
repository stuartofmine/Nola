// SPDX-License-Identifier: Apache-2.0
// Repository-relative artifacts and browser discovery for development checks.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const repositoryRoot = path.resolve(__dirname, '../../..');
const artifactsRoot = path.join(repositoryRoot, '.artifacts');

function artifactPath(relative) {
    if (!relative || path.isAbsolute(relative) || path.win32.isAbsolute(relative)) {
        throw new Error('An artifact path must be relative to .artifacts');
    }
    const resolved = path.resolve(artifactsRoot, relative);
    if (!resolved.startsWith(artifactsRoot + path.sep)) {
        throw new Error('An artifact path must stay inside .artifacts');
    }
    fs.mkdirSync(path.dirname(resolved), {recursive: true});
    return resolved;
}

function temporaryPrefix(name) {
    if (!name || name === '.' || name === '..' || path.basename(name) !== name || /[\\/]/.test(name)) {
        throw new Error('A temporary prefix must be a file name');
    }
    return path.join(os.tmpdir(), name);
}

function browserOptions(environment = process.env) {
    const configured = environment.CHROME_PATH?.trim();
    return configured
        ? {executablePath: path.resolve(repositoryRoot, configured)}
        : {channel: 'chrome'};
}

function isolatedHostOrigin() {
    const statePath = process.env.NOLA_BUTTON_AUDIT_HOST_STATE;
    if (!statePath) throw new Error('Set NOLA_BUTTON_AUDIT_HOST_STATE to the isolated Host state file.');
    const state = JSON.parse(fs.readFileSync(statePath, 'utf8'));
    const origin = new URL(state.url);
    if (origin.protocol !== 'http:' || origin.hostname !== '127.0.0.1'
        || !path.basename(state.root).startsWith('nola-a2ui-real-')) {
        throw new Error('Expected an isolated loopback test Host.');
    }
    return origin;
}
module.exports = {artifactPath, temporaryPrefix, browserOptions, isolatedHostOrigin};
