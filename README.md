# 🎮 Rust_Nes — NES 模擬器

以 Rust 實作的 Nintendo Entertainment System (NES / FC) 模擬器，具備完整的 CPU、PPU、APU 模擬與即時音訊輸出。

[![Rust](https://img.shields.io/badge/rust-edition%202024-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

## ✨ 功能特色

- **6502 CPU 核心** — 支援全部 56 個官方指令與非官方指令（含週期精確 page-cross 懲罰）
- **PPU 圖形處理** — Nametable 渲染、Attribute Table、CHR-ROM/RAM、硬體精靈（8×8 / 8×16）
- **APU 音頻模擬** — 5 聲道：Pulse 1/2、Triangle、Noise、DMC，含 Frame Counter、Envelope、Sweep
- **即時音訊輸出** — 透過 rodio 以 44100Hz 即時播放，含高通濾波器與低延遲佇列控制
- **輸入支援** — 完整 NES 手柄模擬（鍵盤映射）
- **Save / Load State** — 即時存檔與讀檔（F5 存檔 / F7 讀檔）
- **Headless 模式** — 支援無視窗執行以進行自動化測試
- **Mapper 支援** — NROM(0)、MMC1(1)、UxROM(2)、CNROM(3)、MMC3(4)、MMC5(5)、AxROM(7)、VRC6(24/26)、FME-7(69)

## 🎮 按鍵映射

| NES 按鍵 | 鍵盤按鍵 |
|----------|----------|
| A        | Z        |
| B        | X        |
| Start    | Enter    |
| Select   | Right Shift |
| ↑ ↓ ← →  | 方向鍵    |

## 🚀 快速開始

### 環境需求

- **Rust 1.88+**（stable，`edition = "2024"`）
- Windows / macOS / Linux

### 安裝與執行

```bash
# 複製專案
git clone https://github.com/Mariuscheng/Rust_Nes.git
cd Rust_Nes

# 編譯（Release 模式以獲得最佳效能）
cargo build --release

# 執行（預設載入 rom.nes）
cargo run --release

# 指定 ROM 檔案
cargo run --release -- "path/to/rom.nes"

# Headless 模式（無視窗，執行 N 幀後退出）
HEADLESS_FRAMES=180 cargo run --release -- "path/to/rom.nes"
```

## 📁 專案結構

```
src/
├── main.rs           # 進入點（視窗 / headless）
├── nes.rs            # 模擬器主控（整合 CPU / PPU / APU）
├── bus/bus.rs        # 主匯流排與 BusInterface
├── cpu/
│   ├── core.rs       # 6502 CPU 核心（暫存器、指令分派）
│   ├── decoder.rs    # 指令解碼表（opcode → instruction + addressing mode）
│   ├── addressing.rs # 定址模式（含 page-cross 偵測）
│   └── ops/          # 指令實作（ALU、分支、堆疊、非官方指令…）
├── ppu/
│   ├── ppu.rs        # PPU 核心（暫存器、VRAM、調色盤）
│   ├── frame.rs      # 逐 dot 渲染管線
│   └── sprite.rs     # OAM 評估與精靈初始化
├── apu/              # APU（pulse / triangle / noise / dmc / frame counter）
├── audio.rs          # 即時音訊輸出（rodio）
├── rom/              # iNES 解析 + mappers/（9 種 mapper）
├── cartridge.rs      # Cartridge 黑盒封裝
├── ram/ram.rs        # 內部工作 RAM（2KB，含鏡像）
├── joypad.rs         # NES 手柄（strobe latch + 序列讀出）
├── debug.rs          # $6000 測試 ROM 協議觀察者
└── rom_tests.rs      # blargg 測試 ROM 回歸套件（#[ignore]）
```

## 🧪 執行測試

```bash
# 單元測試（快，23 個）
cargo test

# blargg 測試 ROM 回歸套件（慢，需把 .nes 放在專案根目錄）
cargo test --release -- --ignored
```

`all_instrs`、`official_only`、`instr_timing` 應通過；`cpu_interrupts`、`ppu_vbl_nmi`、
`apu_test` 目前是**已知時序問題**，測試會驗證失敗內容，等 cycle-accurate 核心完成後
改成 `assert_pass` 即可。

`golden_frames.txt` 是畫面 hash 回歸（Mario / 1942 / 1943 / Kirby's Adventure /
MMC3 測試 ROM）。若變更是
有意的，重新生成：

```bash
# PowerShell
$env:REGEN_GOLDEN=1; cargo test --release -- --ignored golden_frames
# bash
REGEN_GOLDEN=1 cargo test --release -- --ignored golden_frames
```

## 🏗️ 架構說明

### CPU (6502)
- 暫存器：A, X, Y, PC, SP, P (status)
- 支援所有官方 opcode 與非官方 opcode（SLO, RLA, SRE, RRA, SAX, LAX, DCP, ISC, ANC, ALR, ARR, AXS, KIL）
- 週期精確 page-cross 偵測（用於 DMC DMA 同步）

### PPU (Picture Processing Unit)
- 256×240 軟體渲染
- 4 個 Nametable + Attribute Table
- 64 個硬體精靈（透過 OAM）
- 支援垂直/水平鏡像
- 完整 NMI 中斷

### APU (Audio Processing Unit)
- Pulse 1/2：可程式化 duty、sweep、envelope
- Triangle：線性計數器
- Noise：可程式化 LFSR
- DMC：delta modulation 取樣播放
- Frame Counter：4-step / 5-step 模式
- 整數取樣演算法（無浮點誤差）
- 高通濾波器（HPF, 20Hz cutoff）

## 🗺️ 支援的 Mapper

| Mapper | 狀態 |
|--------|------|
| NROM (0) | ✅ |
| MMC1 (1) | ✅ |
| UxROM (2) | ✅ |
| CNROM (3) | ✅ |
| MMC3 (4) | ✅ IRQ 已實作；CHR bank 在 mapper 寫入後立即同步，且只重抄變動的 1 KiB block（支援 mid-frame 切換） |
| MMC5 (5) | ⚠️ 部分：PRG/CHR 是線性映射；`$5120-$512B` CHR bank、ExRAM、scanline IRQ 尚未實作 |
| AxROM (7) | ✅ |
| VRC6 (24/26) | ✅ |
| FME-7 (69) | ✅ |

CHR 同步的設計取捨與量測見 [`docs/chr-banking.md`](docs/chr-banking.md)。

## ⚠️ 已知限制

以下 blargg 測項目前**不會通過**，且不影響目前測試過的所有遊戲（Mario、Kirby's
Adventure、1942、1943 的畫面 golden 全部一致）：

| ROM | 結果 | 差距 |
|-----|------|------|
| `ppu_vbl_nmi` | 6/10（卡測項 7 `nmi_on_timing`） | `$2000` 開啟 NMI 的時機與硬體差 **1 個 PPU dot**：硬體要求 enable 距 VBL set 至少 25 dots，我們允許到 24 |
| `cpu_interrupts` | 1/5（卡測項 2 `nmi_and_brk`） | 最後一列差 1 個 CPU cycle（NMI 早了） |
| `3-nmi_and_irq` | 失敗 | NMI/IRQ 同拍時的優先權差 1–2 cycles |
| `apu_test` | 6/8（卡測項 7，DMC timing） | — |

成因與已排除的變體（六個 PPU 側相位調整全部無效或破壞已通過的測項）整理在
[`docs/c0-phase-notes.md`](docs/c0-phase-notes.md)。要繼續推進需要把 CPU 改成
cycle-stepped（每 cycle 一個 state、存取有自己的 dot 位置），計畫見
[`docs/cycle-stepped-cpu.md`](docs/cycle-stepped-cpu.md)。

## 📚 設計文件

- [`docs/chr-banking.md`](docs/chr-banking.md) — CHR bank 切換：refresh-on-write 與 dirty block 最佳化
- [`docs/timing-refactor-step1.md`](docs/timing-refactor-step1.md) — per-access ticking 重構進度
- [`docs/nmi-timing-design.md`](docs/nmi-timing-design.md) — NMI 取樣／延遲的設計與實驗
- [`docs/cycle-exact-refactor.md`](docs/cycle-exact-refactor.md) — P1–P4 的進度與量測
- [`docs/c0-phase-notes.md`](docs/c0-phase-notes.md) — 剩餘三個測項的相位分析與排除清單
- [`docs/cycle-stepped-cpu.md`](docs/cycle-stepped-cpu.md) — cycle-stepped CPU 重構計畫（C0–C4）
- [`docs/dot-offset-events.md`](docs/dot-offset-events.md) — dot-offset 事件模型（D1–D5）

## 📝 License

MIT License

## 🙏 參考資源

- [NESdev Wiki](https://www.nesdev.org/wiki/Nesdev_Wiki)
- [6502 Instruction Reference](https://www.masswerk.at/6502/6502_instruction_set.html)
- [NES APU Reference](https://www.nesdev.org/wiki/APU)