// SPDX-License-Identifier: Apache-2.0
const {test}=require('node:test');
const assert=require('node:assert/strict');
const {isSoftwareUpdateDownload}=require('../dist/softwareUpdate.js');
const origin='http://127.0.0.1:51234', hash='a'.repeat(64);
test('only the exact owned Host and release digest can initiate an update download',()=>{
    assert.equal(isSoftwareUpdateDownload(`${origin}/api/software-update/package?release_id=${hash}`,origin),true);
    for(const value of [
        `https://example.com/api/software-update/package?release_id=${hash}`,
        `${origin}/api/software-update/package?release_id=${hash}&release_id=${hash}`,
        `${origin}/api/software-update/package?release_id=../binary`,
        `${origin}/api/software-update/package?release_id=${hash}#fragment`,
        `${origin}/api/software-update/package?release_id=${hash}&url=https://example.com`,
        `http://user@127.0.0.1:51234/api/software-update/package?release_id=${hash}`,
    ])assert.equal(isSoftwareUpdateDownload(value,origin),false);
    assert.equal(isSoftwareUpdateDownload(`${origin}/api/software-update/package?release_id=${hash}`,undefined),false);
});
