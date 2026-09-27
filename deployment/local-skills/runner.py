"""Bounded trusted-author computation runner. Not a hostile-code sandbox."""
import contextlib
import json
import resource
import runpy
import sys

resource.setrlimit(resource.RLIMIT_AS, (128 * 1024 * 1024,) * 2)
resource.setrlimit(resource.RLIMIT_FSIZE, (1024 * 1024,) * 2)
resource.setrlimit(resource.RLIMIT_NOFILE, (64,) * 2)
resource.setrlimit(resource.RLIMIT_CPU, (65, 65))
with open(sys.argv[2], encoding="utf-8") as stream:
    request = json.load(stream)
with contextlib.redirect_stdout(sys.stderr):
    module = runpy.run_path(sys.argv[1])
    result = module["main"](request)
print(json.dumps(result, allow_nan=False, separators=(",", ":")), flush=True)
