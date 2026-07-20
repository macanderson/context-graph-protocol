#!/usr/bin/env python3
"""
Validate the Context Graph Protocol wire examples against the JSON Schema.

Usage:
    python3 schema/validate-examples.py

Exits 0 if every message in examples/ is valid under schema/, and 1 otherwise.
No third-party dependencies beyond `jsonschema` (pip install jsonschema).
"""
import json
import sys
from pathlib import Path

try:
    import jsonschema
except ImportError:
    sys.exit("error: this script needs `jsonschema`. Install it: pip install jsonschema")

ROOT = Path(__file__).resolve().parent.parent
SCHEMA = json.loads((ROOT / "schema" / "contextgraph-envelope.schema.json").read_text())
failures = 0


def check(label: str, ok: bool) -> None:
    global failures
    if ok:
        print(f"  PASS  {label}")
    else:
        print(f"  FAIL  {label}")
        failures += 1


print("Validating examples against schema/contextgraph-envelope.schema.json\n")

# 1. NDJSON transcript — one envelope per line.
ndjson_path = ROOT / "examples" / "full-stdio-session.ndjson"
lines = [l for l in ndjson_path.read_text().splitlines() if l.strip()]
for i, line in enumerate(lines, 1):
    try:
        jsonschema.validate(json.loads(line), SCHEMA)
    except (json.JSONDecodeError, jsonschema.ValidationError) as e:
        check(f"{ndjson_path.name} line {i}", False)
        print(f"        {e}")
        continue
    check(f"{ndjson_path.name} line {i} ({json.loads(line)['type']})", True)

# 2. Pretty-printed reference messages — one envelope per array element.
ref_path = ROOT / "examples" / "reference-messages.json"
for i, msg in enumerate(json.loads(ref_path.read_text())):
    try:
        jsonschema.validate(msg, SCHEMA)
    except jsonschema.ValidationError as e:
        check(f"{ref_path.name} message {i} ({msg.get('type')})", False)
        print(f"        {e}")
        continue
    check(f"{ref_path.name} message {i} ({msg['type']})", True)

print(f"\n{'OK — all examples validate' if failures == 0 else f'{failures} failure(s)'}")
sys.exit(1 if failures else 0)
