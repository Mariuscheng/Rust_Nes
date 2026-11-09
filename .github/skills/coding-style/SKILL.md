---
id: rust-style
displayName: Rust 編程風格與系統設計
description: |
  在專案中撰寫 Rust 程式時，應依本技能進行設計與實作。
  當使用者詢問「如何寫 Rust」、「如何重構 Rust 模組」、「如何依照 SOLID 原則設計」等問題時，應該啟用本技能。
version: 1.0.0
author: Cheng Marius
user-invocable: true
---

# Rust 編程風格技能

## 1. 套用 SOLID 原則（在 Rust 中）

- **S – 單一職責原則 (SRP)**  
  - 每個模組或服務只負責一個領域功能，例如：`user_auth`、`order_service`、`payment_gateway`。  
  - 一個 struct / trait 不應該同時處理「業務邏輯」與「儲存／序列化」細節，該拆成兩個模組或 trait。

- **O – 開放封閉原則 (OCP)**  
  - 用 `trait` 定義抽象介面，讓實作可以擴充，但不修改現有 API。  
  - 例如：定義 `PaymentProcessor` trait，再實作 `StripeProcessor`、`PayPalProcessor`，新增支付方式時不用改呼叫端。

- **L – 里氏替換原則 (LSP)**  
  - 在 Rust 中，trait 實作應確保「任何使用該 trait 的程式碼換成另一個 impl 都仍正確」。  
  - 避免在實作中偷偷改變行為（例如：某個 `save()` 突然會拋 panic 而其他 impl 不會）。

- **I – 介面隔離原則 (ISP)**  
  - 不要讓單一 trait 有太多方法，你可以拆成多個小 trait：  
    - `ReadUser`、`WriteUser`、`DeleteUser` 而不是一個 `UserOps` 大 trait。  
  - 依實際使用場景分離接口，讓呼叫端只依賴自己需要的部分。

- **D – 依賴反轉原則 (DIP)**  
  - 上層模組依賴「抽象 trait」，而不是具體型別。  
  - 例如：應用層依賴 `Database` trait，由外部注入 `PostgresDb` 或 `MockDb`，方便測試。

## 2. Rust 系統設計慣例

- **模組與 Cargo 結構**  
  - 大型專案使用 `Cargo workspace`，拆成多個 crate：`core`、`api`、`db`、`test-utils` 等。  
  - 善用 `mod` 搭配 `pub use`，在 `lib.rs` 導出清晰的公共 API，把內部模組預設為私有。

- **錯誤處理與 Result**  
  - 大型函數一律使用 `Result<T, E>`，避免 `panic!` 在業務邏輯中出現。  
  - 推薦使用 `thiserror` 建立自訂錯誤型別，並用 `?` 來傳播錯誤，讓錯誤訊息具體且可追溯。

- **併發與非同步**  
  - 優先使用 `tokio` 作為 runtime，避免在 `async` 中呼叫阻塞 API。  
  - 對共享狀態使用 `Arc<Mutex<T>>` 或 `tokio::sync::Mutex`，並儘量減少鎖的範圍。

- **測試與品質**  
  - 每個模組至少提供單元測試，並針對關鍵路徑加上集成測試。  
  - 用 `#[cfg(test)]` 模組加上 `mock` 或 stub 來驗證抽象层级的行為。

## 3. 高級封裝與 API 設計

- **資料與行為封裝**  
  - 使用 `struct` / `enum` 封裝資料，並用 `impl` 方法提供行為，不讓呼叫端直接存取內部欄位。  
  - 內部欄位預設為 `pub(crate)` 或私有，只在必要時才公開。

- **Trait 抽象與多型**  
  - 用 trait 來定義「能力」，例如：`Logger`、`Notifier`、`Repository`，讓不同實作可以互換。  
  - 一個型別可以實作多個 trait，避免在 struct 中堆太多功能。

- **模組層級與目錄結構**  
  - 按領域（domain）拆分目錄，例如：`src/user/`、`src/order/`、`src/payment/`。  
  - 每個子目錄有自己的 `mod.rs` 或子模組，外部一律透過 `pub use` 暴露簡潔 API。

- **簡潔的公眾 API**  
  - 外部 crate 只暴露少量、高抽象的型別與函數，例如：`Client`、`Service`、`Config`。  
  - 把「細節」與「實作」藏在 `impl` 或私有模組內，避免呼叫端過度依賴實作細節。

- **可測試與可組合的設計**  
  - 依賴以 trait 注入，讓你可以用 `Mock` 或 `Stub` 替代真實依賴。  
  - 把高階流程拆成小函數組合，讓每個單元測試只驗證一塊邏輯。

## 4. 要與不要（Do's and Don'ts）

- **Do's（要）**  
  - 用 `trait` 定義抽象接口，再用具體型別實作。  
  - 每個模組只負責一個領域，如果太長就拆成子模組。  
  - 用 `Result` 包錯誤，並在關鍵步驟加 log。  
  - 在關鍵函式與模組加上 `///` 註解，說明其用途與限制。  
  - 保持函數短小，單一職責，避免超過 20–30 行。

- **Don'ts（不要）**  
  - 不要隨意用 `panic!` 在業務邏輯中。  
  - 不要直接暴露 struct 內部欄位，除非真的必要。  
  - 不要寫一個 struct 有超過 5 個以上完全不同職責的方法。  
  - 不要在錯誤處理中靜默吞掉錯誤而不記錄或處理。  

## 5. 範例（Examples）

- **當你問：**  
  - 「請幫我設計一個 GBA 模擬器的記憶體存取模組時」  
    - 你應該：  
      - 先用 trait 定義 `MemoryAccess`，再實作 `GbaMemory` 或 `SafeMemory`。  
      - 函式一律用 `Result<T, Error>`，並在關鍵點加上 log。  
      - 用 `impl` 包裝所有實作，不讓呼叫端直接存取內部欄位。

- **當你說：**  
  - 「請用 SOLID 重構這個模組」  
    - 你應該：  
      - 把一個大 struct 拆成多個小 struct，並用 trait 來隔開接口。  
      - 每個模組只負責一個領域，例如 `Memory`、`Cpu`、`Ppu`。

- **當你說：**  
  - 「請依照我的風格重寫這個測試模組」  
    - 你應該：  
      - 用 `test` 模組加上 `#[cfg(test)]`。  
      - 每個測試只驗證一件事，測試名稱清楚，例如 `test_user_login_with_invalid_password_should_return_401`。