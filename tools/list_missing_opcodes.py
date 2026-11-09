import re
from pathlib import Path

p = Path("c:/Users/mariu/Desktop/Rust_Nes/src/cpu/mos6502.rs")
text = p.read_text(encoding='utf-8')
# 尋找形如 0xFF => 的行
matches = re.findall(r'(?m)^\s*0x([0-9A-Fa-f]{2})\s*=>', text)
present = sorted({int(h,16) for h in matches})
allset = list(range(256))
missing = [b for b in allset if b not in present]
print(f"Found {len(present)} explicit opcode entries in decode table.")
print(f"Missing (treated as default NOP): {len(missing)} opcodes")
# 列出逐個值
print(" ".join(f"0x{b:02X}" for b in missing))
# 也輸出壓縮範圍方便閱讀
def compress(lst):
    if not lst:
        return []
    ranges = []
    start = prev = lst[0]
    for x in lst[1:]:
        if x == prev + 1:
            prev = x
        else:
            ranges.append((start, prev))
            start = prev = x
    ranges.append((start, prev))
    return ranges

ranges = compress(missing)
print("Ranges:")
print(", ".join(f"0x{a:02X}" if a==b else f"0x{a:02X}-0x{b:02X}" for a,b in ranges))
