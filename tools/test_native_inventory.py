#!/usr/bin/env python3
import importlib.util,json,subprocess,sys,tempfile
from pathlib import Path
module=Path(__file__).with_name('native_source_inventory.py');spec=importlib.util.spec_from_file_location('inventory',module);inventory=importlib.util.module_from_spec(spec);spec.loader.exec_module(inventory)
with tempfile.TemporaryDirectory() as temporary:
    root=Path(temporary);(root/'driver').mkdir();code=root/'driver/plugin.cpp';code.write_text('original\n')
    lock={'schema':'rx.native-source-inventory.v1','source_tree_sha256':inventory.tree_digest(root)};(root/'source-lock.json').write_text(json.dumps(lock))
    assert subprocess.run([sys.executable,str(module),str(root)],capture_output=True).returncode==0
    code.write_text('changed\n')
    assert subprocess.run([sys.executable,str(module),str(root)],capture_output=True).returncode!=0
    code.write_text('original\n');code.chmod(0o755)
    assert subprocess.run([sys.executable,str(module),str(root)],capture_output=True).returncode!=0
print('Native source mutation and mode drift rejected')
