# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import asyncio
import json
import importlib.util
import sys
import types
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path


class _BaseAgent:
    pass


harbor = types.ModuleType("harbor")
harbor_agents = types.ModuleType("harbor.agents")
harbor_agents_base = types.ModuleType("harbor.agents.base")
harbor_agents_base.BaseAgent = _BaseAgent
harbor_environments = types.ModuleType("harbor.environments")
harbor_environments_base = types.ModuleType("harbor.environments.base")
harbor_environments_base.BaseEnvironment = object
harbor_models = types.ModuleType("harbor.models")
harbor_models_agent = types.ModuleType("harbor.models.agent")
harbor_models_context = types.ModuleType("harbor.models.agent.context")
harbor_models_context.AgentContext = object
for name, module in {
    "harbor": harbor,
    "harbor.agents": harbor_agents,
    "harbor.agents.base": harbor_agents_base,
    "harbor.environments": harbor_environments,
    "harbor.environments.base": harbor_environments_base,
    "harbor.models": harbor_models,
    "harbor.models.agent": harbor_models_agent,
    "harbor.models.agent.context": harbor_models_context,
}.items():
    sys.modules.setdefault(name, module)

root = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location(
    "harbor_nola_agent", root / "evals" / "harbor_nola_agent.py"
)
adapter = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(adapter)


class HarborArtifactContractTests(unittest.TestCase):
    def test_public_acceptance_and_shell_ablation_are_explicit_metadata(self):
        class Environment:
            def __init__(self):
                self.commands, self.uploads = [], {}
            async def exec(self, *, command, **kwargs):
                self.commands.append(command)
                return types.SimpleNamespace(return_code=0)
            async def upload_file(self, source, destination):
                self.uploads[destination] = Path(source).read_text()
        agent = object.__new__(adapter.NolaHarborAgent)
        agent.session_id = 'custom__trial__agent'
        agent.expected_artifacts_by_task = {}
        agent.endpoint_host, agent.model_name = 'localhost', 'fixture'
        agent.network_policy, agent.tool_set = 'allow', 'auto'
        agent.timeout_seconds = 600
        agent.shell_purpose_contract = 'optional'
        agent.task_acceptance_by_task = {'custom':dict(argv=['python3','public_test.py'],
            cwd='/app', timeout_seconds=20, public=True)}
        environment, context = Environment(), types.SimpleNamespace(metadata={})
        asyncio.run(agent.run('Original task', environment, context))
        invocation = next(command for command in environment.commands if '/opt/nola/supervisor.py' in command)
        self.assertIn('NOLA_SHELL_PURPOSE_CONTRACT=optional', invocation)
        self.assertIn('/opt/nola/acceptance.py', invocation)
        self.assertIn('--session nola-eval-acceptance', invocation)
        self.assertEqual(environment.uploads['/tmp/nola-eval-instruction.txt'], 'Original task')
        self.assertEqual(context.metadata['nola_task_acceptance'], 'configured_public_check')
        self.assertIsNone(context.metadata['nola_exit_code'])
        self.assertIsNone(context.metadata['nola_process_succeeded'])
        self.assertEqual(context.metadata['nola_harness_exit_code'], 0)

    def test_preserves_model_contract_but_not_unrelated_user_configuration(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / 'nola'
            binary.touch()
            config = Path(directory) / 'config.json'
            llm = dict(connections=[dict(id='fixture', name='Fixture',
                       endpoint='http://192.168.1.2/v1', protocol='responses',
                       api_key_env='NOLA_BENCHMARK_TOKEN',
                       models=[dict(id='fixture', name='Fixture', output_limit=16384)])],
                       default_model=dict(connection='fixture', model='fixture'))
            config.write_text(json.dumps(dict(llm=llm,
                model_capabilities={'vision_input': True}, network_policy='allow',
                unrelated_service_secret='must-not-copy')))
            with patch.object(adapter.NolaHarborAgent, '_init_model_info',
                              create=True, return_value=None):
                agent = adapter.NolaHarborAgent(binary_path=str(binary), config_path=str(config))
            self.assertEqual(agent.minimal_config['llm'], llm)
            self.assertEqual(agent.minimal_config['model_capabilities'], {'vision_input': True})
            self.assertNotIn('unrelated_service_secret', agent.minimal_config)
            scope = ' '.join(agent.minimal_config['instructions'])
            self.assertIn('isolated, disposable task environment', scope)
            self.assertIn('same task time budget', scope)
            self.assertIn('network/permission restrictions', scope)
            self.assertNotIn('poppler', scope.lower())

    def test_parses_arbitrary_resources_without_case_specific_rules(self) -> None:
        self.assertEqual(
            adapter._parse_expected_artifacts(
                '{"arbitrary-task":["/workspace/a.bin","/workspace/a.bin"],'
                '"另一个任务":["/shared/result"]}'
            ),
            {
                "arbitrary-task": ["/workspace/a.bin"],
                "另一个任务": ["/shared/result"],
            },
        )

    def test_accepts_mapping_already_decoded_by_harbor(self) -> None:
        self.assertEqual(
            adapter._parse_expected_artifacts(
                {"arbitrary-task": [" /workspace/a.bin ", "/workspace/a.bin"]}
            ),
            {"arbitrary-task": ["/workspace/a.bin"]},
        )

    def test_uses_harbor_session_identity_and_never_instruction_text(self) -> None:
        self.assertEqual(
            adapter._task_name_from_session_id("arbitrary-task__RK8btD9__agent"),
            "arbitrary-task",
        )
        self.assertIsNone(
            adapter._task_name_from_session_id(
                "please create /app/result.txt from this instruction"
            )
        )

    def test_verifier_collection_metadata_never_becomes_agent_requirements(self) -> None:
        class Environment:
            def __init__(self):
                self.commands = []
                self.prompt = None

            async def exec(self, *, command, **kwargs):
                self.commands.append(command)
                return types.SimpleNamespace(return_code=0)

            async def upload_file(self, source, destination):
                self.prompt = Path(source).read_text()

        agent = object.__new__(adapter.NolaHarborAgent)
        agent.session_id = 'arbitrary-task__trial__agent'
        agent.expected_artifacts_by_task = {'arbitrary-task': [
            '/tmp/collected.patch', '/workspace/input.yaml', '/workspace/output/']}
        agent.endpoint_host = '192.168.1.2'
        agent.model_name = 'test-model'
        agent.network_policy = 'allow'
        agent.tool_set = 'auto'
        agent.timeout_seconds = 60
        environment = Environment()
        context = types.SimpleNamespace(metadata={})
        asyncio.run(agent.run('Original user instruction', environment, context))
        self.assertEqual(environment.prompt, 'Original user instruction')
        self.assertNotIn('--expected-artifact', ' '.join(environment.commands))
        self.assertNotIn('--no-save', ' '.join(environment.commands))
        self.assertIn('NOLA_DB_PATH=/tmp/nola-eval.db', ' '.join(environment.commands))
        self.assertIn('NOLA_PERMISSION_PROFILE=allow_all', ' '.join(environment.commands))
        self.assertEqual(context.metadata['nola_permission_profile'], 'allow_all')
        self.assertTrue(context.metadata['nola_evidence_exported'])
        export_command = next(command for command in environment.commands
                              if '/opt/nola/export_evidence.py' in command)
        self.assertIn('/logs/agent/nola.ndjson', export_command)
        self.assertIn('/tmp/nola-home/.nola/config.json', export_command)
        self.assertLess(environment.commands.index(export_command),
                        next(i for i, command in enumerate(environment.commands)
                             if 'gzip -f' in command))
        self.assertNotIn('/tmp/collected.patch', ' '.join(environment.commands))
        self.assertEqual(context.metadata['nola_requested_artifacts'],
                         agent.expected_artifacts_by_task['arbitrary-task'])
        self.assertNotIn('nola_collected_artifacts', context.metadata)

    def test_agent_deadline_is_supervised_inside_container_and_still_returns_for_scoring(self):
        class Environment:
            async def upload_file(self, source, destination):
                pass

            async def exec(self, *, command, **kwargs):
                if '/opt/nola/nola ' in command:
                    self.command = command
                    self.transport_timeout = kwargs['timeout_sec']
                    return types.SimpleNamespace(return_code=124)
                return types.SimpleNamespace(return_code=0)

        agent = object.__new__(adapter.NolaHarborAgent)
        agent.session_id = 'arbitrary-task__trial__agent'
        agent.expected_artifacts_by_task = {}
        agent.endpoint_host = '192.168.1.2'
        agent.model_name = 'test-model'
        agent.network_policy = 'allow'
        agent.tool_set = 'auto'
        agent.timeout_seconds = 60
        environment = Environment()
        context = types.SimpleNamespace(metadata={})
        asyncio.run(agent.run('Original instruction', environment, context))
        self.assertIn('python3 /opt/nola/supervisor.py --seconds 60 -- /opt/nola/nola', environment.command)
        self.assertEqual(environment.transport_timeout, 75)
        self.assertTrue(context.metadata['nola_timed_out'])
        self.assertFalse(context.metadata['nola_process_succeeded'])
        self.assertTrue(context.metadata['nola_execution_observed'])
        self.assertEqual(context.metadata['nola_exit_code'], 124)


if __name__ == "__main__":
    unittest.main()
