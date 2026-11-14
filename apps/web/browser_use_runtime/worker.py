# SPDX-License-Identifier: Apache-2.0
"""Fixed browser operations, one Browser Use process per owned conversation.

No Agent, model, cloud browser, arbitrary Python or model-supplied JavaScript.
The parent retains permission admission, element identity and observation.
"""
import asyncio
import contextlib
import importlib.metadata
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import time
from urllib.parse import urlsplit

VERSION = '1.0.2'


def validate_endpoint(value):
    url = urlsplit(value)
    if (url.scheme != 'http' or url.hostname != '127.0.0.1' or not url.port
            or url.username or url.password or url.path not in ('', '/') or url.query or url.fragment):
        raise ValueError('Browser Use requires this owner\'s loopback CDP endpoint')
    return value


def result_value(response):
    if response.get('exceptionDetails'):
        raise ValueError('Owned browser element is unavailable; observe again before acting')
    return response.get('result', {}).get('value')


def bound_element(helpers, key):
    if not isinstance(key, str) or not key.startswith('nola_bu_') or len(key) > 128:
        raise ValueError('Invalid owned element binding')
    response = helpers.cdp('Runtime.evaluate', expression=(
        '(()=>{const e=globalThis[' + json.dumps(key) + '];delete globalThis[' + json.dumps(key)
        + '];if(!e||!e.isConnected||e.ownerDocument!==document)throw Error("stale element");return e})()'))
    if response.get('exceptionDetails') or not response.get('result', {}).get('objectId'):
        raise ValueError('Owned element changed; take a new snapshot')
    return response['result']['objectId']


def call_element(helpers, object_id, function, *values):
    return result_value(helpers.cdp('Runtime.callFunctionOn', objectId=object_id,
        functionDeclaration=function, arguments=[{'value': value} for value in values], returnByValue=True))


def bind_cdp_session(helpers):
    """Disable the harness's implicit default-tab recovery for admitted input.

    Explicit sessions fail on stale pages instead of replaying a command into
    a replacement tab. This module is private to one conversation process.
    """
    original = helpers.cdp
    def owned_cdp(method, session_id=None, **params):
        owned = session_id or getattr(helpers, '_nola_session', None)
        if not method.startswith('Target.') and not owned:
            raise ValueError('No admitted browser page session')
        params.setdefault('_response_timeout', getattr(helpers, '_nola_timeout', 30.0))
        return original(method, session_id=owned, **params)
    helpers.cdp = owned_cdp


def execute(helpers, request):
    operation = request['operation']
    if operation not in ('navigate', 'pointer', 'wheel', 'key', 'text', 'click', 'type_text'):
        raise ValueError('Unsupported Browser Use operation')
    timeout = request.get('timeout_ms')
    helpers._nola_timeout = min(timeout / 1000, 30.0) if isinstance(timeout, (int, float)) and timeout > 0 else 30.0
    target = request['target_id']
    if not isinstance(target, str) or not target:
        raise ValueError('Missing owned page target')
    # Every operation selects the parent's exact target; popups never redirect it.
    if getattr(helpers, '_nola_target', None) != target:
        previous = getattr(helpers, '_nola_session', None)
        helpers._nola_session = helpers.switch_tab(target)
        helpers._nola_target = target
        if previous:
            with contextlib.suppress(Exception):
                helpers.cdp('Target.detachFromTarget', sessionId=previous)
    if operation == 'navigate':
        result = helpers.goto_url(request['url'])
        if result.get('errorText'):
            raise RuntimeError('Navigation was not completed: ' + result['errorText'])
        return
    if not hasattr(helpers, '_nola_points'):
        helpers._nola_points = {}
    if operation == 'pointer':
        helpers._nola_points[target] = (float(request['x']), float(request['y']))
        helpers.click_at_xy(float(request['x']), float(request['y']))
        return
    if operation == 'wheel':
        x, y = helpers._nola_points.get(target, (0, 0))
        helpers.cdp('Input.dispatchMouseEvent', type='mouseWheel', x=x, y=y,
            deltaX=float(request.get('delta_x', 0)), deltaY=float(request.get('delta_y', 0)))
        return
    if operation == 'key':
        if request.get('binding'):
            object_id = bound_element(helpers, request['binding'])
            try:
                call_element(helpers, object_id, '''function(){this.focus();if(document.activeElement!==this)throw Error('focus changed');return true}''')
            finally:
                with contextlib.suppress(Exception):
                    helpers.cdp('Runtime.releaseObject', objectId=object_id)
        helpers.press_key(request['key'], modifiers=int(request.get('modifiers', 0)))
        return
    if operation == 'text':
        helpers.type_text(request['text'])
        return
    if operation not in ('click', 'type_text'):
        raise ValueError('Unsupported Browser Use operation')
    object_id = bound_element(helpers, request['binding'])
    try:
        if operation == 'click':
            point = call_element(helpers, object_id, '''function(){
                if(!this.isConnected || this.matches(':disabled,[aria-disabled="true"]')) throw Error('unavailable');
                this.scrollIntoView({block:'center',inline:'center',behavior:'instant'});
                const r=this.getBoundingClientRect(),x=r.x+r.width/2,y=r.y+r.height/2;
                const hit=document.elementFromPoint(x,y);
                if(!r.width||!r.height||!hit||!(hit===this||this.contains(hit))) throw Error('covered element');
                return {x,y};
            }''')
            helpers._nola_points[target] = (point['x'], point['y'])
            helpers.click_at_xy(point['x'], point['y'])
        else:
            is_select = call_element(helpers, object_id, 'function(){return this instanceof HTMLSelectElement}')
            if is_select:
                selection = call_element(helpers, object_id, '''function(text){
                    if(!this.isConnected || this.matches(':disabled') || this.closest('[inert]') || this.multiple) return {ok:false,reason:'unavailable'};
                    this.scrollIntoView({block:'center',inline:'center',behavior:'instant'});
                    const r=this.getBoundingClientRect(),hit=document.elementFromPoint(r.x+r.width/2,r.y+r.height/2);
                    if(!r.width||!r.height||!hit||!(hit===this||this.contains(hit))) return {ok:false,reason:'covered'};
                    const enabled = Array.from(this.options).filter(option => !option.disabled && !option.closest('optgroup:disabled'));
                    const byValue = enabled.filter(option => option.value === text);
                    const matches = byValue.length ? byValue : enabled.filter(option => option.label === text);
                    if(matches.length !== 1) return {ok:false,reason:'option'};
                    this.focus();
                    if(document.activeElement !== this) return {ok:false,reason:'focus'};
                    const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype,'selectedIndex').set;
                    setter.call(this,matches[0].index);
                    this.dispatchEvent(new Event('input',{bubbles:true}));
                    this.dispatchEvent(new Event('change',{bubbles:true}));
                    return {ok:true};
                }''', request.get('text', ''))
                if not isinstance(selection, dict) or selection.get('ok') is not True:
                    reason = selection.get('reason') if isinstance(selection, dict) else None
                    messages = {'unavailable': 'Owned select is unavailable', 'covered': 'Owned select is covered',
                        'option': 'Select option is missing, disabled or ambiguous', 'focus': 'Owned select could not receive focus'}
                    raise ValueError(messages.get(reason, 'Owned select outcome is unavailable'))
                return
            call_element(helpers, object_id, '''function(clear){
                if(!this.isConnected || this.matches(':disabled,[readonly]')) throw Error('unavailable');
                if(!(this instanceof HTMLInputElement||this instanceof HTMLTextAreaElement||this.isContentEditable)) throw Error('not editable');
                this.focus();
                if(document.activeElement!==this) throw Error('focus changed');
                if(!clear && this.isContentEditable){const r=document.createRange();r.selectNodeContents(this);r.collapse(false);const s=getSelection();s.removeAllRanges();s.addRange(r)}
                return true;
            }''', request.get('clear', True))
            if request.get('clear', True):
                modifiers = 4 if sys.platform == 'darwin' else 2
                select_all = {'key': 'a', 'code': 'KeyA', 'modifiers': modifiers,
                              'windowsVirtualKeyCode': 65, 'nativeVirtualKeyCode': 65}
                helpers.cdp('Input.dispatchKeyEvent', type='rawKeyDown', commands=['SelectAll'], **select_all)
                helpers.cdp('Input.dispatchKeyEvent', type='keyUp', **select_all)
            else:
                helpers.press_key('End')
            if request.get('clear', True) and not request.get('text'):
                helpers.press_key('Backspace')
            else:
                text = request.get('text', '')
                delay = request.get('delay_ms')
                delay = 20 if delay is None else delay
                if not isinstance(delay, (int, float)) or not 0 <= delay <= 1000:
                    raise ValueError('delay_ms must be between 0 and 1000')
                if not request.get('clear', True) and delay:
                    for char in text:
                        helpers.type_text(char)
                        time.sleep(delay / 1000)
                else:
                    helpers.type_text(text)
    finally:
        with contextlib.suppress(Exception):
            helpers.cdp('Runtime.releaseObject', objectId=object_id)


async def main():
    if importlib.metadata.version('browser-use') != VERSION:
        raise RuntimeError('Install the pinned browser-use==' + VERSION + ' runtime')
    initial = json.loads(sys.stdin.readline())
    if initial.get('protocol') != 1:
        raise ValueError('Unsupported Nola browser worker protocol')
    endpoint = validate_endpoint(initial['cdp_url'])
    private = Path(tempfile.mkdtemp(prefix='nola-bu-'))
    os.environ.update({'ANONYMIZED_TELEMETRY': 'false', 'DO_NOT_TRACK': '1',
        'BROWSER_USE_SETUP_LOGGING': 'false', 'PYTHON_DOTENV_DISABLED': '1',
        'BH_HOME': str(private), 'BH_TAB_MARKER': '0', 'BH_DOMAIN_SKILLS': '0'})
    from browser_use import Browser
    browser = Browser(cdp_url=endpoint)
    try:
        env = await browser.start()
        os.environ.update(env)
        from browser_harness import helpers
        # Named harness daemons create one private startup tab. Bind the existing
        # Nola page, then close only that newly introduced blank startup target.
        owned = initial['owned_targets']
        if not owned or not all(isinstance(target, str) and target for target in owned):
            raise ValueError('Missing owned browser targets')
        startup = helpers.current_tab()
        helpers._nola_session = helpers.switch_tab(owned[0])
        helpers._nola_target = owned[0]
        if startup['target_id'] not in owned and startup['url'] == 'about:blank':
            helpers.close_tab(startup['target_id'])
        bind_cdp_session(helpers)
        print(json.dumps({'ready': True, 'protocol': 1, 'engine': 'browser-use', 'version': VERSION}), flush=True)
        for line in sys.stdin:
            request = json.loads(line)
            try:
                execute(helpers, request)
                response = {'id': request['id'], 'ok': True}
            except Exception as error:
                # Never replay: any dispatched input might already have taken effect.
                response = {'id': request['id'], 'ok': False, 'error': str(error)}
            print(json.dumps(response), flush=True)
    finally:
        await browser.stop()
        shutil.rmtree(private, ignore_errors=True)


if __name__ == '__main__':
    try:
        asyncio.run(main())
    except Exception as error:
        print(json.dumps({'ready': False, 'error': str(error)}), flush=True)
        sys.exit(1)
