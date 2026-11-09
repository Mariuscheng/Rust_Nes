import sys

def usage():
    print("usage: dump_prg.py <rom> <cpu_addr_hex> <count>")
    sys.exit(1)

if len(sys.argv) < 4:
    usage()

rom = sys.argv[1]
try:
    addr = int(sys.argv[2], 0)
    count = int(sys.argv[3], 0)
except Exception:
    usage()

with open(rom, 'rb') as f:
    b = f.read()

if len(b) < 16 or b[0:4] != b'NES\x1a':
    print('Not an iNES file or too small')
    sys.exit(1)

prg_units = b[4]
skip_trainer = (b[6] & 0x04) != 0
prg_start = 16 + (512 if skip_trainer else 0)
prg_size = prg_units * 16384
mapper_id = (b[7] & 0xF0) | (b[6] >> 4)
rel = addr - 0x8000

print(f'rom={rom} mapper={mapper_id} prg_units={prg_units} prg_size={prg_size} prg_start={prg_start} rel={rel}')

if rel < 0 or rel >= prg_size:
    print('Requested CPU addr out of PRG range')
    sys.exit(0)

end_rel = min(rel + count, prg_size)
for offset in range(rel, end_rel):
    cpu_addr = 0x8000 + offset
    val = b[prg_start + offset]
    print(f'{cpu_addr:04X}: {val:02X}')
