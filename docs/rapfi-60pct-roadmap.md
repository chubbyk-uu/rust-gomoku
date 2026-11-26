# `rust_gomoku` 冲击对 Rapfi 6:4（1T & 4T 双线）技术路线图

## 1. 目标与基线

### 当前已锁定的基线（`master @ 016382b` / `v0.1.19`）

在严格同等条件下（同 `Mix9svq` 权重、`32 MiB TT`、`1s/move`、`database=off`、`--reuse-engine-state`、`center_50_prefixes.jsonl` 双边对局）：

| 对阵 | 规则 | 线程 | 战绩 (W-L-D) | 得分率 (95% CI) | Elo 差 | 错误 / 超时 |
|---|---|---:|---|---:|---:|---:|
| `v0.1.19` vs `pbrain-rapfi` | Renju | 1T | `43-44-13` | `49.5% ± 3.82%` | `-3.47` | `0 / 0` |
| `master @ 016382b` vs `pbrain-rapfi` | Renju | 1T | `41-42-17` | `49.5% ± 3.58%` | `-3.47` | `0 / 0` |
| `v0.1.19` vs `pbrain-rapfi` | Renju | 4T | `42-41-17` | `50.5% ± 3.58%` | `+3.47` | `0 / 0` |
| `v0.1.19` vs `pbrain-rapfi` | Freestyle | 1T | `44-45-11` | `49.5% ± 3.02%` | `-3.47` | `0 / 0` |

- `OPEN_5` 4T 1s 实测吞吐：`1.39M NPS` vs Rapfi `1.63M NPS`（约 Rapfi 的 `85%`）。
- 经典 `pygomoku` `base`（`use_nnue = false`、休闲模式、桌面/Android 端）保持 `13/13` 基准全绿。

### 下一阶段目标

- **核心指标**：在同等硬件与时限条件下（`1s/move`、`32 MiB TT`、`database=off`、`--reuse-engine-state`），**1T 与 4T 双线对 `pbrain-rapfi` 均稳定达到 6:4（得分率 ≥ `60%`，约 `+70.4 Elo`）**，95% 置信区间下界稳健超过 `55%`，连珠与无禁手均保持 **0 错手、0 超时**。
- **架构边界**：
  - **Phase 1–3**：坚持在**严格同一套 `Mix9svq` 权重**下推进，把底层吞吐、搜索算法和多线程并行做到全面领先 Rapfi。
  - **Phase 4**：在引擎内核已压过 Rapfi 的基础上，允许升级评估网络（如 `Mix10` 架构或自训/蒸馏权重），同时兼顾桌面 x86_64（AVX2 / AVX-VNNI / AVX-512）与 Android ARM64（NEON + `dotprod`），彻底锁死双线 6:4 并冲击 `65%+`。
  - **不变约束**：经典 `base` 主线（`src/search/`）100% 不动，`scripts/run_diff.py --profile all --check-baselines` 始终 `13/13` 全绿。

---

## 2. 四大技术支柱与拆解

### Phase 1：底层微架构与 NNUE 懒更新——有效 NPS 反超 Rapfi `1.4× ~ 1.7×`

**目标**：同 `Mix9svq` 权重下，1T & 4T 胜率推升至 **`54% ~ 56%`**（约 `+30 ~ +40 Elo`），搜索树等价、几乎零棋力回退风险。

1. **P1.1 Lazy Accumulator（NNUE 累加器延迟物化栈）**
   - **现状痛点**：当前每次 `make_move` 都会同步更新 `Mix9svq` 4 方向特征并做 `commit_conv`；但 Alpha-Beta 中有 `30%–45%` 的子节点会立即命中 TT 截断、`quick_win_check`、单防点直通或浅层 mate，从未调用 `evaluate_value()` / `evaluate_policy()`。
   - **改造方案**：在 `Mix9svqEvaluator` 维护轻量 dirty 栈，`make_move` 仅压入 `(side, x, y)`（约 1ns），真正需要静态评估或 Policy 推理时再顺着未同步层一次性刷新累加器；`unmake_move` 若该层尚未物化直接弹栈（零开销）。
   - **预期收益**：TT hit、必胜链与 QVCF 节点的 NNUE 增量更新归零，有效 NPS 提升 `+25% ~ +45%`。
2. **P1.2 顶层 ISA 单态化分发 + AVX-512 VNNI + TT 提前 Prefetch**
   - 在 `FastSearcher::search` 顶层按 CPU 指令集分发泛型实现（`AVX-512 VNNI` / `AVX-VNNI` / `AVX2` / `NEON+DOTPROD`），消除热路径上的特征检测分支并促使 LLVM 彻底内联。
   - 把 `tt.prefetch(child_key)` 提前到 `FastBoard::play_typed` 更新 `Pattern4` 之前，掩盖 DRAM 随机访问延迟；顺带压缩每线程热点结构的工作集。
3. **P1.3 PGO（Profile-Guided Optimization）构建流水线**
   - 用代表性连珠 + 无禁手对局 profile 采集分支热点，在发布与基准二进制构建中启用 PGO（分支密集型搜索引擎通常稳定获得 `+8% ~ +15%` 纯 NPS 收益）。

---

### Phase 2：搜索算法跨代超越——补齐 Stockfish 15–17 级特性

**目标**：同 `Mix9svq` 权重下，1T & 4T 累计胜率推升至 **`57% ~ 60%`**（累计 `+55 ~ +70 Elo`）。每项严格**单因子分支独立验证**，禁止捆绑未验证启发式。

1. **P2.1 Cut-Node 懒 Policy 阶段（延迟整盘 Policy Head）**
   - 非 PV / 高截断概率节点在 TT move 之后，先试探零 NNUE 成本的高优先着法（`Killer`、`CounterMove`、`C_BLOCK4_FLEX3` / `E_BLOCK4` / `F_FLEX3_2X` 等强战术着）；若前 2–3 手产生 beta cutoff，直接跳过整盘 Policy Head 推理。
2. **P2.2 Correction History（静态评估偏差修正表）**
   - 维护轻量的 `CorrectionHistory`（按 `side` + 局部棋型/威胁态势哈希 + 近两步着法哈希索引）：在非强制战术节点搜索结束后，用 `best_value - static_eval` 做指数移动平均更新修正量；后续节点先修正 `eval = static_eval + corr`，再用于 RFP / Razoring / NMP / Futility / LMR。
3. **P2.3 2-ply / 4-ply Continuation History（连续多步历史表）**
   - 将 Rapfi 的 1-ply `CounterMoveHistory` 升级为 `ContHist[ply-1] + ContHist[ply-2] + ContHist[ply-4]`，贴合五子棋“己方多手连续构型”的攻防特征，共同驱动排序、Quiet Pruning 与 LMR 减深。
4. **P2.4 ProbCut + Negative Singular Extension + Low-Ply History**
   - **ProbCut**：`depth >= 7` 且非必胜分时，对少数高威胁/高 Policy 候选做浅层零窗探测，显著超出 `beta + margin` 时提前截断。
   - **Negative SE**：排除搜索仍远超 beta 时适度缩深，把算力集中到真正分叉的复杂节点。
   - **Low-Ply History**：为浅层（`ply < 4`）维护独立高频历史表，降低根部附近反复 fail-high/fail-low 的重搜比例。

---

### Phase 3：SPSA 联合调参 + 4T 异构战术协作

**目标**：在**同 `Mix9svq` 权重**下冲刺并逼近/站稳 **1T & 4T 双线 `60%`**。

1. **P3.1 平衡开局库扩充 + SPSA 自动调参闭环**
   - 在现有 `center_50_prefixes.jsonl` 基础上扩充至 200–400 个均衡连珠与无禁手开局前缀。
   - 基于 [run_gomocup_match.py](../scripts/run_gomocup_match.py) 搭建轻量 SPSA（Simultaneous Perturbation Stochastic Approximation）超短时/定节点自对弈调参器，针对引入 Phase 1–2 后的新搜索树重新联合标定 `LMR`、`RFP`、`Razoring`、`NMP`、`SE`、`ProbCut`、`CorrectionHistory` 权重与 `PolicyReduction` 参数。
2. **P3.2 4T Diverse SMP + 零 NNUE 开销纯 `FastBoard` 战术证明协作线程**
   - 给 helper 线程引入轻量差异化参数扰动与浅层 soft-deferral，降低 32 MiB TT 下的多核重复搜索。
   - 在 `threads >= 4` 且检测到中盘高威胁/进攻态势时，动态调度 1 个协作线程在纯 `FastBoard`（**不运行 NNUE**）上执行高速深层 **VCF / df-pn VCT**，将证明出的精确必胜/必败界限写入共享 `FastTT`，供主搜索线程实时吸收。

---

### Phase 4：评估网络升级（彻底锁死双线 `60%+`，冲击 `65%+`）

**目标**：在不破坏 `Mix9svq` 兼容性的前提下，突破旧网络表达上限，将 1T & 4T 双线稳稳锁定在 **`60% ~ 68%+`**。

1. **新架构支持**：实现更高表达力且保持 CPU 友好的增量 NNUE 架构（如兼容 `Mix10` 或自研更紧凑的局部注意力/残差增量结构），同步提供 x86_64 AVX2/AVX-VNNI/AVX-512 与 Android ARM64 NEON+`dotprod` SIMD 内核。
2. **权重升级与蒸馏**：优先在连珠禁手边角、复杂双活三转换与长程攻防局面上做高质量数据增强、自对弈微调或大模型蒸馏，保证手机端包体大小与单步延迟可控。

---

## 3. 执行节奏、验证门槛与止损规则

### 推进顺序

| 里程碑 | 内容 | 核心验证指标 | 目标对 Rapfi 胜率 |
|---|---|---|---:|
| **M1（Phase 1.1）** | `Mix9svq` Lazy Accumulator | 固定深度 `d15` 节点/着法 100% 一致；有效 NPS `+25%+` | `52% ~ 54%` |
| **M2（Phase 1.2–1.3）** | 顶层 ISA 分发 + Prefetch + AVX-512 + PGO | 有效 NPS 达 Rapfi 的 `1.4× ~ 1.7×` | `54% ~ 56%` |
| **M3（Phase 2.1–2.2）** | Cut-Node 懒 Policy + Correction History | 单因子各过 100 局自对弈/对 Rapfi 正收益 | `56% ~ 58%` |
| **M4（Phase 2.3–2.4）** | 2/4-ply ContHist + ProbCut + Negative SE | 单因子各过 100 局，0 长尾恶化 | `58% ~ 60%` |
| **M5（Phase 3）** | SPSA 联合调参 + 4T 异构 df-pn 协作 | 1T/4T 双线 200 局实测 | `59% ~ 62%` |
| **M6（Phase 4）** | 评估网络升级（含 ARM64 NEON 验证） | 1T/4T 连珠 + 无禁手全线压制 | `62% ~ 68%+` |

### 强制止损与质量门禁

1. **单因子隔离**：每一项搜索启发式必须单独开提交验证；若 100 局自对弈或对 Rapfi 胜率未见正收益、或长尾 `p95/max` 明显恶化，立即回退并记入 [perf-log.md](perf-log.md)。
2. **正确性底线**：任何阶段合并前均须通过 `cargo fmt --check`、`git diff --check`、`cargo test --locked`、`android/rust_bridge` 测试、`scripts/run_diff.py --profile all --check-baselines --jobs 8`（经典 `13/13`）以及 0 错手/0 超时校验。
3. **最终 6:4 验收标准**：
   - 连珠 1T（≥100–200 局）：得分率 ≥ `60%`
   - 连珠 4T（≥100–200 局）：得分率 ≥ `60%`
   - 无禁手 1T / 4T（≥100 局）：得分率 ≥ `60%`
   - 桌面 GUI 与 Android ARM64 APK/AAB 均完成端到端构建与验证。
