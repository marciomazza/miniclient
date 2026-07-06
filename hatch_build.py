import subprocess
from pathlib import Path

from hatchling.builders.hooks.plugin.interface import BuildHookInterface


class CustomBuildHook(BuildHookInterface):
    def initialize(self, version, build_data):
        node_modules = Path(self.root) / "node_modules"
        if not (node_modules / "htmx.org").exists():
            subprocess.run(["npm", "ci"], cwd=self.root, check=True)
