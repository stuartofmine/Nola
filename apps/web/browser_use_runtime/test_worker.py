# SPDX-License-Identifier: Apache-2.0
"""Ownership and failure semantics without a browser, model or dependency import."""
import types
import unittest
from unittest.mock import Mock
import worker


class WorkerContractTests(unittest.TestCase):
    def test_only_the_owned_loopback_debugging_endpoint_can_be_attached(self):
        self.assertEqual(worker.validate_endpoint('http://127.0.0.1:32145'), 'http://127.0.0.1:32145')
        for endpoint in ['https://127.0.0.1:1', 'http://example.com:1', 'http://127.0.0.1',
                         'http://u:p@127.0.0.1:1', 'http://127.0.0.1:1/rpc', 'http://127.0.0.1:1?token=x']:
            with self.subTest(endpoint=endpoint), self.assertRaises(ValueError):
                worker.validate_endpoint(endpoint)

    def test_unknown_operations_fail_before_selecting_or_acting(self):
        helpers = Mock()
        with self.assertRaisesRegex(ValueError, 'Unsupported'):
            worker.execute(helpers, {'operation': 'python', 'code': 'arbitrary()'})
        helpers.switch_tab.assert_not_called()
        helpers.cdp.assert_not_called()

    def test_stale_cdp_session_is_explicit_and_never_replayed_on_another_tab(self):
        original = Mock(side_effect=RuntimeError('Session with given id not found'))
        helpers = types.SimpleNamespace(cdp=original, _nola_session='owned-page-session')
        worker.bind_cdp_session(helpers)
        with self.assertRaisesRegex(RuntimeError, 'Session with given id'):
            helpers.cdp('Input.insertText', text='once')
        original.assert_called_once_with('Input.insertText', session_id='owned-page-session', _response_timeout=30.0, text='once')

    def test_missing_page_session_cannot_fall_back_to_the_daemon_default(self):
        original = Mock()
        helpers = types.SimpleNamespace(cdp=original)
        worker.bind_cdp_session(helpers)
        with self.assertRaisesRegex(ValueError, 'No admitted'):
            helpers.cdp('Page.navigate', url='https://example.com')
        original.assert_not_called()

    def test_detached_reference_never_reaches_click(self):
        helpers = types.SimpleNamespace(_nola_target='target', cdp=Mock(return_value={'exceptionDetails': { 'text': 'stale' }}), click_at_xy=Mock())
        with self.assertRaisesRegex(ValueError, 'changed'):
            worker.execute(helpers, {'operation': 'click', 'target_id': 'target', 'binding': 'nola_bu_fixture'})
        helpers.click_at_xy.assert_not_called()

    def test_native_select_uses_only_the_bound_option_and_never_keyboard_fallback(self):
        responses = [{'result': {'objectId': 'select'}}, {'result': {'value': True}}, {'result': {'value': {'ok': True}}}, {}]
        helpers = types.SimpleNamespace(_nola_target='target', cdp=Mock(side_effect=responses), type_text=Mock(), press_key=Mock())
        worker.execute(helpers, {'operation': 'type_text', 'target_id': 'target', 'binding': 'nola_bu_fixture', 'text': 'high'})
        call = helpers.cdp.call_args_list[2]
        self.assertEqual(call.kwargs['arguments'], [{'value': 'high'}])
        self.assertIn('matches.length !== 1', call.kwargs['functionDeclaration'])
        self.assertIn('option.value === text', call.kwargs['functionDeclaration'])
        helpers.type_text.assert_not_called()
        helpers.press_key.assert_not_called()
        self.assertEqual(helpers.cdp.call_args.kwargs['objectId'], 'select')

    def test_partial_click_failure_is_not_retried_and_releases_its_object(self):
        responses = [{'result': {'objectId': 'node'}}, {'result': {'value': {'x': 10, 'y': 20}}}, {}]
        helpers = types.SimpleNamespace(_nola_target='target', cdp=Mock(side_effect=responses),
            click_at_xy=Mock(side_effect=RuntimeError('disconnected after mouse down')))
        with self.assertRaisesRegex(RuntimeError, 'disconnected'):
            worker.execute(helpers, {'operation': 'click', 'target_id': 'target', 'binding': 'nola_bu_fixture'})
        helpers.click_at_xy.assert_called_once_with(10, 20)
        self.assertEqual(helpers.cdp.call_args.args, ('Runtime.releaseObject',))
        self.assertEqual(helpers.cdp.call_args.kwargs['objectId'], 'node')


if __name__ == '__main__':
    unittest.main()
