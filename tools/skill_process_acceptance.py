"""Real installed-worker acceptance for serial skill composition, not hardware."""
import json
from pathlib import Path
import shutil
import subprocess
import time
import uuid


def exercise(bundle: Path, root: Path, call, http, installation: dict):
    example = bundle / "examples/material-flow"
    for name in ("acquire", "load", "release", "unload"):
        call("skill", "add", str(example / name))
    call("process", "add", str(example))
    request_id = str(uuid.uuid4())
    inputs = '{"part":"part-one","present":true}'
    first = json.loads(call("process", "run", "material-flow", "--input", inputs, "--request-id", request_id))
    assert first["status"] == "SUCCEEDED" and first["output"] == {"part": "part-one", "location": "output-bin"}
    assert [step["step"] for step in first["steps"]] == ["acquire", "load", "release", "unload"]
    for step in first["steps"]:
        execution = step["execution"]
        assert execution["request"]["input"]["part"] == "part-one"
        assert execution["parent"] == {"process_run": request_id, "step": step["step"]}
        assert execution["package_digest"] == step["package_digest"]
    assert first["steps"][1]["execution"]["request"]["input"]["holding"] is True
    assert first["steps"][2]["execution"]["request"]["input"]["clamped"] is True
    assert first["steps"][3]["execution"]["request"]["input"]["released"] is True
    assert json.loads(call("process", "run", "material-flow", "--input", inputs, "--request-id", request_id)) == first
    logs = subprocess.check_output(["docker", "logs", installation["name"] + "-worker"], text=True)
    for step in first["steps"]:
        assert logs.count(step["request_id"]) == 1, (step, logs)
    second = json.loads(call("process", "run", "material-flow", "--input", '{"part":"part-two","present":true}'))
    assert second["output"]["part"] == "part-two"
    assert all(step["execution"]["request"]["input"]["part"] == "part-two" for step in second["steps"])
    failed = json.loads(call("process", "run", "material-flow", "--input", '{"part":"missing","present":false}', code=2))
    assert failed["status"] == "FAILED" and failed["output"] is None
    assert failed["steps"][0]["execution"]["operation"]["outcome"] == "FAILED"
    assert all(step["execution"] is None for step in failed["steps"][1:])
    future = failed["steps"][1]
    http("/v1/runs", {"request_id": future["request_id"], "skill": "load", "version": "1.0.0", "input": {"part": "injected", "holding": True}}, expected=409)
    changed = dict(first["request"], input={"part": "changed", "present": True})
    http("/v1/process-runs", changed, expected=409)

    slow = root / "slow-acquire"
    shutil.copytree(example / "acquire", slow)
    manifest = json.loads((slow / "skill.json").read_text())
    manifest.update(version="2.0.0", timeout_ms=100)
    (slow / "skill.json").write_text(json.dumps(manifest))
    (slow / "skill.py").write_text("import time\ndef main(inputs):\n    time.sleep(2)\n    return {'part': inputs['part'], 'holding': True}\n")
    call("skill", "add", str(slow))
    variant = json.loads((example / "process.json").read_text())
    variant["version"] = "2.0.0"
    variant["bindings"]["acquire"]["version"] = "2.0.0"
    (slow / "process.json").write_text(json.dumps(variant))
    call("process", "add", str(slow))
    unknown = json.loads(call("process", "run", "material-flow", "--version", "2.0.0", "--input", '{"part":"uncertain","present":true}', code=2))
    assert unknown["status"] == "UNKNOWN" and all(s["execution"] is None for s in unknown["steps"][1:])
    metrics = json.loads(call("metrics"))
    groups = {(g["skill"], g["version"]): g for g in metrics["groups"]}
    acquire = groups[("acquire", "1.0.0")]
    assert acquire["counts"]["succeeded"] == 2 and acquire["counts"]["failed"] == 1
    assert acquire["decided_denominator"] == 3
    uncertain = groups[("acquire", "2.0.0")]
    assert uncertain["counts"]["unknown"] == 1 and uncertain["success_fraction_of_decided"] is None
    assert groups[("unload", "1.0.0")]["counts"]["admitted"] == 2
    # Kill both actual services while the second step is executing. The first
    # result and every reserved child identity must survive without replay.
    slow_load = root / "slow-load"
    shutil.copytree(example / "load", slow_load)
    manifest = json.loads((slow_load / "skill.json").read_text())
    manifest.update(version="2.0.0", timeout_ms=20000)
    (slow_load / "skill.json").write_text(json.dumps(manifest))
    (slow_load / "skill.py").write_text("import time\ndef main(inputs):\n    time.sleep(10)\n    return {'part': inputs['part'], 'clamped': True}\n")
    call("skill", "add", str(slow_load))
    variant = json.loads((example / "process.json").read_text())
    variant["version"] = "3.0.0"
    variant["bindings"]["load"]["version"] = "2.0.0"
    (slow_load / "process.json").write_text(json.dumps(variant))
    call("process", "add", str(slow_load))
    restart_id = str(uuid.uuid4())
    args = ("process", "run", "material-flow", "--version", "3.0.0", "--input", '{"part":"part-restart","present":true}', "--request-id", restart_id, "--no-wait")
    original = json.loads(call(*args))
    deadline = time.monotonic() + 10
    while True:
        current = http("/v1/process-runs/" + restart_id)
        second_step = current["steps"][1]["execution"]
        if second_step is not None and second_step["started_ms"] is not None:
            break
        assert time.monotonic() < deadline, current
        time.sleep(.1)
    completed_first = current["steps"][0]["execution"]
    assert completed_first["operation"]["outcome"] == "SUCCEEDED"
    for role in ("server", "worker"):
        subprocess.run(["docker", "kill", installation["name"] + "-" + role], check=True, capture_output=True)
    call("up")
    restarted = json.loads(call(*args))
    assert restarted["status"] == "UNKNOWN"
    assert [s["request_id"] for s in restarted["steps"]] == [s["request_id"] for s in original["steps"]]
    assert restarted["steps"][0]["execution"] == completed_first
    assert all(s["execution"] is None for s in restarted["steps"][2:])
    logs = subprocess.check_output(["docker", "logs", installation["name"] + "-worker"], text=True)
    assert logs.count(completed_first["request"]["request_id"]) == 1
    assert logs.count(restarted["steps"][1]["request_id"]) == 1
    return {"first": first, "second": second, "failed": failed, "unknown": unknown, "metrics": metrics, "restarted": restarted}
