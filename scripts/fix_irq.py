import io

# Fix 1: core.rs - irq() should set last_cycles = 7 (like nmi())
p = r'src/cpu/core.rs'
data = open(p, 'rb').read()
old = b'self.skip_irq = false;\r\n    }\r\n\r\n    fn stack_push_u16'
new = b'self.skip_irq = false;\r\n        self.last_cycles = 7;\r\n    }\r\n\r\n    fn stack_push_u16'
assert old in data, 'core.rs pattern not found'
data = data.replace(old, new, 1)
open(p, 'wb').write(data)
print('core.rs fixed')
