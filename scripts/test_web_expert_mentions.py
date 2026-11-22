#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Click the production Web composer against an isolated, configured real Host.

Requires active Arithmetic QA and Language QA experts with working model defaults.
No mocked requests or replies. Supply the token file of the disposable Host;
this creates a conversation, sends a real turn, and edits it in place for a second turn.
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
            previous = page.url
            page.get_by_role('button', name='New conversation', exact=True).first.click()
            page.wait_for_url(lambda url: str(url) != previous)
            page.get_by_text('Loading conversation…', exact=True).wait_for(state='hidden')
            composer = page.locator('#chat-composer')
            composer.fill('@Arith')
            page.get_by_role('option', name='Arithmetic QA').wait_for()
            page.screenshot(path=str(args.output / 'candidates.png'))
            page.get_by_role('option', name='Arithmetic QA').click()
            expect(composer).to_have_value('@Arithmetic QA')
            assert composer.evaluate('el => document.activeElement === el'), 'Selection lost keyboard focus'
            composer.press('End')
            composer.press_sequentially(' @Lang')
            page.get_by_role('option', name='Language QA').wait_for()
            composer.press('Enter')
            expect(composer).to_have_value('@Arithmetic QA @Language QA')
            composer.press('Backspace')
            expect(composer).to_have_value('@Arithmetic QA ')
            page.screenshot(path=str(args.output / 'inline-mentions.png'))
            composer.press_sequentially('Compute 29 times 13. Reply with WEB_EXPERT_RESULT= followed by the result.')
            page.get_by_role('button', name='Send', exact=True).click()
            page.wait_for_function("document.querySelector('[data-message-id]') || document.querySelector('.composer [role=alert]')", timeout=15_000)
            assert not errors, errors
            page.get_by_text('WEB_EXPERT_RESULT=377', exact=True).first.wait_for(timeout=180_000)
            page.screenshot(path=str(args.output / 'real-reply.png'))
            page.reload(wait_until='domcontentloaded')
            page.get_by_text('WEB_EXPERT_RESULT=377', exact=True).first.wait_for(timeout=15_000)
            saved = page.locator('button.inline-name').filter(has_text='@Arithmetic QA')
            expect(saved).to_have_count(1)
            saved.click()
            expect(page.get_by_role('dialog').get_by_text('@Arithmetic QA')).to_be_visible()
            page.get_by_role('dialog').get_by_role('button', name='Close', exact=True).click()
            original_url = page.url
            page.get_by_role('button', name='Edit and resend', exact=True).first.click()
            edited = page.locator('#edit-expert-message')
            assert '@Arithmetic QA' in edited.input_value()
            expect(page.locator('dialog .highlight strong')).to_have_text('@Arithmetic QA')
            edited.fill('@Lang')
            page.get_by_role('option', name='Language QA').click()
            edited.press('End')
            edited.press_sequentially(' Reply with exactly WEB_EDIT_OK.')
            page.screenshot(path=str(args.output / 'edit-with-replacement.png'))
            page.get_by_role('button', name='Save and resend', exact=True).click()
            page.get_by_text('WEB_EDIT_OK', exact=True).first.wait_for(timeout=180_000)
            expect(page.locator('button.inline-name').filter(has_text='Language QA')).to_have_count(1)
            expect(saved).to_have_count(0)
            assert page.url == original_url
            page.screenshot(path=str(args.output / 'edited-result.png'))
            previous = page.url
            page.get_by_role('button', name='New conversation', exact=True).first.click()
            page.wait_for_url(lambda url: str(url) != previous)
            expect(composer).to_have_value('')
            expect(page.locator('.inline-editor .highlight')).to_have_count(0)
            assert not errors, errors
            print('PASS inline selection, real reply, reload, expert replacement, same-conversation rewind and next-draft isolation')
        except Exception:
            if 'page' in locals():
                page.screenshot(path=str(args.output / 'failure.png'))
                print('Visible page:',page.locator('body').inner_text()[:1200], 'Errors:', errors[:3], flush=True)
            raise
        finally:
            browser.close()


if __name__ == '__main__':
    main()
