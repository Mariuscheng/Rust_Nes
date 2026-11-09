#!/usr/bin/env python3
import re
import sys
import argparse

parser = argparse.ArgumentParser(description='Parse emulator_run_with_pc.log for PPU write timeline')
parser.add_argument('logfile', nargs='?', default='emulator_run_with_pc.log')
parser.add_argument('-o','--out', default='ppu_timeline.txt')
args = parser.parse_args()

pattern = re.compile(r'Bus: write to PPU PC=(0x[0-9A-Fa-f]+) addr ([0-9A-Fa-f]{4}) reg (\d) data ([0-9A-Fa-f]{2})')

vram_addr = 0
addr_latch = False
curr_inc = 1

out_lines = []

with open(args.logfile, 'r', encoding='utf-8', errors='ignore') as f:
    for lineno, line in enumerate(f, start=1):
        m = pattern.search(line)
        if not m:
            continue
        pc_s, addr_s, reg_s, data_s = m.groups()
        pc = int(pc_s, 16)
        reg = int(reg_s)
        data = int(data_s, 16)

        if reg == 0:
            curr_inc = 32 if (data & 0x04) != 0 else 1
            out_lines.append(f"{lineno:6d}: PC={pc_s} write $2000 PPUCTRL data=0x{data:02X} -> vram_inc={curr_inc}")
        elif reg == 6:
            before = addr_latch
            if not before:
                vram_addr = (vram_addr & 0x00FF) | ((data & 0xFF) << 8)
                typ = 'HIGH'
            else:
                vram_addr = (vram_addr & 0xFF00) | (data & 0xFF)
                typ = 'LOW '
            vram_addr &= 0x3FFF
            addr_latch = not addr_latch
            out_lines.append(f"{lineno:6d}: PC={pc_s} write $2006 {typ} data=0x{data:02X} -> addr=0x{vram_addr:04X} (latch={addr_latch})")
        elif reg == 7:
            out_lines.append(f"{lineno:6d}: PC={pc_s} write $2007 data=0x{data:02X} -> VRAM[0x{vram_addr:04X}]")
            vram_addr = (vram_addr + curr_inc) & 0x3FFF
        else:
            out_lines.append(f"{lineno:6d}: PC={pc_s} write reg={reg} data=0x{data:02X}")

with open(args.out, 'w', encoding='utf-8') as out:
    out.write('\n'.join(out_lines))

print(f'Parsed {len(out_lines)} PPU register writes. Output -> {args.out}')
