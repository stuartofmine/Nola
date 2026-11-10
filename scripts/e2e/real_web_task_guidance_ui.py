#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Real Chrome task guidance with a deliberately lost HTTP response.

Uses an existing isolated loopback Host and its real configured model. Creates
work only by clicking the production UI. The intercepted response is fetched
from the real server before being dropped; no task or receipt is fabricated.
"""
import argparse
import json
from pathlib import Path
import sys
import sqlite3
import re
import time
from urllib.parse import urlparse
from playwright.sync_api import sync_playwright

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "apps/web/tests"))
from playwright_browser import launch_test_chromium


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--state', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    state = json.loads(args.state.read_text())
    assert urlparse(state['url']).hostname == '127.0.0.1'
    assert Path(state['root']).name.startswith('nola-subagent-real-')
    args.output.mkdir(parents=True, exist_ok=True)
    args.output.chmod(0o700)
    report = {'kind': 'real Chrome + real provider; lost-response fault injection', 'requests': [], 'pageErrors': []}
    started = time.monotonic()
    with sync_playwright() as playwright:
        browser = launch_test_chromium(playwright)
        context = browser.new_context(viewport={'width': 1360, 'height': 1000})
        page = context.new_page()
        page.on('pageerror', lambda error: report['pageErrors'].append(str(error)))
        def drop_committed_response(route):
            body = route.request.post_data_json
            response = route.fetch(timeout=90000)
            report['requests'].append({'path': urlparse(route.request.url).path,
                'intent': body, 'status': response.status, 'receipt': response.json()})
            route.abort('connectionreset')
        try:
            page.goto(state['url'] + '/chat')
            page.get_by_role('button', name='新建会话', exact=True).click(timeout=60000)
            page.get_by_text('想一起完成什么？', exact=True).wait_for(timeout=60000)
            page.wait_for_timeout(1000)  # Separate guidance test from recorded new-conversation hydration race.
            page.locator('textarea').first.fill(
                '请实际启动一个权限模式为read_only的子助手任务：计算1到500的整数和，按每10个数字一组列出50组小计，最后复核总数。'
                '子助手直接使用数学公式推导，不运行命令、不调用其他工具、不读写文件、不联网。你等待真实子任务完成后汇总。')
            page.evaluate("""() => {
                window.__guidanceUiStates = [];
                let last = '';
                const capture = () => {
                    const cards = [...document.querySelectorAll('.task-actions')].map(node => ({
                        guidance: !!node.querySelector('textarea'),
                        guidanceDisabled: node.querySelector('textarea')?.disabled ?? null,
                        buttons: [...node.querySelectorAll('button')].map(button => ({text:button.textContent,disabled:button.disabled}))
                    }));
                    const key = JSON.stringify(cards);
                    if (cards.length && key !== last) {
                        window.__guidanceUiStates.push({at:performance.now(),cards}); last = key;
                    }
                };
                new MutationObserver(capture).observe(document.body,{subtree:true,childList:true,attributes:true});
            }""")
            page.get_by_role('button', name='发送', exact=True).click()
            page.get_by_role('button', name='请求取消任务', exact=True).first.wait_for(timeout=180000)
            report['unboundGuidanceHidden'] = page.get_by_role('textbox', name='任务指导', exact=True).count() == 0
            if report['unboundGuidanceHidden']:
                page.screenshot(path=str(args.output / 'unbound-no-guidance.png'), full_page=True)
            report['conversationUrl'] = page.url
            guidance = page.get_by_role('textbox', name='任务指导', exact=True).first
            guidance.wait_for(timeout=180000)
            report['guidanceEnabledWithoutReload'] = True
            report['conversationUrl'] = page.url
            page.route('**/api/chat/agent/v2/task/message', drop_committed_response, times=1)
            guidance.fill('保持计算任务不变，请在最终结果中加入校验标记 GUIDANCE_5154。')
            page.get_by_role('button', name='发送指导', exact=True).first.click()
            page.get_by_role('button', name='确认原指令结果', exact=True).first.wait_for(timeout=120000)
            page.screenshot(path=str(args.output / 'lost-response.png'), full_page=True)
            # Reload the actual browser context: the persisted intent must
            # retain the original message ID, timestamp, text and exact owner.
            report['initialTaskUiStates'] = page.evaluate('window.__guidanceUiStates || []')
            persisted = args.output / 'before-cold-browser-storage.json'
            context.storage_state(path=str(persisted), indexed_db=True)
            persisted.chmod(0o600)
            # Close the actual browser, retaining only normal persistent web
            # storage. SessionStorage is deliberately not copied or invented.
            # The current Web contract must persist its own exact client ID.
            browser.close()
            browser = launch_test_chromium(playwright)
            context = browser.new_context(viewport={'width':1360,'height':1000}, storage_state=str(persisted))
            page = context.new_page()
            page.on('pageerror', lambda error: report['pageErrors'].append(str(error)))
            page.goto(report['conversationUrl'], wait_until='domcontentloaded')
            report['coldBrowserRestarted'] = True
            recover = page.get_by_role('button', name='确认原指令结果', exact=True).first
            recover.wait_for(timeout=120000)
            recover.click()
            # Hidden only means the controller entered Sending. Wait for a
            # real receipt response and its rendered settled state instead.
            page.wait_for_function("""() => [...document.querySelectorAll('.task-actions [role=status]')]
                .some(node => /已排队|已提交|已送达|已接收/.test(node.textContent || ''))""", timeout=90000)
            assert report['requests'], 'No real guidance submission was observed'
            assert 200 <= report['requests'][0]['status'] < 300, 'Dropped response was not a committed successful submission'
            intent = report['requests'][0]['intent']
            with sqlite3.connect((Path(state['root']) / 'state/nola-v5.db').as_uri() + '?mode=ro', uri=True) as database:
                records = database.execute('SELECT record_json FROM runtime_task_guidance WHERE task_id=? AND message_id=?',
                    (intent['taskId'], intent['messageId'])).fetchall()
            assert len(records) == 1, 'Recovery must preserve exactly one original guidance record'
            record = json.loads(records[0][0])
            assert record['content'] == intent['message'] and record['submitted_at_ms'] == intent['submittedAtMs']
            report['durableGuidance'] = record
            report['recoveredText'] = page.locator('body').inner_text()
            page.screenshot(path=str(args.output / 'recovered.png'), full_page=True)
            report['recoveryClicked'] = True
            # A queued receipt proves deduplication, not execution. Retain that
            # distinction and wait for the actual Core delivery outcome.
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline:
                with sqlite3.connect((Path(state['root']) / 'state/nola-v5.db').as_uri() + '?mode=ro', uri=True) as database:
                    rows = database.execute('SELECT record_json FROM runtime_task_guidance WHERE task_id=? AND message_id=?',
                        (intent['taskId'], intent['messageId'])).fetchall()
                assert len(rows) == 1, 'Guidance was duplicated or removed'
                report['durableGuidance'] = json.loads(rows[0][0])
                if report['durableGuidance']['status'] in ('delivered', 'rejected'):
                    break
                page.wait_for_timeout(1000)
            assert report['durableGuidance']['status'] == 'delivered', 'Core did not confirm actual guidance delivery'
            page.screenshot(path=str(args.output / 'delivered.png'), full_page=True)
            deadline = time.monotonic() + 240
            while time.monotonic() < deadline:
                with sqlite3.connect((Path(state['root']) / 'state/nola-v5.db').as_uri() + '?mode=ro', uri=True) as database:
                    binding = database.execute('SELECT run_id FROM runtime_task_run_bindings WHERE task_id=?', (intent['taskId'],)).fetchone()
                    assert binding, 'Actual child Run binding disappeared'
                    rows = database.execute('SELECT run_id,seal_json FROM assistant_run_terminal_seals WHERE run_id IN (?,?)',
                        (intent['parentRunId'], binding[0])).fetchall()
                report['terminalSeals'] = {run:json.loads(seal) for run,seal in rows}
                if len(rows) == 2:
                    break
                page.wait_for_timeout(1000)
            assert len(report['terminalSeals']) == 2, 'Parent and child did not both commit terminal seals'
            assert all(seal['result']['value']['kind'] == 'answered' for seal in report['terminalSeals'].values()), 'Terminal was not a successful answer'
            page.wait_for_function(r"() => /125[,\s]?250/.test(document.body.innerText)", timeout=30000)
            composer = page.locator('textarea').last
            assert composer.is_enabled(), 'Composer remained disabled after terminal'
            composer.fill('后续输入可编辑验证（不发送）')
            assert page.get_by_role('button', name='发送', exact=True).is_enabled(), 'Send remained unavailable after parent terminal'
            composer.fill('')
            report['parentChildSealedAndComposerReady'] = True
            page.screenshot(path=str(args.output / 'final-summary.png'), full_page=True)
        except Exception as error:
            report['failure'] = str(error)
            page.screenshot(path=str(args.output / 'failure.png'), full_page=True)
        finally:
            report['elapsedSeconds'] = time.monotonic() - started
            report['finalUrl'] = page.url
            report['textareaStates'] = page.locator('textarea').evaluate_all(
                '(nodes) => nodes.map(node => ({disabled:node.disabled,value:node.value}))')
            context.storage_state(path=str(args.output / 'browser-storage.json'), indexed_db=True)
            (args.output / 'browser-storage.json').chmod(0o600)
            report['finalText'] = page.locator('body').inner_text()
            (args.output / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2))
            browser.close()
    print(json.dumps({'failure': report.get('failure'), 'recoveryClicked': report.get('recoveryClicked', False),
        'realReceipts': len(report['requests']), 'evidence': str(args.output)}, ensure_ascii=False))
    return int('failure' in report)


if __name__ == '__main__':
    raise SystemExit(main())
