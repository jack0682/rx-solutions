"""Definition package client. The platform alone resolves values and generates points."""
import hashlib
import http.cookiejar
import ipaddress
import json
from pathlib import Path
import re
import urllib.request
import urllib.parse
import uuid

from runtime_client import (Terminal, RuntimeClient, RuntimeRejected, RedirectRefused,
                            encoded, read_file, save_new)


class LocalAuthoring(Terminal):
    """Explicit loopback development transport; registered-terminal TLS remains unchanged."""
    def __init__(self, url, public_origin, principal, password_file):
        for value in (url, public_origin):
            parsed = urllib.parse.urlsplit(value)
            if (parsed.scheme != "http" or not parsed.hostname or parsed.username or parsed.password
                    or parsed.path or parsed.query or parsed.fragment):
                raise ValueError("exact loopback HTTP origin required for local authoring")
            if parsed.hostname != "localhost" and not ipaddress.ip_address(parsed.hostname).is_loopback:
                raise ValueError("local authoring refuses non-loopback endpoints")
        self.origin, self.public_origin, self.principal = url, public_origin, principal
        self.fingerprint = hashlib.sha256(encoded([url, public_origin, principal, "LOCAL_DEVELOPMENT"])).hexdigest()
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}),
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), RedirectRefused())
        password = read_file(password_file.absolute(), secret=True).decode().rstrip("\r\n")
        profile = self.request("/api/v1/session", {"principal": principal, "password": password})
        if profile.get("principal") != principal or profile.get("terminal") is not None:
            raise ValueError("local authoring identity differs")

    def request(self, path, body=None):
        if not getattr(self, "_origin_handler", False):
            class OriginHeader(urllib.request.BaseHandler):
                def http_request(handler, request):
                    request.add_unredirected_header("Host", urllib.parse.urlsplit(self.public_origin).netloc)
                    if request.data is not None:
                        request.add_header("Origin", self.public_origin)
                    return request
                https_request = http_request
            self.opener.add_handler(OriginHeader())
            self._origin_handler = True
        return super().request(path, body)


def reference(value):
    if not isinstance(value, dict) or set(value) != {"catalog", "id", "revision", "digest"}:
        raise ValueError("exact definition reference required")
    for key in ("id", "catalog"):
        if str(uuid.UUID(value[key])) != value[key]:
            raise ValueError("canonical UUID required")
    if not re.fullmatch(r"[1-9][0-9]*", value["revision"]) or not re.fullmatch(r"[0-9a-f]{64}", value["digest"]):
        raise ValueError("invalid definition revision/digest")
    return value


class Definitions(RuntimeClient):
    def show(self, ref, latest=False):
        reference(ref)
        query = {"catalog": ref["catalog"], "id": ref["id"]}
        if not latest:
            query["revision"] = ref["revision"]
        value = self.terminal.get("/api/v1/definition", **query)
        got = reference(value["version"]["definition"]["reference"])
        if got["catalog"] != ref["catalog"] or got["id"] != ref["id"] or (not latest and got != ref):
            raise ValueError("definition response differs from the requested reference")
        return value

    def conflict_detail(self, command, key):
        try:
            view = self.terminal.get('/api/v1/definition', catalog=command['catalog'], id=command['id'])
            definition = view['version']['definition']; current = reference(definition['reference'])
            if current['catalog'] != command['catalog'] or current['id'] != command['id']:
                return None
            label = json.dumps(definition['label'], ensure_ascii=False)
            prefix = f"{key}: current read is {label} r{current['revision']}. "
            if command['expected'] is None:
                return prefix + f"expected=null is create-only, but this ID already exists. Use a new ID to create, or inspect the existing definition and explicitly set expected=\"{current['revision']}\" to revise it."
            return prefix + f"The request expected r{command['expected']}. Inspect the current definition before submitting a new revision."
        except (OSError, ValueError, KeyError, TypeError):
            return None

    def apply(self, package, imports, request_id):
        if (not isinstance(package, dict) or set(package) != {"schema", "catalog", "title", "definitions"}
                or package["schema"] != "rx.definition-package.v1"
                or not isinstance(package["definitions"], list) or not 1 <= len(package["definitions"]) <= 256):
            raise ValueError("definition package schema or size differs")
        catalog = str(uuid.UUID(package["catalog"]))
        if catalog != package["catalog"]:
            raise ValueError("canonical catalog UUID required")
        keys, ids = set(), set()
        for entry in package["definitions"]:
            if (set(entry) != {"key", "id", "label", "expected", "body"}
                    or not re.fullmatch(r"[a-zA-Z0-9_.-]{1,100}", entry["key"])
                    or entry["key"] in keys or entry["id"] in ids):
                raise ValueError("invalid or duplicate package definition")
            keys.add(entry["key"]); ids.add(entry["id"])
        refs = dict(imports)
        if any(reference(v)["catalog"] != catalog for v in refs.values()):
            raise ValueError("package references belong to another catalog")
        with self.journal(request_id) as root:
            save_new(root / "intent.json", {"package": package, "imports": imports,
                                            "connection": self.terminal.fingerprint})

            def mutate(stage, path, command):
                body = {"request_key": str(uuid.uuid5(uuid.UUID(request_id), "definition." + stage)), "command": command}
                save_new(root / (stage + ".request.json"), {"path": path, "body": body})
                # Always consult the server, including replay: revoked access must not use a cached success.
                try:
                    return self.terminal.request(path, body)
                except RuntimeRejected as error:
                    if (path == '/api/v1/definitions' and error.status == 409
                            and isinstance(error.body, dict) and error.body.get('code') == 'STALE_REVISION'):
                        detail = self.conflict_detail(command, stage)
                        if detail:
                            error.args = (str(error) + "; " + detail,)
                    raise

            try:
                current = self.terminal.get("/api/v1/definition-catalog", id=catalog)
            except RuntimeRejected as exc:
                if exc.status != 404:
                    raise
                profile = self.terminal.get("/api/v1/session")
                terminal = profile.get("terminal")
                terminal_id = terminal["id"] if isinstance(terminal, dict) else terminal
                current = mutate("catalog", "/api/v1/definition-catalogs", {
                    "id": catalog, "expected": None, "title": package["title"], "members": {},
                    "terminals": [terminal_id] if terminal_id else [], "archived": False})
            if current.get("id") != catalog:
                raise ValueError("catalog response differs")
            checked = set()

            def bind(value):
                if isinstance(value, dict):
                    if set(value) == {"$ref"}:
                        key = value["$ref"]
                        if key not in refs:
                            raise ValueError("reference must precede its use: " + str(key))
                        if key not in checked:
                            self.show(refs[key]); checked.add(key)
                        return refs[key]
                    return {key: bind(v) for key, v in value.items()}
                if isinstance(value, list):
                    return [bind(v) for v in value]
                return value

            applied = {}
            for entry in package["definitions"]:
                command = {"catalog": catalog, "id": entry["id"], "expected": entry["expected"],
                           "label": entry["label"], "body": bind(entry["body"]), "archived": False}
                result = mutate(entry["key"], "/api/v1/definitions", command)
                d = result["version"]["definition"]
                ref = reference(d["reference"])
                if (ref["catalog"] != catalog or ref["id"] != entry["id"]
                        or ref["revision"] != str(int(entry["expected"] or "0") + 1)
                        or d["body"] != command["body"] or d["label"] != entry["label"]
                        or result["version"]["archived"] is not False
                        or result["version"]["updated_by"] != self.terminal.principal):
                    raise ValueError("definition save response differs: " + entry["key"])
                refs[entry["key"]] = ref; applied[entry["key"]] = ref
                save_new(root / (entry["key"] + ".reply.json"), result)
            return {"schema": "rx.definition-package-receipt.v1", "request_id": request_id,
                    "package_sha256": hashlib.sha256(encoded(package)).hexdigest(),
                    "catalog": catalog, "applied": applied, "references": refs}

    def points(self, subject, rule, offset=0, limit=100, latest=False):
        if latest:
            subject = self.show(subject, True)["version"]["definition"]["reference"]
            rule = self.show(rule, True)["version"]["definition"]["reference"]
        query = {"subject": reference(subject), "rule": reference(rule), "offset": str(offset), "limit": limit}
        value = self.terminal.request("/api/v1/definition-points", query)
        if value.get("subject") != subject or value.get("rule") != rule or value.get("offset") != str(offset):
            raise ValueError("point response differs from requested versions/page")
        return value


def arguments(sub):
    p = sub.add_parser("definitions", help="Apply and inspect versioned platform authoring data")
    modes = p.add_mutually_exclusive_group(required=True)
    modes.add_argument("--connection", type=Path)
    modes.add_argument("--local-url", help="Explicit loopback development API; no device authority")
    p.add_argument("--public-origin")
    p.add_argument("--principal", default="admin")
    p.add_argument("--password-file", type=Path)
    p.add_argument("--state-dir", type=Path, default=Path.home() / ".local/share/rx-definition-client")
    p.add_argument("--references", type=Path)
    p.add_argument("--format", choices=("json", "text"), default="json")
    commands = p.add_subparsers(dest="action", required=True)
    a = commands.add_parser("apply"); a.add_argument("package", type=Path)
    a.add_argument("--request-id"); a.add_argument("--output", type=Path)
    a = commands.add_parser("show"); a.add_argument("name"); a.add_argument("--latest", action="store_true")
    a = commands.add_parser("points"); a.add_argument("name"); a.add_argument("--rule", required=True)
    a.add_argument("--offset", type=int, default=0); a.add_argument("--limit", type=int, default=100)
    a.add_argument("--latest", action="store_true")


def run(args):
    if args.connection:
        terminal = Terminal(args.connection.absolute())
    else:
        if not args.password_file:
            raise ValueError("--password-file is required for local authoring")
        terminal = LocalAuthoring(args.local_url, args.public_origin or args.local_url,
                                  args.principal, args.password_file)
    client = Definitions(terminal, args.state_dir.absolute())
    refs = {}
    if args.references:
        receipt = json.loads(read_file(args.references.absolute()))
        if receipt.get("schema") != "rx.definition-package-receipt.v1":
            raise ValueError("definition package receipt required")
        refs = receipt["references"]
    if args.action == "apply":
        package = json.loads(read_file(args.package.absolute()))
        request_id = args.request_id or str(uuid.uuid5(uuid.NAMESPACE_URL,
            terminal.fingerprint + hashlib.sha256(encoded([package, refs])).hexdigest()))
        result = client.apply(package, refs, request_id)
        if args.output:
            save_new(args.output.absolute(), result)
    else:
        if args.name not in refs:
            raise ValueError("definition name missing from --references receipt")
        if args.action == "show":
            result = client.show(refs[args.name], args.latest)
        else:
            if args.rule not in refs:
                raise ValueError("rule name missing from --references receipt")
            result = client.points(refs[args.name], refs[args.rule], args.offset, args.limit, args.latest)
    return result, format_report(client, result, refs) if args.format == "text" else None


def format_report(client, result, aliases):
    """Readable projection of server values; no value resolution or geometry in this client."""
    if result.get('schema') == 'rx.definition-package-receipt.v1':
        return f"Applied {len(result['applied'])} definitions. Exact references are in the package receipt."
    names = {}
    def name(ref):
        key = encoded(ref)
        if key not in names:
            try:
                definition = client.show(ref)['version']['definition']
                alias = next((k for k,v in aliases.items() if v == ref), None)
                names[key] = (alias + ' / ' if alias else '') + json.dumps(definition['label'],ensure_ascii=False)
            except (ValueError, OSError, KeyError, TypeError):
                names[key] = 'Name unavailable [' + ref['id'] + ']'
        return names[key] + ' · r' + ref['revision']
    lines = []
    if 'version' in result:
        d = result['version']['definition']
        names[encoded(d['reference'])] = json.dumps(d['label'],ensure_ascii=False)
        lines.append(name(d['reference']))
    else:
        lines.extend(['Subject: '+name(result['subject']), 'Rule: '+name(result['rule'])])
    effective = result.get('effective',result.get('inputs',{}))
    for field,spec in effective.get('fields',{}).items():
        value = effective['values'].get(field)
        unit = spec['specification']['unit']
        if value:
            rendered = json.dumps(next(iter(value['value'].values())),ensure_ascii=False)
            lines.append(f"{field}: {rendered} {unit} ← {name(value['declared_by'])}")
        else:
            lines.append(f"{field}: MISSING {unit}")
        lines.append('  Field declared by '+name(spec['declared_by']))
        for previous in effective.get('shadowed',{}).get(field,[]):
            rendered = json.dumps(next(iter(previous['value'].values())),ensure_ascii=False)
            lines.append(f"  Overridden: {rendered} {unit} ← {name(previous['declared_by'])}")
    if 'points' in result:
        lines.extend([f"{result['total']} poses · {result['unit']} · frame {result['frame']}",
                      'Orientation [x,y,z,w]: '+json.dumps(result['orientation_xyzw'])])
        for point in result['points']:
            lines.append(f"{point['index']}: indices={','.join(point['indices'])}; position={json.dumps(point['position'])}")
        if result['next'] is not None: lines.append('Next offset: '+result['next'])
        for issue in result['violations']: lines.append(f"{issue['location']}: {issue['code']} — {issue['message']}")
    return '\n'.join(lines)
