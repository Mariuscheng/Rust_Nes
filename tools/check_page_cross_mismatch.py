#!/usr/bin/env python3
import re
import sys

logfile = 'run_output.txt'
if len(sys.argv) > 1:
    logfile = sys.argv[1]

addr_re = re.compile(r'ADDR PAGE CROSS @ PC=0x([0-9A-Fa-f]{4}) mode=([^ ]+) param_ptr=0x([0-9A-Fa-f]{4}) addr=0x([0-9A-Fa-f]{4})')
cpu_re = re.compile(r'CPU: STEP PC=0x([0-9A-Fa-f]{4}) opcode=0x([0-9A-Fa-f]{2}) op=([^ ]+) mode=([^ ]+) base=(\d+) extra=(\d+) total=(\d+)')

allowed_no_extra = set(['STA','STX','STY','JSR','JMP','RTS','PHA','PLA','PHP','PLP','BRK','RTI'])

with open(logfile, 'r', encoding='utf-8', errors='ignore') as f:
    lines = f.readlines()

mismatches = []
for i, line in enumerate(lines):
    m = addr_re.search(line)
    if not m:
        continue
    pc = m.group(1).upper()
    mode = m.group(2)
    param_ptr = m.group(3).upper()
    addr = m.group(4).upper()

    # find next CPU: STEP with same PC
    found = False
    for j in range(i+1, min(i+40, len(lines))):
        cm = cpu_re.search(lines[j])
        if cm:
            cpu_pc = cm.group(1).upper()
            if cpu_pc == pc:
                op = cm.group(3)
                base = int(cm.group(5))
                extra = int(cm.group(6))
                total = int(cm.group(7))
                found = True
                if extra == 0 and op not in allowed_no_extra:
                    mismatches.append((i+1, j+1, pc, mode, param_ptr, addr, op, base, extra, total))
                break
    if not found:
        mismatches.append((i+1, None, pc, mode, param_ptr, addr, None, None, None, None))

if not mismatches:
    print('No mismatches found (all page-crosses matched an extra cycle or were for allowed ops).')
else:
    print('Found potential mismatches (ADDR PAGE CROSS -> CPU STEP has extra=0 for non-store ops):')
    for m in mismatches:
        addr_line, cpu_line, pc, mode, param_ptr, addr, op, base, extra, total = m
        print(f'ADDR LINE {addr_line}: PC=0x{pc} mode={mode} param_ptr=0x{param_ptr} addr=0x{addr}')
        if cpu_line:
            print(f'  CPU LINE {cpu_line}: op={op} base={base} extra={extra} total={total}')
        else:
            print('  CPU STEP not found in next 40 lines')

# Exit code
sys.exit(0 if not mismatches else 2)
