# 台灣暗棋 AI

以 Rust 撰寫的台灣暗棋（4×8、32 子蓋牌）引擎，編譯成 WebAssembly 後在瀏覽器裡執行，附一個可以直接跟它對弈的網頁。

- **搜尋**：Expectiminimax + alpha-beta，翻子視為機會節點（Star1 剪枝）
- **評估**：依「天敵數」變動的動態子力，參數全部由自我對弈（SPSA）學出
- **殘局**：≤ 4 子時即時逆推出完美解，並考慮 60 步無進展判和
- **Mixture of Experts**：多位專家依局面加權決策，每局走法都不一樣

## 快速開始

需要 Rust（含 `wasm32-unknown-unknown` target）、[wasm-pack](https://rustwasm.github.io/wasm-pack/) 與 Node.js。

```sh
./build.sh        # 建置 WASM 並打包網頁
```

輸出 `web/dist/index.html` 是單一檔案（JS、CSS、Worker、WASM 全部內嵌），直接用瀏覽器開啟即可對弈。網頁支援「人機對弈」與「電腦對戰」兩種模式；電腦對戰時雙方各用獨立的大腦（置換表與亂數），可分別設定思考時間、暫停、連續對局並累計戰績。開發時先跑一次 `./build.sh` 產生 WASM，之後 `cd web && npm run dev`。

## 規則

採用台灣 / TCGA 電腦暗棋規則：

- 所有棋子（含俥、傌）每步只能上下左右走一格。
- 帥 > 仕 > 相 > 俥 > 傌 > 炮 > 兵，大吃小、同級互吃；帥不能吃兵，兵可以吃帥；兵不能吃炮。
- 炮必須隔一個棋子（炮架，可為蓋牌）跳吃，距離不限、可吃任何兵種，不能相鄰吃子。
- 蓋著的棋子不能被吃。第一個翻子的人取得翻出棋子的顏色。
- 被吃光或無子可動判負；雙方合計連續 60 步沒有翻子或吃子判和；同一局面出現三次判和。

## 引擎

### 搜尋

- 翻子是機會節點：依剩餘蓋牌中各兵種的數量取期望值，以 Star1 剪枝縮小每個結果的搜尋視窗。
- 走子節點：PVS、Zobrist 置換表（雜湊包含蓋牌池）、killer / history 排序、LMR、空著剪枝（僅在仍有蓋牌時使用，因為翻子幾乎總能當等著）、只搜吃子的靜態搜尋。
- 翻子只在剩餘深度夠深的節點展開（門檻隨思考時間 5–7），非根節點只搜啟發分數最高的幾個翻子點。這是讓中局深度從 5 層提升到 8–9 層的關鍵。
- 重複局面與無進展規則在搜尋中直接判和。

### 評估

- 子力參考 Observer（NTNU）的天敵數模型：每顆子的價值為 `top × ratio^n`，`n` 是還活著（含未翻開）且能吃掉它的敵方非炮棋子數。敵方天敵被吃掉時，己方棋子自動升值，所以引擎懂得用帥換仕這類暗棋特有的交換。帥的價值依敵兵數量遞減，炮獨立計價。
- 位置項：機動性、保護、被困（無安全逃生格）、追殺距離、殘局一對一追殺的奇偶性、炮線、翻子位置風險，以及接近無進展上限時把評估值縮向 0，逼優勢方進取。

### 殘局資料庫

全翻開且總子數 ≤ 4 時，以逆推分析即時算出該子力組合的完美解（原生約 0.2 秒）；剩 5 子且思考時間足夠時，預先建好吃掉任一子後的 4 子表。表中記錄的是距離下一次吃子的步數，因此能正確判斷「贏得了但來不及在 60 步內吃到子」的局面。

### Mixture of Experts

gating 函數依未翻子數、子力差與剩餘棋子數決定各專家的權重：

| 專家 | 作用 |
| --- | --- |
| 一般 / 殘局評估專家 | 兩組各自調參的評估參數，隨未翻子數從 8 到 0 平滑交接 |
| 開局翻子 | 翻子位置風險：翻己方仕、相旁較安全，敵兵未翻完時避開己方帥旁 |
| 戰術搜尋 | Expectiminimax 主搜尋，所有決策的骨幹 |
| 殘局獵手 | 全翻開後偏好縮短與獵物的距離、避免重複 |
| 守勢 / 攻勢 | 落後時製造變數、保住和棋；領先時吃子施壓、避免重複 |
| 殘局資料庫 | ≤ 4 子時接手，給出完美解 |

最後在搜尋分數接近最佳的候選著法中，依專家偏好加權後抽樣。抽樣溫度同樣由 gating 決定：開局隨機性高，戰術與殘局的關鍵局面收斂到最佳著。網頁上會顯示每一步各專家的權重。

## 調參與實驗結果

網路上找不到公開的暗棋棋譜，所以參數全部來自自我對弈：非同步 SPSA，每回合以 θ+cΔ 與 θ−cΔ 下一對（同一牌局、交換先手）的棋局，依勝負更新參數。每項改動都以獨立的擂台對局驗證：

| 改動 | 對上一版 |
| --- | --- |
| 線性子力 → 天敵數子力 | +237 Elo |
| SPSA 第 1 輪 | +148 Elo |
| SPSA 第 2 輪（加入殘局專家參數） | +119 Elo |
| 翻子深度門檻 | +53 Elo |
| SPSA 第 3 輪 | +52 Elo（100ms/步複驗 +58） |
| 迭代加深用滿 65% 時間預算 | +32 Elo |
| SPSA 第 4 輪（兵種加成、蓋牌折價） | +33 Elo |
| SPSA 第 5 輪（60ms/步） | −5 ± 22 Elo，已收斂，採用第 4 輪參數 |

除特別註明外，每項 400–800 盤、每步 25ms。這些是自我對弈的相對 Elo，數字會偏高。MoE 的多變模式對確定性版本為 −1 Elo（誤差內）。對上同一評估但只搜 2 層的版本勝率約 82%，其餘輸局多半來自翻子運氣。

各輪參數在 `tuning/round*.txt`，24 盤自我對弈棋譜（每步 1 秒，含評分與搜尋深度）在 `games/`。

## 開發工具

在 `engine/` 下：

```sh
cargo test --release                                               # 規則與殘局庫測試
cargo run --release --bin arena   -- 400 25 "A 設定" "B 設定" 16    # 自我對弈擂台
cargo run --release --bin spsa    -- 10000 20 14                   # SPSA 調參
cargo run --release --bin bench   -- 2000                          # 各階段搜尋深度
MISTY_BIN=… cargo run --release --bin match -- misty 400 300 6     # 與外部引擎對弈（見下）
cargo run --release --bin records -- 24 1000 ../games 4            # 產生棋譜
python3 apply_params.py "$(cat ../tuning/round4.txt)"              # 把調參結果寫回預設值
```

`match` 與外部引擎對弈，同一副牌交換先後手，終局一律依本專案規則判定：`misty`（[MistyBanqi](https://github.com/brianhliou/misty-banqi)，UCI，需把 `engine.rs` 的 40 步和棋改為 60；`misty:nodes=N` 改用固定節點數）或 `george`（[George0828Zhang hw3](https://github.com/George0828Zhang/chinese-dark-chess-hw)，TCG CDC 協定，需在 `estimatePlyTime` 讀取 `PLY_MS` 環境變數以固定每步時間）。執行檔路徑由 `MISTY_BIN`、`GEORGE_BIN` 指定，`REC=目錄` 可存下每盤棋譜。

`arena` 的設定字串格式為 `key=value,...`，可覆寫任何評估參數（見 `src/eval.rs` 的 `PARAM_NAMES`，殘局專家參數加 `e.` 前綴）或搜尋設定，例如 `fmd`、`iflips`、`nmp`、`contempt`、`variety`、`tmul`。

## 目錄

```
engine/src/
  board.rs    規則、著法產生、Zobrist
  eval.rs     評估函數與參數
  search.rs   Expectiminimax 搜尋
  tb.rs       殘局資料庫
  moe.rs      Mixture of Experts 決策層
  game.rs     對局狀態（持有真實蓋牌配置）
  wasm.rs     WebAssembly 介面
  bin/        arena、spsa、bench、records
web/          Vite + TypeScript 網頁，引擎在 Web Worker 中執行
tuning/       各輪 SPSA 參數
games/        自我對弈棋譜
```

## 參考資料

- Tsan-sheng Hsu，TCGA / NTU 暗棋比賽規則：<https://homepage.iis.sinica.edu.tw/~tshsu/tcg/2019/hwks/rules.pdf>
- Chen, Shen, Hsu, “Chinese Dark Chess”, *ICGA Journal* 33(2), 2010
- 《電腦暗棋程式 Observer 的設計與實作》，國立臺灣師範大學碩士論文
- Ballard, “The *-Minimax Search Procedure for Trees Containing Chance Nodes”, *Artificial Intelligence*, 1983
- PTT DarkChess 版、巴哈姆特暗棋技巧文章
