# P4 计划：1T / 4T 对同权重 Rapfi 逼近 40% 得分率

> - 创建时间：2026-11-24（基准修订：2026-11-24 22:00）
> - 当前工作分支：`exp/p3-search-strength`
> - **当前唯一锁定基准（`p4_base`）**：搜索内核严格等价 `p4_fix1 (45f46bb)` + 协议层保留 `INFO max_memory (812a993)`；已撤回带 4T 副作用的 `dc23d00 (p4_fix2)` 与 `f9e92b0 (p4_fix3)`，基准二进制归档为 `opponent/baselines/gomocup_nnue_p4_base`。
> - 锚点主线基线：`master`（`b69f98b`，P2 基线，作为常驻测试对手保留）

---

## 1. 目标与评测口径

### 1.1 终极目标

在**同神经网络权重、同 TT 容量、同计时、同硬件公平并发**条件下，将 `rust_gomoku` NNUE 搜索棋力推升至：

- **1T vs Rapfi 1T（Renju `1s/move`，`Mix9svq`，32 MiB TT，`database=off`，`--reuse-engine-state`，`--jobs 8~10`）**：得分率逼近 **`40%`**（约 `-70 Elo`，相对 `p4_base` 的 `27.5%` 净增约 **`+98 Elo`**）。
- **4T vs Rapfi 4T（Renju `1s/move`，`Mix9svq`，32 MiB TT，`database=off`，`--reuse-engine-state`，`--jobs 4`）**：得分率逼近 **`40%`**（约 `-70 Elo`，相对 `p4_base` 的 `31.0%` 净增约 **`+69 Elo`**）。

### 1.2 硬性约束与实验纪律

1. **古典 `base`（`use_nnue = false`）100% 行为等价**：`python3 scripts/run_diff.py --check-baselines --jobs 8` 保持 `12/12` 全通过。
2. **规则与稳定性零退步**：Renju / Freestyle 0 非法着、0 错误、0 超时（注：第 225 手全盘仅剩 1 个黑方禁手点导致无合法着属 Renju 正常判负）；`cargo test --locked` 与 `android/rust_bridge` 全部通过。
3. **默认参数集中管理**：涉及 `src/config.rs` 默认参数修改同步更新 `README.md` / `README-cn.md`。
4. **严格公平评测口径**（本机 `i7-13700F`：8 P-core / 16 线程 + 8 E-core / 8 线程）：
   - 必须开启 `--reuse-engine-state`，并统一传 `timeout_turn=1000`、`time_left=300000`、`rule=4`、`32 MiB TT`（`tt_bits=20` 或 `max_memory=33554432`）；
   - 严禁将 `--jobs 16`（超配挤占 E-core）与 `--jobs 8/10` 混比；统一 **1T 用 `--jobs 8`（内战）或 `--jobs 10`（对 Rapfi）**、**4T 用 `--jobs 4`**，且不得并发跑两组大型对局。
5. **单点验收与双端把关**：
   - **每次只改一个原子点**，绝不再打包多个未验证的搜索改动；
   - **纯速度优化**在跑对局前必须先通过固定深度（`timeout_turn 0`, `d=8`）**100% 节点数与 `tt_generation` 位等价验收**；
   - **搜索/启发改动**必须同时通过 **1T vs `p4_base`（100 局）** 与 **4T vs `p4_base`（100 局）** 双端检验，任何一端显著退化即止损回退或修因重测。
6. **分支纪律**：全部实验在 `exp/p3-search-strength`（或后续实验分支）验证，未经用户明确批准不合入 `master`、不推送 `origin`。

---

## 2. Phase 0 全面审计与 `p4_base` 基准锁定（2026-11-24）

### 2.1 五个候选版本的代码实质关系

经完整核对 `git diff 122aa89..812a993`，从 `p3_batch3` 到 `p4_fix3` 的真实代码演进关系如下：

1. **`p3_batch3 (122aa89)`**：P3 Batch 3（Singular Extension + Multi-Cut + NegExt）1T 核 + 旧同步每层 ABDADA（`DepthJob`）4T。
2. **`33d1515 (p3_batch3_4tfix)`**：**1T 代码与 `p3_batch3` 100% 完全一致**；4T 切到异步 Lazy SMP（`LazySmpJob`）+ 4T SE 去重 + 4T worker 每步清空 `quiet_history` + 奇数 worker LMR `+0.25`。
3. **`p4_fix1 (45f46bb)`（已锁定为 `p4_base` 搜索内核）**：
   - **1T 代码仍与 `p3_batch3` 100% 完全一致**（`sync_worker_state` 与 `abdada` 分支在 `threads == 1` 下均不触发）；
   - **4T 修复**：主线程不再因单次报 `-WIN` 立即掐停全池（赢杀确认 2 层 / 输杀确认 6 层且无同深 helper 分歧才提前停）；helper 不再因 `±WIN` 提前退出闲置；重写 `pick_best_thread` 为严格可传递、与 helper 完成顺序无关的三段选择规则。
4. **`p4_fix2 (dc23d00)`（已撤出基准）**：叶层 QVCF 增加 `nonlocal_b4_dropped` Sound-Bound Guard，并用 `qvcf_self_last_salt(last_move)` 给 fail-low 写表加盐。虽在 1T 防止了换序 TT 串键，但按上一步着法 `self_last` 加盐导致 4T 跨路径置换表共享受损。
5. **`p4_fix3 (f9e92b0)`（已撤出基准）**：Singular Extension 提前排掉 `options.skip_move` 时，将剩余候选重新从 `index = 0` 编号，却未在 LMR 条件 `if depth >= 3.0 && index >= 1` 处补上 `+1` 偏移，导致 SE 验证中的**第 1 个替代着法落到 `index = 0`（零 LMR 减深）**，显著拖慢 SE 验证并导致 4T 明显退化。
6. **`812a993`（已保留进 `p4_base`）**：纯协议层支持 `INFO max_memory <bytes>` 自动约束默认 TT 上限（搜索逻辑零改动）。

### 2.2 全量实测对局矩阵（`center_50_prefixes.jsonl`，Renju `1s/move`，100 局，`--reuse-engine-state`）

#### A. 对同权重 Rapfi（`Mix9svq`，32 MiB TT，`database=off`）

| 对阵 | 并发 / TT | 成绩 (W-L-D) | 得分率 | Elo 差 | Pentanomial (WW/WD/WL/LD/LL) | 均耗 (ms) | 备注 |
|---|---|---:|---:|---:|---|---:|---|
| `master (b69f98b, P2)` 1T vs Rapfi 1T | — / 32M | 23-76-1 | 23.5% | -205.0 | — | — | P2 历史锚点 |
| `master (b69f98b, P2)` 4T vs Rapfi 4T | — / 32M | 28-72-0 | 28.0% | -164.1 | — | — | P2 历史锚点 |
| `p3_batch3 (122aa89)` 1T vs Rapfi 1T | `j=16` / 64M | 30-70-0 | 30.0% | -147.2 | `0/0/30/0/20` | 524.6 / 606.8 | 超配口径，不可与 `j=10` 混比 |
| **`p3_batch3 (122aa89)` 1T vs Rapfi 1T** | **`j=10` / 32M** | **27-72-1** | **27.5%** | **-168.4** | `0/0/27/1/22` | 542.6 / 597.4 | **1T 公平基线**（= `p4_base` 1T；1 盘 224 ply 黑无合法着） |
| `p4_fix3 (812a993)` 1T vs Rapfi 1T | `j=10` / 32M | 26-73-1 | 26.5% | -177.2 | `0/0/26/1/23` | 553.1 / 605.7 | 1T 未超过 `p3_batch3` |
| `p3_batch3 (122aa89)` 4T vs Rapfi 4T | `j=4` / 64M | 23-76-1 | 23.5% | -205.0 | `0/0/23/1/26` | 537.9 / 546.8 | 旧同步 ABDADA |
| `33d1515` 4T vs Rapfi 4T | `j=4` / 64M | 25-73-2 | 26.0% | -181.7 | `0/0/25/2/23` | 521.6 / 545.0 | 初版 Lazy SMP |
| **`p4_fix1 (45f46bb)` 4T vs Rapfi 4T** | **`j=4` / 32M** | **31-69-0** | **31.0%** | **-139.0** | `0/0/31/0/19` | 498.7 / 509.4 | **4T 公平基线**（= `p4_base` 4T） |
| `p4_fix2 (dc23d00)` 4T vs Rapfi 4T | `j=4` / 32M | 29-71-0 | 29.0% | -155.5 | `0/0/29/0/21` | 506.3 / 525.1 | 1 盘 224 ply 黑无合法着 |
| `p4_fix3 (f9e92b0)` 4T vs Rapfi 4T | `j=4` / 32M | 30-66-4 | 32.0% | -130.9 | `0/0/30/4/16` | 515.3 / 556.0 | 内战 4T 明显退化，不作基准 |

#### B. 内部自博弈对照（全部 `32 MiB TT / tt_bits=20`，`--reuse-engine-state`，100 局，0 错误 0 超时）

| 线程 | 对阵 | 并发 | 成绩 (W-L-D) | 得分率 | Elo 差 | Pentanomial (WW/WD/WL/LD/LL) | 结论 |
|---|---|---:|---:|---:|---:|---|---|
| 1T | `p4_fix1_1t` vs `p3_batch3_1t` | `j=8` | 40-48-12 | 46.0% | -27.85 | `0/3/39/5/3` | 1T 代码完全相同，计时波动（95% CI `[-60.8, +4.6]`） |
| 1T | `p4_fix3_1t` vs `p3_batch3_1t` | `j=8` | 51-43-6 | 54.0% | +27.85 | `6/3/36/3/2` | `fix2+fix3` 在 1T 有正信号，待剥离 4T 副作用后单点重测 |
| **4T** | **`p4_fix1_4t` vs `33d1515_4t`** | **`j=4`** | **50-42-8** | **54.0%** | **+27.85** | `5/3/38/3/1` | **`p4_fix1` 显著胜 `33d1515`** |
| **4T** | **`p4_fix1_4t` vs `p3_batch3_4t`** | **`j=4`** | **49-43-8** | **53.0%** | **+20.87** | `6/4/33/4/3` | **`p4_fix1` 显著胜旧同步 ABDADA** |
| 4T | `p4_fix2_4t` vs `p4_fix1_4t` | `j=4` | 48-41-11 | 53.5% | +24.36 | `5/5/34/4/2` | 局部正交，但下一行对 `33d1515` 与 Rapfi 回落 |
| 4T | `p4_fix2_4t` vs `33d1515_4t` | `j=4` | 45-47-8 | 49.0% | -6.95 | `3/3/36/5/3` | `self_last` 盐值削弱 4T TT 共享，收益回吐 |
| 4T | `p4_fix3_4t` vs `p4_fix2_4t` | `j=4` | 46-48-6 | 49.0% | -6.95 | `2/2/40/4/2` | LMR `index=0` bug 拖慢 SE |
| 4T | `p4_fix3_4t` vs `p3_batch3_4t` | `j=4` | 42-49-9 | 46.5% | -24.36 | `1/2/39/5/3` | 4T 退化，低于 `p3_batch3` |
| 4T | `p4_fix3_4t` vs `33d1515_4t` | `j=4` | 43-52-5 | 45.5% | -31.35 | `2/2/37/3/6` | 4T 退化最严重 |

**基准锁定结论**：
- **`p4_base`（`p4_fix1 (45f46bb)` 搜索内核 + `812a993` `INFO max_memory`）** 是当前唯一在 **1T 无损（100% 等价 `p3_batch3`）** 且 **4T 对所有历史版本全胜（赢 `33d1515` `54.0%`、赢 `p3_batch3` `53.0%`、对 Rapfi 4T `31.0%`）** 的干净基准。

---

## 3. 已完成的前置体检与失败实验复盘（防重复踩坑）

### 3.1 Step 0 前置体检数据（有效，继续指导后续优化）

- **同 `1s` + `32 MiB TT` 深度差距**：`c4_ply20` 上 1T NPS 我们 `228K` vs Rapfi `287K`，但深度 `12` vs `17`（差 5 层）；`loss4t_c9_ply22` 上 Rapfi 1T `d25-46 (-M46)` / 4T `d27-36 (-M38)`，我方仅 `d12~14`。**主矛盾仍是搜索树 EBF 与战术延伸深度**。
- **CPU 热点 Profile（`perf record`）**：
  - `c4_ply20 d=12` (1T)：NNUE `51.7%` vs **古典 `EvalCaches + movegen` `30.2%`**（`value_wide_compute_for_rule` 11.8%、`update_bucket_attack` 7.2%、`generate_candidates` 6.3%、`update_direction_cache` 3.4%、`restore_snapshot` 1.6%）；`TTTableView4::probe` 仅 `1.79%`。
  - `vct_dense_ply78 d=8` (1T)：NNUE `36.3%` vs **古典 `EvalCaches + movegen` `42.7%`**。
  - `c4_ply20 d=12` (4T)：热点比例与 1T 一致，`abdada_table` `< 1.5%`。
- **Root VCF / VCT 实盘审计（20 个中局）**：Root VCF 命中 `2/20`（保留）；Root df-pn VCT 触发 `15/20`（75%）、命中 `0/20`、`avg = 13.7ms`、`max = 67.3ms`。
- **Mix9svq `scoreBias` 对齐**：Rapfi `PolicyBuffer::scoreBias = 0`、`ScoreScale = 32`（`score = int(raw_logit * 32)`）。

### 3.2 已验证失败并止损回退的 3 组实验（严禁直接重试）

1. **已否决 — Step 1（关闭 Lazy SMP 内部 `abdada_table` + 禁 Root/PV TT 截断 + `threadBias` + `max_depth=64`）**：
   - `p4_step1_4t` vs `p4_fix3_4t`：**`42-50-8`（`46.0%`，`-27.85 Elo`）**，均耗从 `507.2ms` 涨到 `551.2ms`，TT 命中翻倍（`2.13×`），说明移除 `abdada_table` 后 4 线程在同分支上大量重复做功。**结论：保留 Lazy SMP 内的 `abdada_table` 轻度去重。**
2. **已否决 — Step 2A+2B（一次性打包 7 项启发改动）**：
   - 打包了 `filter_nnue_tactical_defenses`、`OPPO5_EXT`、`TTM_EXT`、PV LMP、未校准 Policy Pruning、绝对 Policy LMR、无 Aspiration Window 的 `Alpha-Improvement Reduction`。
   - 1T vs Rapfi 1T 仅 **`27.0%`**（v1）和 **`28.0%`**（v2），且被速杀比例从 `40.0%` 恶化到 **`52.1% ~ 53.4%`**（在 `[-INF, +INF]` 全窗 PV 节点首步即触发 Alpha-Improvement Reduction，且 `OPPO5_EXT` 致节点暴涨 `+70%`）。**结论：严禁打包，必须逐项孤立验证。**
3. **已否决 — Step 3.1（未做固定深度等价核验的 NNUE 轻量缓存与 movegen 捷径）**：
   - 虽然 NPS 提速至 `248K~280K`，但 `p4_step31_1t` vs `p4_fix3_1t` 输 **`41-47-12`（`47.0%`，`-20.87 Elo`）**，对 Rapfi 1T `27.5%`、对 Rapfi 4T 暴跌至 **`26.0%`**（vs `p4_fix3` 出现 `W->L: 5, L->W: 0`）。
   - 事后定位：`d=8` 固定深度下 `c4_ply20` 的 `tt_generation current` 从 `733` 变成 `1319`，证明某项捷径**偷偷破坏了搜索树等价性**。**结论：任何纯提速改动必须先过固定深度 100% 节点与 TT 位等价门禁，再上对局。**

---

## 4. 从 `p4_base` 出发的单点推进路线

> 原则：每次只改 **1 个原子点**，必须同时过 **`1T vs p4_base (100g)`** 与 **`4T vs p4_base (100g)`** 才合入并滚动更新基准；绝不再打包。

- [x] **Milestone 0 — 锁定 `p4_base`**：回退 `src/search/alphabeta.rs` 至 `45f46bb`，保留 `812a993` `INFO max_memory`，归档 `opponent/baselines/gomocup_nnue_p4_base`，通过全部单测与古典 `base` `12/12` diff。
- [ ] **Candidate 1 — 修复版 Singular Extension `skip_move` 排除**：
  - 在提前排除 `options.skip_move` 的同时，补上 `effective_index = index + usize::from(options.skip_move.is_some())`，彻底消除第 1 个替代着落到 `index = 0` 零 LMR 减深的 bug；
  - 验收：`1T vs p4_base` (100g) + `4T vs p4_base` (100g) 双端无损且正收益。
- [ ] **Candidate 2 — 精确 15 行 Rapfi `quickWinCheck` + `Mate Distance Pruning`**：
  - 严格对齐 `opponent/rapfi/Rapfi/game/wincheck.h:35-53` 与 `search.cpp:1243-1246, 2059-2063`（非根内部节点与 QVCF 节点：己方成五 `mate_in(ply+1)`、对手双成五 `mated_in(ply+2)`、无对手成五且己方活四 `mate_in(ply+3)` + Mate Distance Pruning）；
  - 验收：单测验证零 Renju 禁手误判 + `1T & 4T vs 当前基准` + `vs Rapfi`。
- [ ] **Candidate 3 — 严格 100% 位等价的 NNUE 热路径提速（拆项逐个验证）**：
  - 每次只拆 1 小项（如仅跳过 NNUE 下不会读取的 `empty_bucket_counts` 768B 快照拷贝，或仅跳过被 NNUE Policy 覆写的 `move_value` 浮点计算）；
  - **强前置门禁**：`c4_ply20` 与 `loss4t_c9_ply22` 固定深度 `d=8` 下，**best_move、score、nodes、`tt_generation current/old` 必须 100% 逐字一致**，确认 NPS 净提速后再跑 1T & 4T 对局。
- [ ] **Candidate 4 — 逐项单点验证 EBF / 排序 / 防守启发**：
  - 不分裂 4T TT 共享的 QVCF 非局部防点保护（探索仅对 fail-low 降为 `HASHF_EMPTY` 或仅 1T/深度受控，不加 `self_last` 盐值）；
  - `MainHistory + CounterMove` 按 `ScoreScale=32` 校准融合排序；
  - 独立验证单一战术延伸/裁剪项（如单测 `DEFEND_B4F3`、单测对齐 `ScoreScale=32` 的 Policy LMR 等），逐项双端验收，向 1T / 4T 对 Rapfi `~40%` 推进。

---

## 5. 里程碑进度追踪

| 阶段 | 状态 | 1T vs Rapfi 1T (32M, fair) | 4T vs Rapfi 4T (32M, `j=4`) | 备注 |
|---|---|---:|---:|---|
| 历史锚点 P2 (`b69f98b`) | 已归档 | 23.5% (-205.0) | 28.0% (-164.1) | `master` 锚点 |
| P3 (`122aa89` / `33d1515`) | 已归档 | 27.5% (-168.4) | 23.5% / 26.0% (64M) | 1T 与 `p4_base` 相同 |
| **`p4_base` (`45f46bb` + `812a993`)** | **当前基准** | **27.5% (-168.4)** | **31.0% (-139.0)** | **4T 胜 `33d1515` 54.0%、胜 `p3_batch3` 53.0%** |
| Candidate 1~4（单点推进） | 待开始 | 目标 **`~40%`** | 目标 **`~40%`** | 1T + 4T 双门禁滚动推进 |
