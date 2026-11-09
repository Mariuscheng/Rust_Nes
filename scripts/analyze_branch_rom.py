"""Analyze 1.Branch_Basics.nes ROM for debug output protocol."""
import sys

data = open(sys.argv[1] if len(sys.argv) > 1 else "../1.Branch_Basics.nes", "rb").read()
prg = data[16:16+16384]

# Check for debug magic
magic = bytes([0xDE, 0xB0, 0x61])
has_magic = magic in prg
print("Has $DE $B0 $61 magic in PRG:", has_magic)

# Look for STA $6000-$7FFF
print("\nSTA $6000-$7FFF occurrences:")
for i in range(len(prg)-3):
    if prg[i] == 0x8D:  # STA Absolute
        addr = prg[i+1] | (prg[i+2] << 8)
        if 0x6000 <= addr <= 0x7FFF:
            print(f"  ${i+16+0x8000:04X}: STA ${addr:04X}")

# Look for writes to PPU registers
print("\nPPU register writes:")
for i in range(len(prg)-3):
    if prg[i] in (0x8D, 0x8E, 0x8F):
        addr = prg[i+1] | (prg[i+2] << 8)
        if 0x2000 <= addr <= 0x2007:
            stype = "A" if prg[i] == 0x8D else ("X" if prg[i] == 0x8E else "Y")
            print(f"  ${i+16+0x8000:04X}: ST{stype} ${addr:04X}")

# Look for NMI writes at $6000
print("\nSTA $6000 at reset init:")
for i in range(0, min(500, len(prg)-3)):
    if prg[i] == 0x8D:
        addr = prg[i+1] | (prg[i+2] << 8)
        if 0x2000 <= addr <= 0x2007 or 0x6000 <= addr <= 0x6003:
            print(f"  ${i+16+0x8000:04X}: STA ${addr:04X}")

# Check if there's a LDA #$80 followed by STA $2000 (enable NMI)
print("\nPattern search - LDA #$xx then STA $2000:")
for i in range(len(prg)-4):
    if prg[i] == 0xA9 and prg[i+2] == 0x8D:
        addr = prg[i+3] | (prg[i+4] << 8)
        if addr == 0x2000:
            print(f"  ${i+16+0x8000:04X}: LDA #${prg[i+1]:02X} ; STA $2000")