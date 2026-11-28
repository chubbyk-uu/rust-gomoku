# Mix9svq NNUE 接入设计与可行性审查方案（`M3.2`）

> 历史计划。文中的基线、预期和门槛记录当时的假设及阶段；现行工作见[引擎路线图](../../optimization-roadmap.md)。

> 状态：阶段 1–3a 已在分支 `exp/m32-nnue-mix9svq`（`9f6b899`）实现并通过 300 局验收；**暂缓合入 `master`**，优先修复审计发现的 P0 缺陷并推进 P1 搜索剪枝/TT 优化。
> 基线：`master` `8cc9dc7`（代码等价 `8563005`，已完成 P0、`M1.1`、`M2.3` `df-pn` VCT）。
> 参考实现：[opponent/rapfi](file:///home/jerry/rust_ws/rust-gomoku/opponent/rapfi)（GPL-3.0）与 [opponent/rapfi-networks/mix9svq](file:///home/jerry/rust_ws/rust-gomoku/opponent/rapfi-networks/mix9svq)（权重 **CC0 公有领域**）。

---

## 1. 为什么直接上 `Mix9svq` NNUE（而不是继续微调旧评估/旧搜索）

前序实验（见 `docs/perf-log.md`）已经用实测证明：
1. 在旧 92 维线性评估 + `running_downf` 树上做搜索/TT 局部补丁，极易出现“限时赢、不限时固定 `d8/w40` 掉分或打平”的撕裂；
2. 根部 `df-pn` VCT（`M2.3`）已在限时下带来 `+17.4 Elo`（Renju）/ `+7.0 Elo`（Freestyle）和密盘 `2.6x–3.3x` 提速，但在不限时 `d8/w40` 下仍受限于 95% 中盘走法所用的评估函数（得分率 `50.0%`）；
3. `M3.1` 线性评估调参上限仅在 `+30 ~ +80 Elo`，而 **`Mix9svq` NNUE 是实打实的 `+300 ~ +500 Elo` 质变**，且同时提供 **Value + Policy**，天然在**不限时 `d8/w40` 和限时 `1s/move`**、**Renju 与 Freestyle** 全场景通吃。

---

## 2. 技术摸底与可行性结论

### 2.1 权重与协议核查
- **许可证**：`rapfi-networks` 全部权重以 **CC0** 发布；本仓库 `Cargo.toml` 为 `GPL-3.0-or-later`，完全兼容且未来可无缝替换为本机 RTX 5080 自训权重。
- **现成权重文件**（已拉取至本地 `opponent/rapfi-networks/mix9svq/`，不入 git）：
  - Freestyle：`mix9svqfreestyle_bsmix.bin.lz4`（`9.6 MB`，黑白共用，内存展开 ~`18 MB`）
  - Renju：`mix9svqrenju_bs15_black.bin.lz4`（`9.6 MB`）+ `mix9svqrenju_bs15_white.bin.lz4`（`9.4 MB`）（黑白共 ~`36 MB`）
  - 多线程通过 `Arc<Mix9svqWeight>` 只读零拷贝共享；每搜索线程累加器仅约 `5.2 MB`。
- **压缩与二进制头格式**：
  - 已核验前 32 字节：`04 22 4d 18` 为**标准 LZ4 Frame 格式**；解压后首 4 字节为 `0xacd8cc6a`（`crc32("gomoku network weight version 1")`），随后依次为 `arch_hash`（`Mix9svq` 为 `0x84a071fe ^ ((4<<20)|(8<<14)|(4<<8)|8)`）、`rule_mask`、`boardsize_mask`、`desc_len + desc`、10-bit 打包 `codebook[2][65536][64]`、`mapping_index[2][442503]`、`dwconv`、`HeadBucket`。
  - 纯 Rust 实现标准 LZ4 Frame 解压（~150 行，零 C 依赖），PC（Linux/Windows）与 Android ARM64 全平台零构建障碍。

### 2.2 架构与硬件算力核查
- **本机 CPU ISA**：实测支持 `sse4_1 / sse4_2 / avx / avx2 / fma / avx_vnni`。
- **增量更新机制**（对齐 `mixcommon.h` + `mix9svqnnue.cpp`）：
  - **版本栈（Version Stack）**：`move()` 将受影响的最多 44 个内点（`mapSum`）与 13×13 范围外点（`mapConv`）追加到版本表；**`undo()` 仅为 `current_version -= 1`（O(1)）**。
  - **懒更新 `MoveCache`**：`play / undo` 先推入每边 4 字节的 `MoveCache`，连续相反动作自动抵消；**被 beta 剪枝的分支永远不触发累加器更新**。
- **单叶算量与速度估算（必须前置审核的核心瓶颈）**：
  - `evaluateValue`：9 个 `starBlock<64,64>` + 4 个 `starBlock<64,64>` + 3 层 MLP（`320->64->64->4`）≈ **`291K` 次 `i8×i8→i32` 乘加**。
  - **若只写标量版**：单次评估约 `40–80 µs`，会导致限时 `1s/move` 节点暴跌 30 倍（**绝对不可用**）。
  - **使用 AVX2 SIMD**（`_mm256_maddubs_epi16` / `_mm256_mulhi_epi16` / `_mm256_avg_epu8`）：单次 `evaluateValue` 约 **`1.5–2.5 µs`**，单次增量 `move()` 约 **`0.5–0.8 µs`**。
  - **结论**：**阶段 1 必须同步落地 AVX2 SIMD**（并在编译/运行时自动检测 `is_x86_feature_detected!("avx2")`，保留完全等价的标量 fallback 及后续 Android ARM64 NEON 路径）。

---

## 3. 实施前深度风险审计（防“无用功”清单）

在动任何代码前，已逐一核查以下 6 个最容易返工的隐蔽陷阱并确定规避设计：

| # | 潜在陷阱 | 代码级核查结论与硬规避措施 |
| :-: | :--- | :--- |
| 1 | **SIMD 整数舍入/饱和与 Rapfi 差 1 导致评估失真** | 已逐行核对 `activation.h`、`linear.h`、`vec.h`：<br>- `I8Op::avg` 是无符号向上取整 `_mm256_avg_epu8`：`((a as u16 + b as u16 + 1) >> 1)`（不能写成 `(a+b)/2`）；<br>- `I16Op::mulhi` 是有符号高 16 位 `((w as i32 * x as i32) >> 16)`，且输入先做 `i16` 左移 2 位（`slli<2>`）；<br>- `SignedInput = true` 的 `linear`（`starBlock` 下投影）必须处理有符号 `i8×i8` 累加；<br>- **硬门禁**：阶段 1 必须用本地 `g++` 编出 Rapfi C++ 黄金输出，对空盘 + 20 个 Freestyle/Renju 中盘局面比对 **`layer3i32`（比特级完全一致）与全部 225 个 policy（误差 `< 1e-5`）**，不通过绝不接入搜索！ |
| 2 | **视角/符号与分值域冲突** | `rust-gomoku` 在 `alphabeta.rs:473` 对古典评估取 `-evaluate_board(..., -side)`，而 `Mix9svq` 直接输出**当前行棋方 `side` 的视角分**（无需取反）；Rapfi `ScalingFactor = 200.0`、`winRateToValue` 钳位在 `[-15_000, +15_000]`，严格低于本引擎 `WIN = INF = 20_000`，绝不与 `±(INF - ply)` 杀棋分冲突。 |
| 3 | **破坏既有精确战术杀棋判据** | `Mix9svq` 仅替代非必杀叶节点的静态评估：根部 `VCF + df-pn VCT` 与叶节点既有 `WIN5 / LAST5 / NEXT4 / NEXT43 -> ±WIN` 精确判据**全部保留在 NNUE 之前**，命中精确必杀直接返回 `±(INF - ply)` 且零 NNUE 开销。 |
| 4 | **不限时 `d8/w40` 变慢过多** | 纯 Value 阶段（阶段 2）单叶从 `~0.1 µs` 升至 `~2.0 µs`，`d8/w40` 墙钟可能变慢 `2x–3x`。应对策略分两层：<br>1. 阶段 2 先在 `1s/move` 和 `d8/w40` 验证 NNUE 纯 Value 棋力是否压倒性碾压 `master`（目标 `>= 80%`）；<br>2. 紧接在阶段 3 接入 **`Mix9svq` Policy 排序 + 窗口懒评估门控（`classicalEvalMargin`）**，通过大幅降低分支因子与跳过远离 `[alpha, beta]` 窗口的叶子，把 `d8/w40` 耗时拉回到持平甚至快于旧 `master`！ |
| 5 | **多线程 ABDADA 与 `play/undo` 状态失步** | 每个 `AlphaBetaSearcher`（包括 `WorkerLocal`）持有独立 `Option<Mix9svqEvaluator>`；每层 `DepthJob` 开始调用 `sync_with_board(board)`；`alphabeta.rs` 仅在配对的 `board.play / board.undo` 处调 `before_move / after_undo`，加 `debug_assert_eq!` 校验版本栈深度与 `board.move_count()` 一致。 |
| 6 | **破坏 `base` / `run_diff.py` 或污染仓库** | 保留 `RuntimeOptions::use_nnue`（与 `nnue_dir` / `INFO use_nnue` / `INFO nnue_dir`）；未提供权重或关闭时 100% 走原古典评估，`run_diff.py` 13/13 保持全绿；权重文件留在 `.gitignore` 覆盖的 `opponent/rapfi-networks/mix9svq/`，绝不提交进 git。 |

---

## 4. 三阶段执行计划与止损门禁

### 阶段 1：纯 Rust `Mix9svq` 推理内核 + AVX2 SIMD + C++ 比特级对齐（分支 `exp/m32-nnue-mix9svq`）
- **新增模块**：
  - `src/nnue/mod.rs`：权重路径解析、按规则（Freestyle/Renju）`Arc<Mix9svqWeight>` 全局缓存、`NnueEvaluator` 统一接口；
  - `src/nnue/lz4.rs`：纯 Rust 标准 LZ4 Frame 解压器；
  - `src/nnue/mix9svq.rs`：`StandardHeader` 校验、10-bit `codebook` 解包、`Mix9svqAccumulator` 版本栈、`MoveCache`、`evaluate_value`、`evaluate_policy`；
  - `src/nnue/simd.rs`：标量参考实现 + `x86_64` AVX2 内核（`#[target_feature(enable = "avx2")]`，运行时检测；预留 `aarch64` NEON）。
- **验证门禁（不达标不进阶段 2）**：
  1. 用 `g++` 编译本地临时对照程序（跑在 `/tmp/`）调用 `opponent/rapfi/Rapfi/eval/mix9svqnnue.cpp`，在空盘 + 20 个 Freestyle/Renju 局面（含混合 `play/undo`）上核对 `layer3i32` **100% 比特级一致**、全部 225 格 `policy` 误差 `< 1e-5`；
  2. 微基准测试：`move() + evaluate_value()` 单次耗时 `< 4 µs`（AVX2 目标 `~2 µs`），权重单次加载 `< 150 ms`。

### 阶段 2：接入 `Mix9svq` Value 到主搜索 + 双模式实力验收
- **改动范围**：
  - `src/config.rs` / `src/protocol/gomocup.rs` / `src/bin/gomocup_engine.rs`：增加 `use_nnue`、`nnue_dir`（支持 CLI `--nnue-dir` 与 Gomocup `INFO nnue_dir` / `INFO use_nnue`，以及环境变量 `RUST_GOMOKU_NNUE_DIR`）；
  - `src/search/alphabeta.rs` / `src/search/root.rs` / `src/search/abdada.rs`：挂载 `Option<Mix9svqEvaluator>`，配对 `before_move / after_undo`，叶节点在 `±WIN` 精确必杀未命中时调用 NNUE Value（含 `ScalingFactor = 200.0` 映射与可选 `classicalEvalMargin` 窗口门控消融测试）。
- **验证门禁（防空转：先快测再满测）**：
  1. `cargo fmt --check`、全量 `cargo test`、`android/rust_bridge` 测试、`scripts/run_diff.py --profile all --check-baselines`（默认无权重或 `use_nnue=0` 保持 13/13 全绿）；
  2. **40 局快速 smoke**（Renju `d6/w30` + `1s/move` vs `master` `8563005`）：目标得分率 **`>= 75%`**；若 `< 60%` 立即停下排查视角/尺度/缩放问题；
  3. **300 局正式验收**（`center_50_prefixes.jsonl`, `--reuse-engine-state`, 0 错误 / 0 超时）：
     - Renju 不限时 `d8/w40`（100 局）
     - Renju 限时 `1s/move`（100 局）
     - Freestyle 限时 `1s/move`（100 局）
     - 报告完整得分率、Elo、Pentanomial、`avg / median / p95 / max` 耗时。

### 阶段 3（阶段 2 大胜后再开启）：Policy 排序 + 耗时压缩 + 搜索升级
- 在非叶节点（`depth >= 2` / root）接入 `Mix9svq::evaluate_policy` 排序候选着（与既有 `tt_best_move` / 必杀迫着结合），实测节点数与 `d8/w40` 墙钟耗时缩减；
- 在高质量 NNUE Value + Policy 基础上，重新评估之前因旧评估受阻的 **4-way TT / Policy-LMR / 无 `running_downf` 现代搜索**；
- 补充 **ARM64 NEON SIMD** 以支持 Android 端极速运行；后续可按需使用本机 **RTX 5080** + `opponent/rapfi/Trainer` 微调/自训同格式权重。

---

## 5. 为什么这套方案几乎不会做“无用功”

1. **零猜测、零黑盒**：权重已经过世界顶级五子棋引擎验证，阶段 1 用 Rapfi C++ 做逐层比特级对拍，不存在“调参调半天不知道是权重差还是代码错”的问题。
2. **每一步都是长期资产**：纯 Rust LZ4、`Mix9svq` 版本栈累加器、AVX2/NEON 算子、Policy/Value 接口，后续哪怕用 5080 自训新权重也能 100% 复用。
3. **快速失败闭环**：C++ 对拍（分钟级）→ 微基准 µs/eval（秒级）→ 40 局 smoke（~2 分钟）→ 300 局全量验收，任何问题都在几分钟内暴露，绝不浪费半小时跑必输局。

---

## 6. 阶段 1–3a 实测结果与暂缓合并前的审计清单（2026-11-23）

### 6.1 已完成的 300 局中心开局验收（`exp/m32-nnue-mix9svq` `9f6b899` vs `master` `8563005`）

| 赛制 | 战绩 (W-L-D) | 得分率 / Elo | 配对 Pentanomial | 备注 |
| --- | :---: | :---: | :---: | --- |
| Renju 限时 `1s/move`（100 局） | `79-21-0` | `79.0%` / `+230.2 Elo` | `WW=29, WL=21, LD=0, LL=0` | 0 错误 / 0 超时，均耗 `491.5ms` vs `463.3ms` |
| Freestyle 限时 `1s/move`（100 局） | `71-28-1` | `71.5%` / `+159.8 Elo` | `WW=21, WD=1, WL=28, LD=0, LL=0` | 0 错误 / 0 超时，均耗 `491.0ms` vs `446.7ms` |
| Renju 不限时 `d8/w40`（100 局） | `79-18-3` | `80.5%` / `+246.6 Elo` | `WW=29, WD=3, WL=18, LD=0, LL=0` | 其中 3 局 225 手满盘和棋重跑完赛，`nnue` 均耗 `4.21s` vs `master` `5.41s`，但双方单手长尾均达 `66–68s` |

### 6.2 暂缓合并的原因与防返工修正后的修复路线（详见 [docs/optimization-roadmap.md](optimization-roadmap.md) 第 7 节）

1. **P0（合并前必修 + 零风险提速，全部可比特级/节点级验证）**：
   - `P0-1`：`src/search/root.rs:646` 覆盖 `alphabeta.config` 未触发 NNUE 重载，且 `load_nnue_for_config().ok()` 静默吞错；
   - `P0-2`：`src/protocol/gomocup.rs` `INFO nnue_dir` 按空白切分导致带空格路径被截断；
   - `P0-3`：NNUE Policy 排序抢跑在 `tt_best_move` 截断之前（推迟至 `index == 1`，100% 等价），且迭代加深每层入口都从空盘重放 `sync_with_board(board)`（改 LCP 增量同步）；顺带补齐标量 `evaluate_policy_moves` 仅算候选子集与 `evaluate_policy_avx2` 末层 `policy_dot16_avx2`；
   - `P0-4`：`src/bin/case_probe.rs` 补齐 `--nnue-dir` / `RUST_GOMOKU_NNUE_DIR` 支持；
   - `P0-5`：**提级 `AVX-VNNI`（`_mm256_dpbusd_epi32`）**：已实测 `rustc 1.96.0` 稳定版原生支持，~100 行代码、100% 比特级对齐、零搜索风险，直接为后续全部实验降耗 ~25%–40%。
2. **P1（依赖顺序已校正：先 4-way TT 含 `static_eval`，再逐项消融现代剪枝）**：
   - `P1-1`（前置）：**4-way 64B 对齐 NNUE TT（32-bit key tag + 96-bit 载荷，内置 `static_eval: i16` 与 `tt_pv`）+ 同 key 去重 + 世代替换 + 停写纯叶节点 + 修 `probe()` slot 1 瑕疵**（不破坏古典 `base` 13/13 diff）；
   - `P1-2`：**逐项消融 LMP（`~107 Elo`）→ 战术保护下 NNUE 浅层 RFP（`~121 Elo`，复用 TT `static_eval`）→ 归一化 softmax Policy-LMR（`~59 Elo`）与移除 `running_downf` 跨层余数 / 延伸上限**；每项同时对照 `master`（`8563005`）与 `M3.2`（`9f6b899`）做 `case_probe` + 40 局 smoke + 100 局全测。
3. **P2（实战与战术补齐）**：`P2-1` 解锁 `VCFM = 5` + Zobrist VCF TT + 证明集对手 VCF 过滤；`P2-2` 现代 `timectl` 与安全复用末层未完搜索；`P2-3` 多线程 ABDADA + NNUE 实测与去强屏障。
4. **P3（全平台落地与后置大改）**：`P3-1` ARM64 NEON SIMD；`P3-2` 桌面 GUI（`src/bin/gomoku_gui.rs` / `src/app.rs`）与 Android JNI Bridge 接入 NNUE；`P3-3` 轻量迫着 `qsearch` 与剥离 `EvalCaches`（**已从 P1 降级**：核查发现 `EvalCaches` 深度耦合 movegen/Renju 禁手快筛且叶子古典点积仅 `~0.1 µs`，避免过早大改返工）；`P3-4` Rapfi/SlowRenju 外部标定与 CI 合成权重回归。
