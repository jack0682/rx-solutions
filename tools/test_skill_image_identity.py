#!/usr/bin/env python3
"""Independent archive controls for classic and OCI image identity checks."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("image_identity", ROOT / "deployment/local-skills/image_identity.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ImageIdentity(unittest.TestCase):
    def archive(self, root, corrupt=False):
        config = json.dumps({"architecture": "arm64", "os": "linux", "config": {"Cmd": ["python3"]}}).encode()
        config_id = "sha256:" + hashlib.sha256(config).hexdigest()
        manifest = json.dumps({"schemaVersion": 2, "config": {"digest": config_id}, "layers": []}).encode()
        manifest_id = "sha256:" + hashlib.sha256(manifest).hexdigest()
        descriptor = {"digest": manifest_id, "size": len(manifest)}
        files = {"manifest.json": json.dumps([{"Config": "blobs/sha256/" + config_id[7:]}]).encode(),
                 "index.json": json.dumps({"manifests": [descriptor]}).encode(),
                 "blobs/sha256/" + config_id[7:]: config,
                 "blobs/sha256/" + manifest_id[7:]: manifest + (b" " if corrupt else b"")}
        path = root / "image.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            for name, raw in files.items():
                entry = tarfile.TarInfo(name); entry.size = len(raw)
                archive.addfile(entry, io.BytesIO(raw))
        return path, config_id, manifest_id

    def test_classic_and_containerd_ids_are_both_content_bound(self):
        with tempfile.TemporaryDirectory() as temp:
            path, classic, containerd = self.archive(Path(temp))
            self.assertEqual(module.identities(path, "arm64"), {classic, containerd})
            self.assertNotIn("sha256:" + "00" * 32, module.identities(path, "arm64"))
            with self.assertRaises(ValueError): module.identities(path, "amd64")

    def test_changed_descriptor_bytes_are_rejected(self):
        with tempfile.TemporaryDirectory() as temp:
            path, _, _ = self.archive(Path(temp), corrupt=True)
            with self.assertRaises(ValueError): module.identities(path, "arm64")


if __name__ == "__main__":
    unittest.main()
