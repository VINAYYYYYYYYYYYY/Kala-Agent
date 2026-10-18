#!/usr/bin/env python3
"""Package the final CAD, source, checks and engineering review; omit backups."""
import hashlib
import json
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT_FILES = {
    'README.md', 'ENGINEERING_BASIS.md', 'INDEPENDENT_ENGINEERING_REVIEW.md',
    'design_basis.json', 'engineering_results.json', 'engineering_calculations.py',
    'general_arrangement.svg', 'general_arrangement.png', 'build_general_arrangement.py',
    'tool_build.py', 'script_build.py', 'calls.json', 'build_in_kala.FCMacro',
    'present_builds.FCMacro', 'verify_cad.py', 'assembly_verification.json',
    'independent_thread_verification.json', 'session_execution.json', 'package_results.py',
}
paths = []
for path in sorted(HERE.rglob('*')):
    relative = path.relative_to(HERE)
    if not path.is_file() or '__pycache__' in relative.parts:
        continue
    if path.suffix.lower() in {'.fcbak', '.pyc', '.zip'} or path.name == 'failed_validation.json':
        continue
    if relative.parts[0] in {'tool_based', 'script_based'} or str(relative) in ROOT_FILES:
        paths.append(path)
verification = json.loads((HERE / 'assembly_verification.json').read_text())
session = json.loads((HERE / 'session_execution.json').read_text())
assert verification['passed'], 'Final geometry verification must pass before packaging'
assert all(row['status'] == 'completed' for row in session['builds'])
assert not session['model_gateway_attempts'] and session['gateway_restored']
assert session['preserved_documents']
manifest = {'target': '2000 kg vertical screw jack, 120 mm lift',
            'status': 'Engineering prototype; no production release or lifting certification',
            'geometry_checks_passed': True, 'external_kala_model_calls': 0,
            'files': [{'path': str(path.relative_to(HERE)), 'bytes': path.stat().st_size,
                       'sha256': hashlib.sha256(path.read_bytes()).hexdigest()} for path in paths]}
manifest_path = HERE / 'manifest.json'
manifest_path.write_text(json.dumps(manifest, indent=2))
archive = HERE / 'kala_screw_jack_two_builds.zip'
with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as output:
    for path in [*paths, manifest_path]:
        output.write(path, Path(HERE.name) / path.relative_to(HERE))
with zipfile.ZipFile(archive) as check:
    assert check.testzip() is None
print(json.dumps({'archive': str(archive), 'bytes': archive.stat().st_size, 'files': len(paths) + 1,
                  'sha256': hashlib.sha256(archive.read_bytes()).hexdigest()}, indent=2))
