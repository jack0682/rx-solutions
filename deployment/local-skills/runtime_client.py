"""Skill facade over existing registered-terminal P APIs.

The journal owns request identity, not outcomes or operating permission. Execution
and resource release remain in P/Host/Executor. No database seeding or local success.
"""
import contextlib
import fcntl
import hashlib
import http.cookiejar
import json
import os
from pathlib import Path
import ssl
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


class RuntimeRejected(ValueError):
    def __init__(self, status, body):
        super().__init__("P rejected request (HTTP " + str(status) + "): " + json.dumps(body)[:2400])
        self.status, self.body = status, body


class RedirectRefused(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False, allow_nan=False).encode()


def read_file(path, secret=False):
    path = Path(path)
    if not path.is_absolute() or path.is_symlink() or not path.is_file() or path.stat().st_size > 1_048_576:
        raise ValueError("regular absolute configuration file required")
    if secret and path.stat().st_mode & 0o077:
        raise ValueError("private configuration file requires owner-only permissions")
    return path.read_bytes()


def save_new(path, value):
    raw = encoded(value)
    if path.exists():
        if path.read_bytes() != raw:
            raise ValueError("original runtime request content cannot be replaced")
        return
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(raw); stream.flush(); os.fsync(stream.fileno())
    directory = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


def counter(value):
    if not isinstance(value, str) or not value.isascii() or not value.isdigit() or str(int(value)) != value or int(value) >= 2**64:
        raise ValueError("invalid P counter")
    return int(value)


def scope(installation, live=False):
    fields = ["id", "store_generation"] + (["runtime_boot", "clock_id"] if live else [])
    return {key: installation[key] for key in fields}


class Terminal:
    def __init__(self, config_file):
        c = json.loads(read_file(config_file, secret=True))
        required = {"schema", "origin", "ca", "certificate", "private_key", "principal", "password_file"}
        if set(c) != required or c["schema"] != "rx.runtime-skill-connection.v1":
            raise ValueError("runtime connection schema differs")
        origin = urllib.parse.urlsplit(c["origin"])
        if origin.scheme != "https" or not origin.hostname or origin.username or origin.password or origin.path or origin.query or origin.fragment:
            raise ValueError("exact HTTPS P origin required")
        ca = read_file(c["ca"])
        certificate = read_file(c["certificate"])
        read_file(c["private_key"], secret=True)
        password = read_file(c["password_file"], secret=True).decode().removesuffix("\n").removesuffix("\r")
        tls = ssl.create_default_context(cafile=c["ca"])
        tls.load_cert_chain(c["certificate"], c["private_key"])
        self.origin, self.principal = c["origin"], c["principal"]
        self.fingerprint = hashlib.sha256(encoded({"origin": self.origin, "principal": self.principal,
            "ca": hashlib.sha256(ca).hexdigest(), "certificate": hashlib.sha256(certificate).hexdigest()})).hexdigest()
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
            urllib.request.HTTPSHandler(context=tls), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), RedirectRefused())
        profile = self.request("/api/v1/session", {"principal": self.principal, "password": password})
        # Credentials are never written into the request journal.
        del password
        if profile.get("principal") != self.principal or profile.get("terminal") is None:
            raise ValueError("registered-terminal identity is required")

    def request(self, path, body=None):
        if not path.startswith("/api/v1/") or path.startswith("//"):
            raise ValueError("P API path required")
        headers = {"Accept": "application/json"}
        if body is not None:
            headers.update({"Content-Type": "application/json", "Origin": self.origin, "X-RX-Client": "browser-v1"})
        req = urllib.request.Request(self.origin + path, None if body is None else encoded(body), headers)
        try:
            with self.opener.open(req, timeout=20) as reply:
                raw = reply.read(1_048_577)
                if len(raw) > 1_048_576:
                    raise ValueError("P response exceeds supported size")
                return json.loads(raw)
        except urllib.error.HTTPError as exc:
            raw = exc.read(1_048_576)
            try: body = json.loads(raw)
            except (ValueError, UnicodeError): body = {"unreadable_response_sha256": hashlib.sha256(raw).hexdigest()}
            raise RuntimeRejected(exc.code, body) from None

    def get(self, path, **query):
        return self.request(path + ("?" + urllib.parse.urlencode(query) if query else ""))


class RuntimeClient:
    def __init__(self, terminal, state_directory):
        self.terminal = terminal
        self.state = Path(state_directory)
        self.state.mkdir(parents=True, exist_ok=True, mode=0o700)

    def catalog(self):
        result = self.terminal.get("/api/v1/runtime-skills")
        if result.get("schema") != "rx.runtime-skill-catalog.v1":
            raise ValueError("runtime skill catalog schema differs")
        return result

    def inspect(self, run):
        result = self.terminal.get("/api/v1/runtime-skill-result", run=str(uuid.UUID(run)))
        if result.get("schema") != "rx.runtime-skill-result.v1" or result.get("result_owner") != "PLATFORM" or result["run"]["value"]["id"] != run:
            raise ValueError("P run result correlation differs")
        return result

    @contextlib.contextmanager
    def journal(self, request_id):
        identifier = str(uuid.UUID(request_id))
        if identifier != request_id:
            raise ValueError("canonical request UUID required")
        root = self.state / identifier
        root.mkdir(mode=0o700, exist_ok=True)
        fd = os.open(root / "writer.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield root
        except BlockingIOError:
            raise ValueError("this runtime request is already being handled") from None
        finally:
            os.close(fd)

    def mutation(self, root, stage, request_id, path, command=None):
        request_file = root / (stage + ".request.json")
        if request_file.exists():
            saved = json.loads(request_file.read_bytes())
            if saved["path"] != path or (command is not None and saved["body"]["command"] != command):
                raise ValueError("stored runtime request differs")
        else:
            if command is None:
                raise ValueError("original mutation body missing")
            saved = {"path": path, "body": {"request_key": str(uuid.uuid5(uuid.UUID(request_id), "rx.runtime-skill." + stage)), "command": command}}
            save_new(request_file, saved)
        response = root / (stage + ".reply.json")
        if response.exists():
            return json.loads(response.read_bytes())
        value = self.terminal.request(saved["path"], saved["body"])
        save_new(response, value)
        return value

    def invoke(self, request_id, name, cell, count=1, wait_seconds=60):
        if not isinstance(count, int) or isinstance(count, bool) or count < 1:
            raise ValueError("positive material count required")
        with self.journal(request_id) as root:
            intent = {"request_id": request_id, "name": name, "cell": cell, "count": count, "connection": self.terminal.fingerprint}
            save_new(root / "intent.json", intent)
            catalog = self.catalog()
            basis_file = root / "basis.json"
            if basis_file.exists():
                basis = json.loads(basis_file.read_bytes())
                if scope(catalog["installation"]) != scope(basis["installation"]):
                    raise ValueError("installation/store generation changed; original request retained")
            else:
                matches = [v for v in catalog["bindings"] if v["binding"]["name"] == name and v["binding"]["cell"] == cell]
                if len(matches) != 1:
                    raise ValueError("installed skill binding not found or catalog truncated")
                selected = matches[0]
                if selected["binding"]["input_mode"] != "BOUND_CONFIGURATION" or count > counter(selected["binding"]["maximum_budget"]):
                    raise ValueError("unsupported invocation or count exceeds installed envelope")
                basis = {"installation": catalog["installation"], "selected": selected}
                save_new(basis_file, basis)
            bound = basis["selected"]["binding"]
            if not (root / "start.reply.json").exists() and scope(catalog["installation"], True) != scope(basis["installation"], True):
                raise ValueError("P runtime/clock changed with a pending request; inspect/reconcile the original run without replay")
            create_command = {"cell": cell, "recipe_digest": bound["recipe"]["sha256"], "site_config_digest": bound["site_config_digest"], "expected_cell": basis["selected"]["cell_revision"]}
            created = self.mutation(root, "prepare", request_id, "/api/v1/runs", create_command)
            run_id = str(uuid.UUID(created["id"]))
            if created["cell"] != cell or created["recipe_digest"] != bound["recipe"]["sha256"] or created["envelope_digest"] != bound["envelope"]["sha256"]:
                raise ValueError("P prepare receipt differs from selected skill")
            if not (root / "start.request.json").exists():
                context = self.terminal.get("/api/v1/run/start-context", cell=cell, run=run_id, purpose="PRODUCTION", budget_limit=str(count))
                if scope(context["installation"], True) != scope(basis["installation"], True) or context["cell"] != cell or context["run"]["id"] != run_id or context["recipe"]["sha256"] != bound["recipe"]["sha256"] or context["site_config_digest"] != bound["site_config_digest"]:
                    raise ValueError("P start context changed from selected skill")
                command = context["request"]
                if command["run"] != run_id or command["purpose"] != "PRODUCTION" or command["budget_unit"] != "PART_ATTEMPT" or command["budget_limit"] != str(count) or command["envelope_digest"] != bound["envelope"]["sha256"] or command["expected_cell"] != context["cell_revision"] or command["expected_run"] != context["run_revision"]:
                    raise ValueError("P start candidate scope differs")
                if context["can_request"] is not True:
                    raise ValueError("P has not admitted start: " + json.dumps(context["blocking_reason"]))
                attempt = self.mutation(root, "start", request_id, "/api/v1/runs/start", command)
            else:
                attempt = self.mutation(root, "start", request_id, "/api/v1/runs/start")
            # P stores the post-prepare Run revision in the arm attempt.
            original_start = json.loads((root / "start.request.json").read_bytes())["body"]["command"]
            if attempt["run"] != run_id or attempt["cell"] != cell or attempt["actor"] != self.terminal.principal or attempt["expected_cell_revision"] != original_start["expected_cell"] or counter(attempt["expected_run_revision"]) != counter(original_start["expected_run"]) + 1:
                raise ValueError("P start receipt correlation differs")
            deadline = time.monotonic() + max(0, wait_seconds)
            while True:
                result = self.inspect(run_id)
                if scope(result["installation"]) != scope(basis["installation"]) or result["binding"]["binding_digest"] != bound["binding_digest"]:
                    raise ValueError("P result is not the original installed skill")
                state = result["run"]["value"]["state"]
                if state not in ("PREPARED", "EXECUTING") or time.monotonic() >= deadline:
                    return {"request_id": request_id, "run_id": run_id, "start_attempt": attempt["id"], "result": result}
                time.sleep(.2)

    def recover(self, request_id, wait_seconds=60):
        request_id = str(uuid.UUID(request_id))
        intent = json.loads((self.state / request_id / "intent.json").read_bytes())
        return self.invoke(request_id, intent["name"], intent["cell"], intent["count"], wait_seconds)

    def result(self, request_id):
        request_id = str(uuid.UUID(request_id))
        root = self.state / request_id
        intent = json.loads((root / "intent.json").read_bytes())
        if intent["connection"] != self.terminal.fingerprint:
            raise ValueError("runtime connection changed; original request retained")
        created = json.loads((root / "prepare.reply.json").read_bytes())
        basis = json.loads((root / "basis.json").read_bytes())
        value = self.inspect(created["id"])
        if scope(value["installation"]) != scope(basis["installation"]) or value["binding"]["binding_digest"] != basis["selected"]["binding"]["binding_digest"]:
            raise ValueError("runtime result scope differs")
        return value

    def steps(self, cell):
        result = self.terminal.get("/api/v1/process-draft/binding-options", cell=cell)
        if result["cell"] != cell:
            raise ValueError("P binding catalog cell differs")
        return result

    def compose(self, request_id, name, cell, steps):
        if not isinstance(name, str) or not name.strip() or len(name) > 120:
            raise ValueError("process name must have 1-120 characters")
        if not isinstance(steps, list) or not 1 <= len(steps) <= 64 or any(not isinstance(s, str) or not s for s in steps):
            raise ValueError("select 1-64 installed steps in execution order")
        with self.journal(request_id) as root:
            intent = {"kind": "COMPOSE", "request_id": request_id, "name": name,
                      "cell": cell, "steps": steps, "connection": self.terminal.fingerprint}
            save_new(root / "intent.json", intent)
            installation = self.catalog()["installation"]
            basis_file = root / "composition-basis.json"
            if basis_file.exists():
                basis = json.loads(basis_file.read_bytes())
                if scope(installation) != scope(basis["installation"]):
                    raise ValueError("installation/store changed; composition requests retained")
            else:
                catalog = self.steps(cell)
                available = {v["step"] for v in catalog["candidates"]}
                missing = sorted(set(steps) - available)
                if missing:
                    raise ValueError("installed steps not found: " + ", ".join(missing))
                basis = {"installation": installation, "catalog": catalog}
                save_new(basis_file, basis)
            aliases = ["skill/" + str(i + 1) for i in range(len(steps))]
            nodes = [{"id": "sequence", "body": {"kind": "SEQUENCE", "children": aliases}}]
            nodes.extend({"id": alias, "body": {"kind": "OPERATION", "binding": alias}} for alias in aliases)
            source = {"schema": "rx.process-source.v1", "process": name, "entry": "main",
                      "flows": [{"id": "main", "root": "sequence", "nodes": nodes}], "conditions": {}}
            draft_id = str(uuid.uuid5(uuid.UUID(request_id), "rx.runtime-skill.draft"))
            detail = self.mutation(root, "compose-source", request_id, "/api/v1/process-drafts",
                {"id": draft_id, "cell": cell, "expected": None, "title": name, "document": source})
            version = detail["version"]
            if version["id"] != draft_id or version["cell"] != cell or detail["document"] != source:
                raise ValueError("P composition receipt differs")
            if version["validation"]["structurally_valid"] is not True:
                raise ValueError("P rejected process structure: " + json.dumps(version["validation"]))
            selections = dict(zip(aliases, steps))
            bindings = self.mutation(root, "compose-bindings", request_id, "/api/v1/process-draft-bindings",
                {"draft": draft_id, "cell": cell, "source_revision": version["revision"], "expected": None,
                 "catalog_digest": basis["catalog"]["catalog_digest"], "selections": selections})
            if bindings["draft"] != draft_id or bindings["cell"] != cell or bindings["source_revision"] != version["revision"] or bindings["selections"] != selections or bindings["complete"] is not True:
                raise ValueError("P composition bindings are incomplete or differ")
            # Export rechecks the current source, catalog and binding revisions in P.
            # A saved local receipt must never hide a stale server-side composition.
            compiled = self.terminal.get("/api/v1/process-draft-compile-input", cell=cell, id=draft_id,
                source_revision=version["revision"], binding_revision=bindings["revision"])
            if compiled["draft"] != draft_id or compiled["cell"] != cell or compiled["source"] != source or compiled["source_revision"] != version["revision"] or compiled["binding_revision"] != bindings["revision"] or compiled["catalog_digest"] != basis["catalog"]["catalog_digest"] or set(compiled["bindings"]) != set(aliases):
                raise ValueError("P composition export differs")
            return {"status": "DRAFT_READY_FOR_COMPILER", "request_id": request_id, "draft_id": draft_id,
                    "execution_authorized": False, "compile_input": compiled}

    def recover_composition(self, request_id):
        request_id = str(uuid.UUID(request_id))
        intent = json.loads((self.state / request_id / "intent.json").read_bytes())
        if intent.get("kind") != "COMPOSE":
            raise ValueError("request is not a composition")
        return self.compose(request_id, intent["name"], intent["cell"], intent["steps"])
