# `rust_gomoku` 冲击对 Rapfi 6:4（1T & 4T 双线）技术路线图（已审核版）

## 1. 目标与基线

### 当前已锁定的基线（`master @ b7d750e` / `v0.1.20`）

在严格同等条件下（同 `Mix9svq` 权重、`32 MiB TT`、`1s/move`、`database=off`、`--reuse-engine-state`、`center_50_prefixes.jsonl` 双边对局）：

| 对阵 | 规则 | 线程 | 战绩 (W-L-D) | 得分率 (95% CI) | Elo 差 | 错误 / 超时 |
|---|---|---:|---|---:|---:|---:|
| `v0.1.19` vs `pbrain-rapfi` | Renju | 1T | `43-44-13` | `49.5% ± 3.82%` | `-3.47` | `0 / 0` |
| `v0.1.20 (016382b)` vs `pbrain-rapfi` | Renju | 1T | `41-42-17` | `49.5% ± 3.58%` | `-3.47` | `0 / 0` |
| `v0.1.19` vs `pbrain-rapfi` | Renju | 4T | `42-41-17` | `50.5% ± 3.58%` | `+3.47` | `0 / 0` |
| `v0.1.19` vs `pbrain-rapfi` | Freestyle | 1T | `44-45-11` | `49.5% ± 3.02%` | `-3.47` | `0 / 0` |

- 开发机硬件：`Intel Core i7-13700F`（24 线程，支持 `avx2 / avx_vnni / bmi1 / bmi2 / fma`，**无 AVX-512**）+ `NVIDIA GeForce RTX 5080`。
- `OPEN_5` 4T 1s 基线吞吐：`1.39M NPS` vs Rapfi `1.63M NPS`（约 Rapfi 的 `85%`）。
- 经典 `pygomoku` `base`（`use_nnue = false`、休闲模式、桌面/Android 端）保持 `13/13` 基准全绿。

### 下一阶段目标

- **核心指标**：在同等硬件与时限条件下（`1s/move`、`32 MiB TT`、`database=off`、`--reuse-engine-state`），**1T 与 4T 双线对 `pbrain-rapfi` 均稳定达到 6:4（得分率 ≥ `60%`，约 `+70.4 Elo`）**，95% 置信区间下界稳健超过 `55%`，连珠与无禁手均保持 **0 错手、0 超时**。
- **架构边界**：
  - **Phase 1–3（同 `Mix9svq` 权重内核超越）**：通过真实热点提速、现代搜索算法（Stockfish 15–17 级）与 SPSA 联合调参，在严格同权重下先把内核做到 **`57% ~ 61%`**（`+50 ~ +80 Elo`）。
  - **Phase 4（RTX 5080 评估网络升级）**：利用本机 `RTX 5080` 微调/蒸馏更强的 `Mix9svq` 兼容权重（或升级架构），同时兼容桌面 x86_64 与 Android ARM64 NEON+`dotprod`，将 1T & 4T 双线彻底锁死在 **`62% ~ 68%+`**。
  - **不变约束**：经典 `base` 主线（`src/search/`）100% 不动，`scripts/run_diff.py --profile all --check-baselines` 始终 `13/13` 全绿。

### 审核排除的无效/高风险方向

在启动 Phase 1 前对照代码与硬件核实，已明确剔除或降级以下方向：
1. **“新增 Lazy Accumulator”已排除**：[mix9svq.rs](../src/nnue/mix9svq.rs) 与 Rapfi 均已实现 `move_cache` 延迟队列与相反步抵消，不存在这项增量红利。
2. **“AVX-512 VNNI 内核”已排除**：本机 `i7-13700F` 与绝大多数消费级 12/13/14 代酷睿均无 AVX-512。
3. **“Cut-Node 朴素跳过 Policy 先试 Killer”降级为低优先备选**：`Mix9svq` Policy Top-1 命中率高，且 `policyPruning` / `LMR` 依赖 `raw_policy`；盲目重排极易破坏排序增加节点。
4. **“4T 中永久抽 1 核跑独立 df-pn”降级为备选**：4 核抽 1 核等于损失 25% AB 吞吐（约 -12 Elo），且需在 `FastBoard` 重写并重验连珠禁手零误杀，性价比偏低。

---

## 2. 务实四大阶段拆解

### Phase 1：基于真实 `perf` 热点的吞吐反超（目标：有效 NPS 达 Rapfi 的 `1.05× ~ 1.25×`，预期 `+10 ~ +18 Elo`）

1. **P1.1 真实热点采样与构建指令集对齐**
   - 用 `perf` 对 `FastEngine` 做 1T/4T 热点剖析，量化 `Mix9svq`、`FastBoard`、`MovePicker`、`FastTT`、`search` 各自的真实占比。
   - 验证 `x86-64-v3`（开启 `POPCNT / BMI1 / BMI2 / AVX2 / FMA`，消除基线 SSE2 模拟）及运行时多版本/编译配置对 `Bitboard225` 与整体 NPS 的真实收益（同时注意发版二进制的 x86_64 兼容性策略）。
2. **P1.2 `Mix9svq` 与 `FastEngine` 热路径微架构优化（100% 搜索等价）**
   - 将 `Mix9svqEvaluator::move_cache` 从堆 `Vec` 改为定长栈缓冲，减少分配与间接访存；消除内层评估循环中重复的运行时 `use_avx2 / use_avxvnni` 特征分发与冗余边界检查。
   - 按 `perf` 实测结果优化 `evaluate_policy_moves`、`play_move` / `commit_conv`、`FastBoard::play_typed`、`MovePicker` 与 `tt.prefetch` 时机。
3. **P1.3 PGO（Profile-Guided Optimization）构建**
   - 采集连珠 + 无禁手真实对局 profile，启用 LLVM PGO 压榨分支预测与代码布局红利（通常稳定 `+8% ~ +15%` NPS）。

---

### Phase 2：搜索算法跨代超越——补齐 Stockfish 15–17 级特性（预期 `+30 ~ +50 Elo`）

严格**单因子分支独立验证**，每项过 100 局自对弈/对 Rapfi 门槛才合入，绝不捆绑未验证启发式。

1. **P2.1 Correction History（静态评估偏差修正表，预期 `+15 ~ +25 Elo`）**
   - 维护轻量 `CorrectionHistory`（按 `side` + 局部棋型/威胁态势哈希 + 近期着法哈希索引）：在非强制战术节点搜索结束后，用 `best_value - static_eval` 做指数移动平均更新修正量；后续节点先执行 `eval = static_eval + corr`，再喂给 RFP / Razoring / NMP / Futility / LMR。
2. **P2.2 2-ply / 4-ply Continuation History（连续多步历史表，预期 `+8 ~ +15 Elo`）**
   - 将 Rapfi 的 1-ply `CounterMoveHistory` 升级为 `ContHist[ply-1] + ContHist[ply-2] + ContHist[ply-4]`，贴合五子棋“己方多手连续构型”的攻防特征，联合驱动候选排序、Quiet Pruning 与 LMR。
3. **P2.3 ProbCut + Negative Singular Extension + Low-Ply History（预期 `+8 ~ +15 Elo`）**
   - **ProbCut**：`depth >= 7` 且非必胜分时，对少数高威胁/高 Policy 候选做浅层零窗探测，显著超出 `beta + margin` 时提前截断。
   - **Negative SE**：排除搜索仍远超 beta 时反向缩深，把算力留给真正分叉的复杂节点。
   - **Low-Ply History**：为浅层（`ply < 4`）维护独立高频历史表，降低根部附近反复 fail-high/fail-low 重搜。

---

### Phase 3：SPSA 联合调参 + 4T 多线程扩展优化（预期 `+10 ~ +20 Elo`）

**目标**：同 `Mix9svq` 权重下将 1T & 4T 整体推向 **`57% ~ 61%`**。

1. **P3.1 平衡开局库扩充 + SPSA 自动调参闭环**
   - 在 `center_50_prefixes.jsonl` 基础上扩充至 200–400 个均衡连珠与无禁手开局前缀。
   - 基于 [run_gomocup_match.py](../scripts/run_gomocup_match.py) 搭建轻量 SPSA 超短时/定节点自对弈调参脚本，针对引入 Phase 1–2 后的新搜索树重新联合优化 `LMR`、`RFP`、`Razoring`、`NMP`、`SE`、`ProbCut`、`CorrectionHistory` 权重与 `PolicyReduction` 参数。
2. **P3.2 4T Diverse SMP 与跨线程协同优化**
   - 优化 helper 线程的差异化深度/LMR/Aspiration 调度与低开销浅层防重复机制，提升 32 MiB TT 下的 4T 扩展效率。

---

### Phase 4：评估网络升级（RTX 5080，彻底锁死双线 `62% ~ 68%+`）

1. **兼容现有推理内核的权重升级（优先路线）**：利用本机 `RTX 5080` + 24 线程自对弈/高深度标注数据，针对连珠禁手边角、双活三攻防转换与中残局微调或蒸馏更强的 **`Mix9svq` 兼容权重**——直接复用现有 x86_64 AVX2/AVX-VNNI 与 Android ARM64 NEON+`dotprod` 内核，零推理回归风险。
2. **可选架构升级**：若 `Mix9svq` 表达容量见顶，再评估兼容 `Mix10` 或自研更紧凑增量架构（同样要求桌面与 Android ARM64 NEON 端到端低延迟）。

---

## 3. 执行节奏、验证门槛与止损规则

### 推进里程碑

| 里程碑 | 内容 | 核心验证指标 | 目标对 Rapfi 胜率 |
|---|---|---|---:|
| **M1（Phase 1.1–1.2）** | `perf` 热点消除 + `Mix9svq`/搜索微优化 + ISA 对齐 | 固定深度 100% 行为等价；有效 NPS 反超 Rapfi | `51% ~ 53%` |
| **M2（Phase 1.3）** | PGO 构建闭环 | 综合有效 NPS `+20% ~ +35%`（达 Rapfi `1.05×~1.25×`） | `52% ~ 54%` |
| **M3（Phase 2.1）** | Correction History | 100 局自对弈 > `52%` + 对 Rapfi 明显正收益 | `55% ~ 57%` |
| **M4（Phase 2.2–2.3）** | 2/4-ply ContHist + ProbCut + Negative SE + Low-Ply Hist | 单因子逐项过关，长尾无恶化 | `57% ~ 59%` |
| **M5（Phase 3）** | SPSA 联合调参 + 4T Diverse SMP | 1T/4T 同权重双线实测 | `58% ~ 61%` |
| **M6（Phase 4）** | RTX 5080 权重微调/蒸馏（含 ARM64 NEON 验证） | 1T/4T 连珠 + 无禁手全线压制 | **`62% ~ 68%+`** |

### 强制止损与质量门禁

1. **单因子隔离**：每一项搜索启发式必须单独验证；若 100 局自对弈或对 Rapfi 未见正收益、或长尾 `p95/max` 明显恶化，立即回退并记入 [perf-log.md](perf-log.md)。
2. **正确性底线**：任何阶段合并前均须通过 `cargo fmt --check`、`git diff --check`、`cargo test --locked`、`android/rust_bridge` 测试、`scripts/run_diff.py --profile all --check-baselines --jobs 8`（经典 `13/13`）以及 0 错手/0 超时校验。
3. **最终 6:4 验收标准**：
   - 连珠 1T（≥100–200 局）：得分率 ≥ `60%`
   - 连珠 4T（≥100–200 局）：得分率 ≥ `60%`
   - 无禁手 1T / 4T（≥100 局）：得分率 ≥ `60%`
   - 桌面 GUI 与 Android ARM64 APK/AAB 均完成端到端构建与验证。
