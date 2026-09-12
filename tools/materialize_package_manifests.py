#!/usr/bin/env python3
import json,sys,xml.etree.ElementTree as ET
from pathlib import Path
source=Path(sys.argv[1]);target=Path(sys.argv[2]);target.mkdir(parents=True,exist_ok=True)
for name,xml in json.loads(source.read_text()).items():
    if not name.replace('_','').isalnum() or ET.fromstring(xml).findtext('name')!=name:raise SystemExit('invalid package manifest identity')
    p=target/name;p.mkdir();(p/'package.xml').write_text(xml)
