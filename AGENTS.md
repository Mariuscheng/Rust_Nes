# AGENTS.md — Rust_Nes 開發契約

這份檔案是 agent 與人類共用的契約，不是風格散文。
可機械檢查的規則只寫在三個地方：`Cargo.toml` 的 `[lints]`、`clippy.toml`、`scripts/verify.ps1`。
本檔只解釋「為什麼」與「哪些判斷不能交給工具」。

## 0. 優先次序

1. 本檔 > `.github/skills/*` > 任何模型自帶的「最佳實踐」。
2. 本檔與 lint 設定衝突 → 以 lint 為準，並回報人類；不要自行修改設定。
3. 本檔沒說可以，不等於允許。§6 的紅線優先於「把任務做完」。

## 1. 完成的定義

- 改動完成 = `pwsh -File scripts/verify.ps1` 回傳 0（零格式差異、零警告、零政策違規、測試通過）。
- CI（GitHub Actions）只驗格式、lint、政策與編譯：`testing/`、`scripts/`、`tools/` 都不上傳，workflow 內嵌同樣的指令。**通過 CI 不等於完成，本機跑過才算。**
- 動到 `cpu/`、`ppu/`、`apu/`、`bus.rs`、`rom/mappers/` 的可觀察行為 → 再加 `-Full`（跑 blargg 套件）。
- 沒跑過不能說「完成」；跑不過不能以「與本次無關」結案，要回報。

## 2. 硬閘門：用檢查代替註解

| 檢查 | 由誰保證 | 它取代的註解 |
| --- | --- | --- |
| 格式一致 | `cargo fmt --all -- --check` | 「請保持排版整齊」 |
| 靜態分析 | `cargo clippy --all-targets --all-features -- -D warnings` | 「注意這裡」「這樣寫應該可以」 |
| 不得失敗即崩 | `clippy::unwrap_used` / `expect_used` = deny | 「這裡不會失敗，放心」 |
| 例外必須說明 | `clippy::allow_attributes_without_reason` = deny | 「為什麼這裡破例」 |
| unsafe 必須說明 | `clippy::undocumented_unsafe_blocks` = deny | 唯一允許的 `// SAFETY:` |
| 不得留下未完成 | `clippy::todo` / `unimplemented` / `dbg_macro` = deny | 「之後再補」「先印出來看」 |
| 跳過的測試要說原因 | `clippy::ignore_without_reason` = deny | 「這個測試先關掉」 |
| 不得在堆疊配大緩衝 | `clippy::large_stack_arrays` = deny | 「這個陣列放堆疊就好」 |
| 時間運算不得裸減 | `clippy::unchecked_time_subtraction` = deny | 「這裡不會小於零」 |
| 不得 inline 抑制 | `scripts/verify.ps1` 掃 `#[allow(` | 「先關掉這個 lint」 |
| 不得留 marker | `scripts/verify.ps1` 掃 `TODO/FIXME/HACK/XXX` | 「有空再處理」 |

規則只改設定檔，不靠在文件裡提醒。加規則或放寬規則都是契約變更（§4）。

## 3. 註解政策

只允許三種註解：

1. `///`、`//!` 契約註解：不變量、單位、panic 條件、可觀察行為、外部事實出處。
2. `// SAFETY:`：`undocumented_unsafe_blocks` 要求，寫「呼叫的前置條件為何成立」。
3. 引用外部事實的 doc comment，且必須可查（NESdev wiki 條目、blargg 測試名、ROM 名）。

禁止：描述程式碼正在做什麼、防呆提醒、橫幅分隔線、被註解掉的舊程式碼、`TODO`/`FIXME`。

想寫註解時，依序問：

- a. 能用型別表達？→ newtype、`const` 泛型、`enum`，讓錯誤狀態無法表達。
- b. 能用 lint 表達？→ 提出規則變更（§4），而不是寫一句「請勿…」。
- c. 能用測試表達？→ 寫進 `testing/`，測試名就是文件。
- d. 是外部事實？→ doc comment，附出處。
- e. 都不是？→ 刪掉那行，或改寫到意圖自明。

既有註解不追溯。但只要你改了某個函式，該函式內的註解就要依本節處理（升級成 doc comment 或刪除）；不得為了「清理註解」而單獨大改未涉入的程式碼。

## 4. 例外程序

- 唯一形式：`#[expect(lint, reason = "…")]`。禁止 `#[allow]`、禁止 `#![allow]`。
- `reason` 寫「為什麼無法重構」，不是「為什麼現在不改」。
- 一次例外涵蓋一個 lint、一個位置。crate 級或 module 級例外需人類同意（現有：`src/main.rs` 的縮寫詞彙、`testing/rom_tests.rs` 的 harness helper）。
- 選 `expect` 而非 `allow` 的理由：lint 不再觸發時 `unfulfilled_lint_expectations` 會報錯，例外自己會過期，不會腐爛。
- 修改 `Cargo.toml` / `clippy.toml` 的 lint 設定 = 契約變更：需人類明確同意，且與程式改動分開 commit。

## 5. 反 slop 的結構規則（無法 lint 化，靠判斷）

- 抽象預算：新 trait、泛型、`Box<dyn>` 需要至少兩個實際呼叫點或一個 crate 邊界；不得為「未來可能」而加。
- 熱路徑：`step_frame`、bus read/write、PPU dot、APU tick 不得引入動態分派、`Rc<RefCell<_>>`、每 cycle 的配置或 clone。
- 依賴：不新增 crate（`minifb`、`rodio` 已定案）。需要時先問。
- diff 紀律：一次一個關注點；不重排沒被要求改的檔案；不「順手」改風格或改名。
- 不刪除描述外部事實的註解，除非能提出事實已失效的證據。
- 新行為要有測試；測試放 `testing/`（feature `local_harness`），不得為了變綠而改既有期望值（尤其 golden frames）。

## 6. 紅線：停下來問，不要自行決定

- 要改 lint 設定、要加 `#[expect]`、要加依賴、要改公開行為或時序。
- lint 與硬體行為衝突（例：`arithmetic_side_effects` 對 wrapping 運算是選錯工具）。
- 測試失敗但看起來「與本次無關」，或要改 `testing/` 期望值才會過。
- 需要刪除既有註解，或改動 `docs/architecture.md` 的架構描述。

## 7. 專案事實（不要重新推導）

- 單一 binary crate，沒有 `lib.rs`；平台層只有 `src/main.rs`，`nes.rs` 以下與平台無關。
- 測試在 `testing/`，需 `--features local_harness`；ROM 套件標記 `--ignored`，要 `--release`。
- `testing/rom_tests.rs` 由 `src/nes.rs` 掛載（為了讀 private bus state），不是 `testing/mod.rs` 的子模組，因此 clippy 的測試豁免蓋不到它的 helper。
- 領域詞彙使用大寫縮寫（CPU/PPU/APU/OAM/CHR/PRG），已在 crate 級 `#[expect]` 說明理由。
- 存檔格式由 `src/state.rs` 的 `Writer`/`Reader` 定義，第 1 byte 是版本（現為 `0x04`）。改欄位順序或長度必須同時 bump 版本、更新 `testing/state_tests.rs::state_format_is_pinned` 的 (長度, 雜湊)，並保留舊版載入分支。
- 存檔路徑（`nes.rs` / `apu.rs` / 9 個 mapper）全部經過 `src/state.rs` 的 `Writer`/`Reader`；mapper 的 payload 沒有獨立版本號，改它的欄位順序等於讓舊存檔失效，必須維持順序或 bump 外層版本。
- `testing/state_tests.rs::every_mapper_round_trips_its_state` 覆蓋每個 mapper 的 save/load 對稱性；它會先斷言「poke 真的改了某個被序列化的欄位」，否則等於什麼都沒驗。
- 知識來源：NESdev wiki、blargg 測試 ROM（`nes-test-roms-master/`）、`docs/architecture.md`。

## 8. 維護這份文件

- 新規則必須在加入當天就是綠的。先清基線再加閘門：一次套用 `deny(unwrap, expect)` 曾產生 123 個警告（119 個在測試碼），這種規則最後只會被 `#[expect]` 淹沒。
- 每條新規則要能回答：「它取代哪一種註解、哪一種 slop？」答不出來就不要加。
- 本專案不建議開啟的 lint（語義會全面衝突，屬選錯工具）：`arithmetic_side_effects`、`indexing_slicing`、`pedantic` 的 cast 系列。要用就先用型別（`Wrapping<u8>`、newtype）表達意圖，而不是放寬 lint 或加例外。
- 2026-11 實測後明確拒絕（不要再提議）：`clippy::pedantic` 全組 852 條（`cast_lossless` 264、`unreadable_literal` 128——NES 位址加分隔底線才難讀、`semicolon_if_nothing_returned` 84、`bool_to_int_with_if` 68）；`clippy::restriction` 全組 6364 條（`implicit_return` 792、`arbitrary_source_item_ordering` 629、`missing_docs_in_private_items` 508 與 §3 衝突、`single_call_fn` 223）；個別拒絕 `cast_possible_truncation` 75（`u16 → u8` 取低位是位址語義）、`struct_excessive_bools` 16（硬體暫存器就是 bit 集合）。
