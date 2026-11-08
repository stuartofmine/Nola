// SPDX-License-Identifier: Apache-2.0
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {restoreWindowGeometry} = require('../dist/windowGeometry.js');
const primary = {x:0,y:0,width:1366,height:728};
test('first launch fits a small work area and retains native centering',()=>{
    assert.deepEqual(restoreWindowGeometry({},[primary],primary),{width:1366,height:728,maximized:false,x:undefined,y:undefined});
});
test('restored oversized window fits entirely instead of exposing only its corner',()=>{
    assert.deepEqual(restoreWindowGeometry({x:1200,y:600,width:1600,height:1000},[primary],primary),{width:1366,height:728,maximized:false,x:0,y:0});
});
test('removed monitor restores centered on primary without changing maximized intent',()=>{
    const state=restoreWindowGeometry({x:4000,y:100,width:900,height:600,maximized:true},[primary],primary);
    assert.deepEqual(state,{width:900,height:600,maximized:true,x:undefined,y:undefined});
});
test('negative monitor origins are retained and the title bar stays reachable',()=>{
    const left={x:-1920,y:-100,width:1920,height:1080};
    const state=restoreWindowGeometry({x:-2000,y:-160,width:1000,height:700},[primary,left],primary);
    assert.deepEqual(state,{x:-1920,y:-100,width:1000,height:700,maximized:false});
});
test('invalid scalar types never become accidental zero coordinates or infinite bounds',()=>{
    const state=restoreWindowGeometry({x:null,y:'0',width:Infinity,height:-2},[primary],primary);
    assert.equal(state.x,undefined);assert.equal(state.y,undefined);
    assert.equal(state.width,1366);assert.equal(state.height,728);
});
test('a work area smaller than the ordinary minimum still fits',()=>{
    const tiny={x:0,y:0,width:640,height:480};
    const state=restoreWindowGeometry({x:0,y:0,width:980,height:680},[tiny],tiny);
    assert.equal(state.width,640);assert.equal(state.height,480);
});
