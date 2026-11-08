#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Click the production Web composer against an isolated, configured real Host.

Requires active Arithmetic QA and Language QA experts with working model defaults.
No mocked requests or replies. Supply the token file of the disposable Host;
this creates conversations and makes one real provider request.
"""
import argparse
from pathlib import Path
import tempfile
from playwright.sync_api import sync_playwright, expect


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', required=True)
    parser.add_argument('--token-file', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=Path(tempfile.mkdtemp(prefix='nola-web-mentions-')))
    parser.add_argument('--headless', action='store_true')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with sync_playwright() as playwright:
        browser = playwright.chromium.launch(channel='chrome', headless=args.headless)
        try:
            context = browser.new_context(viewport={'width': 1280, 'height': 900}, locale='en-US')
            response = context.request.post(args.url + '/api/web/login', data={'token': args.token_file.read_text().strip()})
            assert response.ok, f'Login failed: {response.status}'
            page = context.new_page()
            errors = []
            page.on('pageerror', lambda error: errors.append(str(error)))
            page.goto(args.url + '/chat', wait_until='domcontentloaded')
            page.locator('#chat-composer').wait_for()
            previous = page.url
            page.get_by_role('button', name='New conversation', exact=True).click()
            page.wait_for_url(lambda url: str(url) != previous)
            page.get_by_text('Loading conversation…', exact=True).wait_for(state='hidden')
            composer = page.locator('#chat-composer')
            composer.fill('@Arith')
            page.get_by_role('option', name='Arithmetic QA').wait_for()
            page.screenshot(path=str(args.output / 'candidates.png'))
            page.get_by_role('option', name='Arithmetic QA').click()
            expect(composer).to_have_value('')
            assert composer.evaluate('el => document.activeElement === el'), 'Selection lost keyboard focus'
            selected = page.locator('.selected-mentions button')
            expect(selected).to_have_count(1)
            composer.fill('@Lang')
            page.get_by_role('option', name='Language QA').wait_for()
            composer.press('Enter')
            expect(selected).to_have_count(2)
            selected.filter(has_text='Language QA').click()
            expect(selected).to_have_count(1)
            composer.fill('Compute 29 times 13. Reply with WEB_EXPERT_RESULT= followed by the result. Do not use tools.')
            page.get_by_role('button', name='Send', exact=True).click()
            page.get_by_text('WEB_EXPERT_RESULT=377', exact=True).wait_for(timeout=180_000)
            page.screenshot(path=str(args.output / 'real-reply.png'))
            page.reload(wait_until='domcontentloaded')
            page.get_by_text('WEB_EXPERT_RESULT=377', exact=True).wait_for(timeout=15_000)
            previous = page.url
            page.get_by_role('button', name='New conversation', exact=True).click()
            page.wait_for_url(lambda url: str(url) != previous)
            expect(selected).to_have_count(0)
            assert not errors, errors
            print('PASS: selection, focus, Enter selection, removal, real reply, reload, conversation isolation')
        finally:
            browser.close()


if __name__ == '__main__':
    main()
