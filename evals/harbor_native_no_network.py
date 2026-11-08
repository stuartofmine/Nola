# SPDX-License-Identifier: Apache-2.0
"""Harbor Docker provider with a fail-closed native no-network fallback."""

from __future__ import annotations

import tempfile
from pathlib import Path
from typing import Any, Sequence

from harbor.environments.capabilities import EnvironmentCapabilities
from harbor.environments.docker.docker import DockerEnvironment
from harbor.models.task.config import NetworkMode, NetworkPolicy


class DockerNativeNoNetworkEnvironment(DockerEnvironment):
    """Use Docker's network namespace for static no-network environments."""

    def __init__(
        self,
        *args: Any,
        environment_dir: Path,
        network_policy: NetworkPolicy | None = None,
        phase_network_policies: Sequence[NetworkPolicy] = (),
        extra_docker_compose: Sequence[Path | str] | None = None,
        **kwargs: Any,
    ) -> None:
        startup = network_policy or NetworkPolicy(network_mode=NetworkMode.PUBLIC)
        phases = list(phase_network_policies)
        policies = [startup, *phases]
        non_public = [policy for policy in policies if policy.network_mode != NetworkMode.PUBLIC]
        self._native_no_network = bool(non_public) and all(
            policy.network_mode == NetworkMode.NO_NETWORK for policy in policies
        )
        self._native_compose_dir: tempfile.TemporaryDirectory[str] | None = None

        if non_public and not self._native_no_network:
            raise ValueError(
                "Docker native isolation supports only an environment whose "
                "baseline and every phase are no-network"
            )

        overlays = list(extra_docker_compose or [])
        if self._native_no_network:
            if (environment_dir / "docker-compose.yaml").exists() or overlays:
                raise ValueError(
                    "Docker native isolation cannot prove full isolation for "
                    "a task-defined or run-provided multi-service Compose environment"
                )
            self._native_compose_dir = tempfile.TemporaryDirectory()
            overlay = Path(self._native_compose_dir.name) / "docker-native-no-network.yaml"
            overlay.write_text(
                "services:\n  main:\n    network_mode: none\n",
                encoding="utf-8",
            )
            overlays.append(overlay)

        public = NetworkPolicy(network_mode=NetworkMode.PUBLIC)
        super().__init__(
            *args,
            environment_dir=environment_dir,
            network_policy=public if self._native_no_network else startup,
            phase_network_policies=(
                [public for _ in phases] if self._native_no_network else phases
            ),
            extra_docker_compose=overlays,
            **kwargs,
        )
        if self._native_no_network:
            self._network_policy = startup
            self._phase_network_policies = phases

    @property
    def capabilities(self) -> EnvironmentCapabilities:
        capabilities = super().capabilities
        if not self._native_no_network:
            return capabilities
        return capabilities.model_copy(
            update={
                "disable_internet": True,
                "dynamic_network_policy": False,
            }
        )

    async def _apply_network_policy(self, network_policy: NetworkPolicy) -> None:
        if self._native_no_network:
            if network_policy.network_mode == NetworkMode.NO_NETWORK:
                return
            raise ValueError(
                "Docker native no-network environments cannot change network policy"
            )
        await super()._apply_network_policy(network_policy)

    async def stop(self, delete: bool) -> None:
        try:
            await super().stop(delete)
        finally:
            if self._native_compose_dir is not None:
                self._native_compose_dir.cleanup()
                self._native_compose_dir = None
