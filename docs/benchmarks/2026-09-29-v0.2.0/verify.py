"""Read-only integrity and reproducibility check for this archived campaign."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parent
manifest = root / 'SHA256SUMS.txt'
expected = {}
for line in manifest.read_text(encoding='utf-8-sig').splitlines():
    digest, name = line.split('  ', 1)
    path = (root / name).resolve()
    if not path.is_relative_to(root) or name in expected:
        raise SystemExit('Invalid or duplicate manifest path')
    expected[name] = digest
actual = {p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and p != manifest}
if actual != set(expected):
    raise SystemExit('Manifest coverage differs from directory contents')
for name, digest in expected.items():
    if hashlib.sha256((root / name).read_bytes()).hexdigest() != digest:
        raise SystemExit(f'Checksum mismatch: {name}')
with tempfile.TemporaryDirectory(prefix='wardoff-evidence-') as temp:
    copy = Path(temp) / 'campaign'
    shutil.copytree(root, copy)
    result = subprocess.run([sys.executable, str(copy/'analyze.py')], capture_output=True, text=True)
    if result.returncode:
        raise SystemExit('Analysis failed: ' + result.stderr)
    for name in ['summary.csv', 'summary.json', 'validation.json', 'RAPPORT.md']:
        if (root/name).read_text(encoding='utf-8') != (copy/name).read_text(encoding='utf-8'):
            raise SystemExit(f'Recomputed artifact differs: {name}')
print(f'PASS: {len(expected)} file hashes verified; statistics and report reproduced; stored evidence unchanged.')
