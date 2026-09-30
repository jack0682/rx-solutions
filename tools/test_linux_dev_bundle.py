#!/usr/bin/env python3
"""Installer refusal and preservation tests; runtime acceptance is a separate check."""
import hashlib
import importlib.machinery
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
loader = importlib.machinery.SourceFileLoader("rx_dev", str(ROOT / "deployment/linux-dev/rx-dev"))
spec = importlib.util.spec_from_loader(loader.name, loader)
module = importlib.util.module_from_spec(spec)
loader.exec_module(module)
session_spec = importlib.util.spec_from_file_location("rx_dev_session", ROOT / "deployment/linux-dev/session.py")
session_module = importlib.util.module_from_spec(session_spec)
session_spec.loader.exec_module(session_module)


class InstallerBoundaries(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / "bundle"
        self.root.mkdir()
        (self.root / "source.py").write_text("print('source')\n")
        self.manifest = {"schema": "rx.linux-dev-bundle.v1", "profile": "FILE_SIMULATION",
                         "architecture": "amd64", "files": {
                             "source.py": hashlib.sha256((self.root / "source.py").read_bytes()).hexdigest()}}
        (self.root / "bundle.json").write_text(json.dumps(self.manifest))
        self.state = Path(self.temp.name) / "state"
        self.override = patch.object(module, "ROOT", self.root)
        self.override.start()
        self.addCleanup(self.override.stop)

    def test_modified_source_is_refused_before_docker(self):
        (self.root / "source.py").write_text("print('different')\n")
        with patch.object(module, "docker_info") as docker:
            with self.assertRaisesRegex(ValueError, "content changed"):
                module.install(self.state, None)
            docker.assert_not_called()
        self.assertFalse(self.state.exists())

    def test_added_build_input_is_refused(self):
        (self.root / "extra.py").write_text("print('extra')\n")
        with self.assertRaisesRegex(ValueError, "unlisted files"):
            module.verify_bundle()

    def test_symlink_to_identical_source_is_refused(self):
        file = self.root / "source.py"
        outside = Path(self.temp.name) / "outside.py"
        outside.write_bytes(file.read_bytes())
        file.unlink()
        file.symlink_to(outside)
        with self.assertRaisesRegex(ValueError, "Unsafe bundle path"):
            module.verify_bundle()

    def test_other_installation_is_preserved(self):
        self.state.mkdir()
        original = b'{"bundle_sha256":"other-release"}\n'
        (self.state / "installation.json").write_bytes(original)
        with patch.object(module, "docker_info", return_value={}), patch.object(module, "run") as run:
            with self.assertRaisesRegex(ValueError, "another bundle"):
                module.install(self.state, None)
            run.assert_not_called()
        self.assertEqual((self.state / "installation.json").read_bytes(), original)

    def binary_bundle(self):
        archive = self.root / "runtime-images.tar.gz"
        archive.write_bytes(b"test archive")
        self.manifest["files"][archive.name] = hashlib.sha256(archive.read_bytes()).hexdigest()
        self.manifest.update(binary_images_included=True, images={"platform": "sha256:p", "solutions": "sha256:s"})
        (self.root / "bundle.json").write_text(json.dumps(self.manifest))

    def test_binary_install_does_not_build_or_fetch_sources(self):
        self.binary_bundle()
        def command(*args, **kwargs):
            self.assertEqual(args[0], "docker")
            if args[1:3] == ("image", "inspect"):
                return json.dumps([{"Os": "linux", "Architecture": "amd64"}])
            self.assertEqual(args[1:3], ("load", "--input"))
        with patch.object(module, "docker_info") as prerequisites, patch.object(module, "run", side_effect=command):
            module.install(self.state, None)
            prerequisites.assert_called_once_with(needs_git=False)
        self.assertEqual(json.loads((self.state / "installation.json").read_text())["images"], self.manifest["images"])

    def test_wrong_binary_architecture_never_marks_installation_complete(self):
        self.binary_bundle()
        def command(*args, **kwargs):
            return json.dumps([{"Os": "linux", "Architecture": "arm64"}])
        with patch.object(module, "docker_info"), patch.object(module, "run", side_effect=command):
            with self.assertRaisesRegex(ValueError, "architecture differs"):
                module.install(self.state, None)
        self.assertFalse((self.state / "installation.json").exists())

    def test_unrelated_container_cannot_be_inspected_as_owned(self):
        value = [{"Config": {"Labels": {"rx.dev.session": "different-session"}}}]
        with patch.object(module, "run", return_value=json.dumps(value)):
            with self.assertRaisesRegex(ValueError, "unrelated container"):
                module.inspect_owned("existing-container", "expected-session")

    def test_session_path_cannot_escape_state(self):
        with self.assertRaises(ValueError):
            module.sessions(self.state, "../../existing")

    def test_stop_requires_explicit_session(self):
        with self.assertRaisesRegex(ValueError, "no global stop"):
            module.stop(self.state, None)

    def test_shutdown_refuses_foreign_container_without_signalling(self):
        folder = self.state / "12345678-1234-1234-1234-123456789abc"
        folder.mkdir(parents=True)
        (folder / "session.json").write_text(json.dumps({"containers": ["foreign"]}))
        value = [{"Config": {"Labels": {"rx.dev.session": "another"}},
                  "State": {"Running": True, "ExitCode": 0}}]
        with patch.object(session_module.subprocess, "check_output", return_value=json.dumps(value)), \
                patch.object(session_module.subprocess, "run") as command:
            results = session_module.shutdown(folder)
            command.assert_not_called()
        self.assertIn("unrelated container", results[0]["error"])

    def test_shutdown_orders_services_and_never_deletes_volumes(self):
        folder = self.state / "12345678-1234-1234-1234-123456789abc"
        folder.mkdir(parents=True)
        prefix = "rx-dev-" + folder.name[:12]
        names = [prefix + "-" + suffix for suffix in ["p", "h", "e"]]
        (folder / "session.json").write_text(json.dumps({"containers": names, "volumes": ["retained"]}))
        running = set(names)
        def inspect(args, **kwargs):
            return json.dumps([{"Config": {"Labels": {"rx.dev.session": folder.name}},
                                "State": {"Running": args[-1] in running, "ExitCode": 0}}])
        def signal(args, **kwargs):
            self.assertEqual(args[:4], ["docker", "kill", "--signal", "TERM"])
            running.remove(args[-1])
        with patch.object(session_module.subprocess, "check_output", side_effect=inspect), \
                patch.object(session_module.subprocess, "run", side_effect=signal) as command:
            results = session_module.shutdown(folder)
        self.assertEqual([v.args[0][-1] for v in command.call_args_list],
                         [prefix + "-" + suffix for suffix in ["e", "p", "h"]])
        self.assertTrue(all(not value["running"] for value in results))
        self.assertEqual(json.loads((folder / "session.json").read_text())["volumes"], ["retained"])


if __name__ == "__main__":
    unittest.main()
