"""Verify image identifiers from the checksum-bound Docker/OCI archive.

Classic Docker reports a config digest; containerd can report an OCI manifest or
index digest. Accept only verified objects that lead to the same image config.
"""
import hashlib
import json
import re
import tarfile


def identities(path, architecture):
    with tarfile.open(path) as archive:
        def read(name):
            member = archive.getmember(name)
            if not member.isfile() or member.size > 1_048_576:
                raise ValueError("invalid image metadata")
            return archive.extractfile(member).read()

        manifests = json.loads(read("manifest.json"))
        if len(manifests) != 1:
            raise ValueError("one image per release archive required")
        config_path = manifests[0]["Config"]
        match = re.fullmatch(r"(?:blobs/sha256/)?([0-9a-f]{64})(?:\.json)?", config_path)
        if not match:
            raise ValueError("invalid image config reference")
        raw = read(config_path)
        config_digest = "sha256:" + hashlib.sha256(raw).hexdigest()
        if config_digest != "sha256:" + match[1]:
            raise ValueError("image config digest mismatch")
        config = json.loads(raw)
        if config["architecture"] != architecture or config["os"] != "linux":
            raise ValueError("image config target mismatch")
        allowed = {config_digest}
        visited = set()

        def visit(reference, depth=0):
            digest = reference["digest"]
            if depth > 4 or len(visited) > 16 or not re.fullmatch(r"sha256:[0-9a-f]{64}", digest):
                raise ValueError("invalid image descriptor graph")
            if digest in visited:
                return digest in allowed
            visited.add(digest)
            data = read("blobs/sha256/" + digest[7:])
            if len(data) != reference["size"] or hashlib.sha256(data).hexdigest() != digest[7:]:
                raise ValueError("image descriptor content mismatch")
            value = json.loads(data)
            if "manifests" in value:
                matches = [visit(child, depth + 1) for child in value["manifests"]]
                matched = any(matches)
            else:
                matched = value.get("config", {}).get("digest") == config_digest
            if matched:
                allowed.add(digest)
            return matched

        try:
            index = json.loads(read("index.json"))
        except KeyError:
            index = {"manifests": []}
        for reference in index["manifests"]:
            visit(reference)
        return allowed
