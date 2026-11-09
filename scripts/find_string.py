import sys

s = b"There should be a one-byte buffer that's filled immediately if empty"
fname = sys.argv[1] if len(sys.argv) > 1 else "rom.nes"

try:
    with open(fname, 'rb') as f:
        data = f.read()
except Exception as e:
    print(f"ERROR: cannot open {fname}: {e}")
    sys.exit(2)

occurs = []
start = 0
while True:
    idx = data.find(s, start)
    if idx == -1:
        break
    occurs.append(idx)
    start = idx + 1

if not occurs:
    print("NOT FOUND")
    sys.exit(0)

print(f"Found {len(occurs)} occurrence(s) of the assertion string in {fname}:")

if len(data) < 16 or data[0:4] != b'NES\x1a':
    print("WARNING: file is not a valid iNES ROM or is too small")

prg_units = data[4]
skip_trainer = (data[6] & 0x04) != 0
prg_start = 16 + (512 if skip_trainer else 0)
prg_size = prg_units * 16384
print(f"PRG units={prg_units} prg_start={prg_start} prg_size={prg_size}")

for idx in occurs:
    in_prg = (prg_start <= idx < prg_start + prg_size)
    if in_prg:
        rel = idx - prg_start
        cpu_addr = 0x8000 + rel
        print(f"- file_offset={idx} (in PRG): rel={rel} => CPU addr={hex(cpu_addr)}")
    else:
        print(f"- file_offset={idx} (outside PRG): offset={idx}")

# Print a small hex preview around first occurrence
idx = occurs[0]
start = max(idx - 32, 0)
end = min(idx + len(s) + 32, len(data))
chunk = data[start:end]
print('\nHex dump around first occurrence (file offsets):')
print(' '.join(f"{b:02X}" for b in chunk))
print('\nASCII view around first occurrence:')
print(''.join((chr(b) if 32 <= b <= 126 else '.') for b in chunk))
