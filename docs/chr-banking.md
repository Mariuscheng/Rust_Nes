# CHR bank 切換：refresh-on-write 與 dirty block 最佳化

> 狀態：**已完成**（`af19152` refresh-on-write、本頁記錄的 dirty block 最佳化）
> 目的：讓 MMC3 / VRC6 / FME-7 的 mid-frame CHR bank 切換在下一次 tile fetch 就生效，
> 同時不讓「每次 mapper 寫入就重抄 8 KiB」的固定成本吃掉 frame budget。

---

## 1. 問題：CHR 有兩份

硬體上 PPU 直接對卡匣的 CHR-ROM（或卡匣上的 CHR-RAM）拉線，mapper 只負責選 bank。
本專案為了讓 PPU 保持自足（不需要在渲染中途 borrow mapper），把 pattern memory 放在
`PPU::chr_ram: [u8; 8192]`，是一份**副本**。

於是每次 mapper 暫存器寫入後，這份副本都可能過期：

- 開發初期只在開機時 `Cartridge::init_chr_ram()` 抄一次 → 遊戲中途換 bank 後
  畫面會拿舊圖形（Kirby 的標題／狀態列會缺圖、錯圖）。
- N64 之前的 mapper（NROM/CNROM）CHR 是靜態的，所以這個缺陷只在 MMC3 之後的卡匣
  才看得到。

## 2. 曾評估：fetch-through-mapper（否決）

讓 PPU 渲染時直接向 mapper 要 byte（`ChrSource`），理論上最省——沒有副本、沒有同步問題。
實作後 **Mario 標題畫面的 logo 消失**，而 `chr_ram` 內容在兩條路徑下逐 byte 相同，
原因沒找到（懷疑是 borrow 期間的時序或 `has_chr_ram` 分支差異）。

在「原因不明」的情況下不採用：一個會讓既有 golden 退化的架構，比多一次 memcpy 更貴。
（記錄在 `docs/timing-refactor-step1.md` 的實驗段落。）

## 3. 採用：refresh-on-write

`MainBus::write()` 在 `$4020..=$FFFF`（mapper 暫存器區）寫入後呼叫 `refresh_chr()`，
把 mapper 目前的 CHR 映射抄回 `ppu.chr_ram`。因為抄寫發生在寫入的當下，
下一個 dot 的 tile fetch 就會看到新 bank——含 mid-frame 切換、含 IRQ handler 內的切換。

- CHR-RAM 卡匣（`mapper.has_chr_ram()`）直接跳過：那是 PPU 自己的 RAM，遊戲透過
  `$2007` 寫入，mapper 不持有資料。
- 開機仍走 `Cartridge::init_chr_ram()`；兩條路徑互不依賴。

正確性代價：無（PPU 永遠看到完整的 8 KiB）。效能代價：每次 mapper 寫入 8 KiB 拷貝
＝ 8192 次 `read_chr()` 虛擬呼叫＋除法取模。

## 4. 最佳化：dirty block mask（1 KiB 粒度）

mapper 是唯一知道「這次寫入到底動了哪些 bank」的人，所以把這個知識交給它：

```rust
// src/rom/mapper.rs
/// 自上次呼叫以來，哪些 1 KiB CHR block（bit i = $0000 + i*$400）可能改變。
/// 0xFF = 不確定，全部重抄（預設值）。
fn take_chr_dirty_blocks(&mut self) -> u8 { 0xFF }
```

`MainBus::refresh_chr()` 只抄被標記的 block。三個約定：

1. **預設 0xFF**：不實作的 mapper 行為完全不變，不會因為忘記覆寫而壞掉。
2. **初始值 0xFF**：確保開機後第一次 refresh 是完整拷貝。
3. **`load_state()` 重置為 0xFF**：讀檔後的 bank 可能和 PPU 手上的不同。

目前的覆寫：

| Mapper | 標記策略 | 說明 |
|---|---|---|
| MMC3 | `$8000` 偶：僅在 bit7（CHR mode）翻轉時 0xFF；`$8001`：依 R0/R1 vs R2–R5 標半張表；IRQ/mirroring：0 | Kirby 的 IRQ handler 每條 scanline 只寫 `$C000/$E000`，現在**零拷貝** |
| FME-7 | command 0–7 只標 `1 << command`；command 8–15（mirroring/IRQ）：0 | 最精確的一個：一次寫入只抄 1 KiB |
| VRC6 | `$D000-$EFFF` → 0xFF（保守），其餘 0 | VRC6 的 CHR 暫存器解碼本身還有疑問，先保守處理 |
| MMC1 / UxROM / CNROM / AxROM / MMC5 | 未覆寫 → 0xFF | 這些卡匣的 CHR 幾乎是靜態的，維持原行為 |

MMC3 的 `$8001` 之所以能只標半張表：CHR mode 0 時 R0/R1 餵 `$0000-$0FFF`、
R2–R5 餵 `$1000-$1FFF`；mode 1 時互換。

## 5. 量測（release, headless, 1200 frames）

同一顆 binary、只切換 `take_chr_dirty_blocks()` 回傳 0xFF 與真實 mask：

| ROM | 全量 refresh | dirty mask | 差異 |
|---|---|---|---|
| Kirby's Adventure (MMC3) | 3.85 / 3.91 / 3.87 ms | 2.71 / 2.72 / 2.82 ms | **−1.13 ms（−29%）** |
| Mario (NROM) | 2.26 ms | 2.26 ms | 0（未覆寫，行為不變） |
| 1942 (mapper 0) | 2.28 ms | 2.22 ms | 雜訊 |
| 1943 (mapper 0) | 2.07 ms | 1.99 ms | 雜訊 |

Kirby 的 2.75 ms 已經是「不含 CHR 同步」的水準（純 CPU/PPU/APU 工作）。

## 6. 驗證

- 單元測試 `chr_refresh_only_copies_banks_a_write_can_touch`：poke `ppu.chr_ram` 當標記，
  證明 IRQ latch／enable 寫入不會觸發拷貝、選 register 不會、`$8001` 只重抄對應半張表。
- 單元測試 `fme7::tests::chr_dirty_marks_only_the_bank_a_write_touches`：command 3 只標
  block 3，IRQ command 不標。
- golden：`Kirby's_Adventure.nes` frames 300 / 900 在最佳化前後**完全相同**；
  其餘 10 個既有檢查點也沒有位移。
- 目視：標題畫面、開場文字、遊戲中捲動與狀態列皆正確。

## 7. 已知缺口 / 未來工作

1. **MMC5 沒有實作 CHR bank**：`read_chr()` 忽略 `chr_banks`，`$5120-$512B` 完全沒接，
   ExRAM 與 scanline IRQ 也沒有。目前 MMC5 卡匣跑得起來但畫面不可信（README 已改註記）。
   另外 `$5000-$5FFF` 的 mapper 寫入不會觸發 `refresh_chr()`，補 MMC5 時要一起處理。
2. **拷貝仍是逐 byte 的 `read_chr()`**：若之後要再快，可讓 mapper 回報
   `chr_data() -> &[u8]` 與每個 1 KiB block 的 offset，讓 refresh 退化成 8 次
   `copy_from_slice`（MMC3 每次只抄 4 KiB → 約 4 次 memcpy）。
3. **VRC6 可以更精確**：先確認 `$D000-$D003` / `$E000-$E003` 的暫存器解碼
   （目前 `(addr - 0xD000) >> 2` 疑似有誤），再改成 per-block mask。

## 8. 相關文件

- `docs/timing-refactor-step1.md` — per-access ticking 的整體重構進度
- `docs/nmi-timing-design.md` — NMI 取樣與延遲的設計與實驗紀錄
