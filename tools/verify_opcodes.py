"""Verify that decoder.rs has all 256 opcodes defined."""
import re
from pathlib import Path

DECODER_PATH = Path(__file__).parent.parent / "src" / "cpu" / "decoder.rs"

text = DECODER_PATH.read_text(encoding='utf-8')
# Match both old-style (0xNN =>) and macro-style (0xNN, ...)
matches = re.findall(r'0x([0-9A-Fa-f]{2})[\s\),]', text)
present = sorted({int(h, 16) for h in matches})
missing = [b for b in range(256) if b not in present]

print(f"Decoded: {len(present)} entries")
print(f"Missing: {len(missing)} entries")
if missing:
    print(f"Missing codes: {[f'0x{b:02X}' for b in missing]}")
else:
    print("All 256 opcodes covered!")

# Check for forbidden #[allow(dead_code)] in CPU files
cpu_dir = Path(__file__).parent.parent / "src" / "cpu"
for rust_file in cpu_dir.rglob("*.rs"):
    for i, line in enumerate(rust_file.read_text(encoding='utf-8').splitlines(), 1):
        if 'allow(dead_code)' in line:
            print(f"  WARNING: allow(dead_code) in {rust_file}:{i}: {line.strip()}")
else:
    print("No allow(dead_code) in CPU module!")