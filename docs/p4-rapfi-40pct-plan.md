# P4 计划：1T / 4T 对同权重 Rapfi 逼近 40% 得分率

> - 创建时间：2026-11-24
> - 当前工作分支：`exp/p3-search-strength`（HEAD: `33d1515`）
> - 锚点主线基线：`master`（`b69f98b`，P2 基线，作为常驻测试对手保留）

---

## 1. 目标与评测口径

### 1.1 终极目标

在**同神经网络权重、同 TT 容量、同计时**的公平条件下，将 `rust_gomoku` NNUE 搜索棋力推升至：

- **1T vs Rapfi 1T（Renju `1s/move`，`Mix9svq`，32 MiB TT，`database=off`）**：得分率逼近 **`40%`**（约 `-70 Elo`，相对当前 `30.0%` 净增约 **`+77 Elo`**）。
- **4T vs Rapfi 4T（Renju `1s/move`，`Mix9svq`，32 MiB TT，`database=off`）**：得分率逼近 **`40%`**（约 `-70 Elo`，相对当前 `26.0%` 净增约 **`+112 Elo`**），彻底消除 4T 相对 1T 的额外落后。

### 1.2 硬性约束

1. **古典 `base`（`use_nnue = false`）100% 行为等价**：`run_diff.py --check-baselines` 保持 `12/12` 全通过。
2. **规则与稳定性零退步**：Renju / Freestyle 0 非法着、0 错误、0 超时；`cargo test --locked` 与 `android/rust_bridge` 全部通过。
3. **默认参数集中管理**：涉及 `src/config.rs` 默认参数修改（如 `DEFAULT_TIMED_SEARCH_MAX_DEPTH`）同步更新 `README.md`。
4. **常驻锚点与止损纪律**：保留 `P2 (b69f98b)` 与 `P3 (33d1515)` 作为历史锚点；每阶段（或每个高协同改动包）对上一阶段基线得分率 `< 50%` 即止损回退或拆解归因。
5. **分支纪律**：全部实验在 `exp/p3-search-strength`（或后续实验分支）验证，未经用户明确批准不合入 `master`、不推送 `origin`。

### 1.3 当前基线成绩（Renju `1s/move`，100 局，32 MiB TT，`Mix9svq`）

| 对阵 | 成绩 (W-L-D) | 得分率 | Elo 差 | 备注 |
|---|---:|---:|---:|---|
| `master (b69f98b, P2)` 1T vs Rapfi 1T | 23-76-1 | 23.5% | -205.0 | P2 锚点 |
| `master (b69f98b, P2)` 4T vs Rapfi 4T | 28-72-0 | 28.0% | -164.1 | P2 锚点 |
| **`33d1515 (P3)` 1T vs P2 1T** | 56-42-2 | 57.0% | +49.0 | 相对 P2 净涨 ~+58 Elo（对 Rapfi） |
| **`33d1515 (P3)` 1T vs Rapfi 1T** | **30-70-0** | **30.0%** | **-147.2** | 40.0% 负局以被连杀结束 |
| **`33d1515 (P3)` 4T vs P3 1T** (50g) | 29-17-4 | 62.0% | +85.0 | 自战 4T 强于 1T，但对 Rapfi 4T 吃亏 |
| **`33d1515 (P3)` 4T vs Rapfi 4T** | **25-73-2** | **26.0%** | **-181.7** | **58.9%** 负局以被连杀结束 |

---

## 2. 核心差距诊断（对照 Rapfi 源码）

### 2.1 初步实测信号

在 3 个典型局面（`vct_dense_ply78`、`c4_ply20`、`loss4t_c9_ply22`）对比 Rapfi（`1s`）与当前 `33d1515`（注：初次探针我们跑的是固定深度，Step 0 将统一到同 `1s` 口径复测）：

- **1T 差距核心在“搜索树有效分支因子（EBF）与战术延伸深度”，而非纯单核 NPS**：
  - `c4_ply20` 上 1T NPS 我们 `228K` vs Rapfi `287K`（仅差 20%），但 Rapfi 在 `279K` 节点打到 **Depth 17**，我们 `171K` 节点仅到 **Depth 12**（差 **5 层**）。
  - `loss4t_c9_ply22` 上 Rapfi 1 秒打到 **Depth 26（selDepth 44）直接看穿 `-M42`**，我们仅到 **Depth 12（1T）/ 14（4T）**，且 `DEFAULT_TIMED_SEARCH_MAX_DEPTH = 25` 本身就会在深度上去后截顶。
- **4T 相对 1T 额外落后 34.5 Elo**：
  - 4T 对 Rapfi 负局中，以连续 `>=4` 手 `<50ms` 被速杀收尾的比例从 1T 的 `40.0%` 飙升到 **`58.9%`**；顶满 `>=950ms` 的比例我们 `45.3%` vs Rapfi `36.8%`。

### 2.2 源码级 4 大差距

1. **4T 并行层 Bug 与共享争用**（`src/search/abdada.rs`、`src/search/root.rs`、`src/search/alphabeta.rs`）：
   - `pick_best_thread` 中 `cand.score.abs() >= WIN` 允许**浅层误报 `-WIN`（必败）的 helper 覆盖主线程深层可守着法**；主线程/helper 碰到 `|score| >= WIN` 直接退出或掐停全池。
   - Lazy SMP 内部仍残留 `use_abdada = true`（512 KiB `abdada_table` 跨深度原子写 + 两轮 deferral），造成 L3 cache-line bouncing。
   - 缺少 Rapfi 的多线程 `threadBias = 0.1 * ln(numThreads)` LMR 收紧；根节点（非受限时）仍允许 early TT cutoff；超时中断直接丢弃当前层已找到的更好着法。
2. **战术防守收窄与强制延伸不足**（`src/search/movegen.rs`、`src/search/alphabeta.rs`）：
   - `movegen.rs` 虽有 `hsflag / apply_hostile_three_extension`，但 `hsflag` 仅记单个 `Option<Move>`（多活三覆盖）、古典 `-5000` 阈值会漏进闲着，且**完全缺少 `DEFEND_B4F3`（防对手四三杀）**。
   - 挡冲四（`single_forcing`）当前扣 `0.5` 层（一攻一防耗 `1.5` 层），而 Rapfi 的 `OPPO5_EXT = 1.1`（一攻一防仅耗 `0.9` 层），长冲四链下视野天然浅一半。
3. **非强制节点 EBF 裁剪/延伸/排序差距**（对照 Rapfi `search/ab/search.cpp`、`parameter.h`、`movepick.cpp`）：
   - 缺少 Alpha-Improvement Reduction（`-1.05`）、Policy Pruning、对齐 `scoreBias` 的强 Policy LMR、LMR Extra Extension / Shallower Re-search、`TTM_EXT`、Double SE（`+2.0`）、PV 节点 LMP、以及 MovePicker 融合 History / Killer / CounterMove。
4. **计时最大深度天花板与潜在 NNUE 热点开销**：
   - `DEFAULT_TIMED_SEARCH_MAX_DEPTH = 25` 过低，必须改大（需同步核查 `SUBPLY_TT_DEPTH_CAP` / `TTEntry4` 12-bit 深度编码上限）。
   - NNUE 路径仍在运行古典 `EvalCaches` / `move_value` / ~900B `EvalSnapshot`，但其真实 CPU 占比需在 Step 0 实测确认后再决定优化力度。

---

## 3. 关键大坑与“防无用功”清单

为避免踩坑或陷入高成本低收益的工程深坑，明确以下 5 条红线：

1. **大坑 1 — `pick_best_thread` 的 `-WIN` 覆盖与早停漏洞**：
   - 必须先修，且区分“找到必胜（`>= WIN`）”与“误判必败（`<= -WIN`）”，绝不让浅层 `-WIN` 覆盖深层非必败着法；主线程判 `-WIN` 时也不应立刻扼杀仍在更高深度找解杀的 helper。
2. **大坑 2 — Rapfi Policy 参数尺度不兼容**：
   - Rapfi 的 `policyReduction` 与 `policyPruningScore` 输入是 `int(raw_logit * 32) + scoreBias + history + counterMove`，而非我们的裸 float logit。接入任何 Policy Pruning / Policy LMR / History 排序混合前，**必须先在 Step 0 核对 `Mix9svq` 的 `scoreBias` 及同局面 value/policy 数值一致性**。
3. **大坑 3 — 严禁盲目关闭 NNUE 同步 Root df-pn VCT**：
   - 先在 Step 0 统计已有 100 局里 root VCF/VCT 的真实触发率、直接胜局数与平均耗时；如果 VCT 净赚胜局，优先考虑“限预算”或“4T 下 1 核 VCT + 3 核并行 Lazy SMP”，绝不一刀切关闭。
4. **大坑 4 — `DEFAULT_TIMED_SEARCH_MAX_DEPTH` 调大时核对 TT 深度编码上限**：
   - 当前 `TTEntry4` 深度字段为 12 bit（`DEPTH_MAX = 4095`），`SUBPLY_TT_SCALE = 64`，`SUBPLY_TT_DEPTH_CAP = 3200`（`50.0` 层），`SUBPLY_TT_WIN_BONUS = 640`（`3200 + 640 + 3 = 3843 <= 4095`）。
   - 将 `DEFAULT_TIMED_SEARCH_MAX_DEPTH` 调大到 **`64`**（或 `99`）时：
     - 验证 `depth > 50` 时的降级行为（当前 `raw_probe_depth > max_encodable_depth` 会自动退化为仅用 TT move hint、不截断，是安全的）；
     - 同时检查是否应把 `SUBPLY_TT_WIN_BONUS` 从 `640` 收紧到 `256`（4 层）以把可精确编码深度提升到 `~59` 层，或评估 1/32-ply 精度（12 bit 可覆盖 `~120` 层），并补单元测试。
5. **大坑 5 — 拒绝拿 100 局噪声去逐个测 `5~10 Elo` 微调，并砍掉 3 项低 ROI 无用功**：
   - 100 局 1σ 噪声约 `±25 Elo`，中小协同改动打包成合理批次验收；
   - **明确砍掉**：16B 4-way TT 改 12B 5-way TT（收益仅 `+3~6 Elo`，破坏无锁 64-bit 原子校验，性价比极低；仅保留现有 4-way 并按需加 probe age 刷新）；
   - **明确暂缓**：未经 CPU profile 证明的全量重写 `EvalCaches` 为新 `Pattern4` 棋盘（Renju 递归禁手风险极高）；
   - **明确后置**：Root Aspiration Window 与复杂时间分配（`1s/move` 下 Rapfi 走的是简单 `ampleMatchTime` 路径；Aspiration 须等超时保 PV 稳固后再做）。

---

## 4. 分阶段推进计划

### Phase 0：前置三笔已复现 Bug 孤立修复 + 32 MiB TT 口径对齐（最优先）

> 原则：每笔独立提交、独立验收，绝不把调深度上限/禁根 TT 截断/调 LMR 混进这三笔。所有对 Rapfi 4T 测试统一显式传 `INFO tt_bits 20`（对齐 32 MiB TT，避免我方默认 64 MiB 造成口径失真）。

- [x] **Fix 1（P1）：4T 停搜与最终选步**（`src/search/root.rs`、`src/search/abdada.rs`）
  - 主线程不再因单次报 `-WIN` 立即叫停全池（赢杀确认 2 层 / 输杀确认 6 层且无同深 helper 分歧，才提前停）；helper 不再因 `±WIN` 提前退出闲置；
  - 重写 `pick_best_thread` 为严格可传递、100% 与 helper 提交顺序无关的三段选择规则（赢杀取最短 → 若无严格更深的输杀则仅在普通分上按深度加权投票 → 否则取最深输杀里的最长抵抗）。
  - 实测结果（均严格 `INFO tt_bits 20` = 32 MiB TT，Renju `1s/move`，`center_50_prefixes.jsonl` 100 局）：
    - `4T vs 33d1515 4T`：**`50-42-8`（`54.0%`，`+27.85 Elo`，`WW=5, WD=3, WL/DD=38, LD=3, LL=1`）**，`0` 错误/`0` 超时，`avg_ms=509.4ms`；
    - `4T vs Rapfi 4T (32 MiB)`：**`31-69-0`（`31.0%`，`-138.99 Elo`，`WL=31, LL=19`）**，`0` 错误/`0` 超时，`avg_ms=498.7ms` vs Rapfi `509.4ms`（相比修前 64 MiB 的 `26.0% / -181.7 Elo` 跃升 **`+42.7 Elo`**，并反超 1T 相对差距 `30.0% / -147.2 Elo`）。
- [x] **Fix 2（P1）：叶层 QVCF 局部裁剪与共享 TT 污染**（`src/search/alphabeta.rs`）
  - 采用 Sound-Bound Guard + `self_last` salted key（Candidate C+）：用 4 条位运算检测 `nonlocal_b4_dropped = (attack4_mask & !mask) != 0`；未丢弃非局部 B4 或已取得 `HASHF_BETA`/杀胜下界时照常读写纯盘面 `zobrist_key` 保持高命中率；丢弃非局部 B4 且 fail-low 时仅写入 `board.zobrist_key() ^ qvcf_self_last_salt(last_move)`（纯盘面键仅存 `HASHF_EMPTY` 静评），彻底杜绝共享 TT 污染且避免密集局炸节点。
  - 实测结果（均严格 `INFO tt_bits 20` = 32 MiB TT，Renju `1s/move`，`center_50_prefixes.jsonl` 100 局）：
    - `4T vs Fix 1 4T`：**`48-41-11`（`53.5%`，`+24.36 Elo`，`WW=5, WD=5, WL/DD=34, LD=4, LL=2`）**，`0` 错误/`0` 超时，`avg_ms=514.5ms`；
    - `4T vs Rapfi 4T (32 MiB)`：**`29-71-0`（`29.0%`，`-155.54 Elo`，`WL=29, LL=21`）**，`avg_ms=506.3ms` vs Rapfi `525.1ms`（其中 1 盘坚持到第 225 手全盘仅剩 `(11,8)` 为黑方禁手导致无合法着，属 Renju 正常判负）。
  - 回归单测：`qvcf_leaf_is_move_order_invariant_and_does_not_poison_tt`（复现 `renjunet_center_g151985_ply6` 第 28 手换序）+ `qvcf_self_last_salts_are_unique_and_nonzero` 通过。
- [x] **Fix 3（P2）：奇异延伸验证提前排除 `skip_move`**（`src/search/alphabeta.rs`）
  - 在排序、`wide` 截断、`policy_top_logit` 与 `index` 分配之前剔除 `options.skip_move`，避免占名额并避免首替代着误触发 LMR/LMP 偏移；
  - 实测结果（均严格 `INFO tt_bits 20` = 32 MiB TT，Renju `1s/move`，`center_50_prefixes.jsonl` 100 局）：
    - `4T vs Fix 2 4T`：**`46-48-6`（`49.0%`，`-6.95 Elo`，`WW=2, WD=2, WL/DD=40, LD=4, LL=2`）**，`0` 错误/`0` 超时（该正确性小修仅影响 ~0.7% 奇异判定，100 局处于 50% 噪声带内）；
    - `4T vs Rapfi 4T (32 MiB)`：**`30-66-4`（`32.0%`，`-130.94 Elo`，`WL=30, LD=4, LL=16`）**，`0` 错误/`0` 超时，`avg_ms=515.3ms` vs Rapfi `556.0ms`（**创 4T 对 Rapfi 新高**，较 Phase 0 前 64 MiB 的 `26.0% / -181.7 Elo` 净涨 **`+50.8 Elo`**，`LL` 双负对子从 23 降到 16）。
  - 回归单测：`singular_skip_move_is_excluded_before_width_truncation_and_indexing` 通过。
- [ ] **协议卫生（可选单笔）**：`src/protocol/gomocup.rs` 补 `INFO max_memory` 解析，自动映射为对齐字节数的 `tt_bits`。

### Step 0：前置体检（在 Phase 0 干净基线上补齐实测证据）

- [ ] **0.1 统一 `1s/move` + `32 MiB TT` 深度/NPS 对照**：对比我们与 Rapfi（1T & 4T）在 4~6 个典型局面（含 `vct_dense_ply78`、`c4_ply20`、`loss4t_c9_ply22`）的完成深度、节点数、NPS。
- [ ] **0.2 `vct_dense_ply78` 节点构成拆解**：统计主 AB 节点 vs 叶 QVCF 节点 vs `hostile_threat` 节点占比，确认 `vct_dense` 节点暴涨的真正主因。
- [ ] **0.3 CPU 热点 Profile**：在 1T 和 4T 下实测 `nnue.before_move/after_undo`、`nnue.evaluate_value`、`nnue.evaluate_policy_moves`、`value_wide_compute_for_rule`、`EvalSnapshot`、`movegen` 的真实时间占比。
- [ ] **0.4 Root VCF / VCT 实盘收益审计**：统计 100 局实盘中 root VCF / df-pn VCT 触发次数、直接拿下的胜局数、以及平均/长尾耗时。
- [ ] **0.5 Mix9svq 数值与 `scoreBias` 对齐核对**：核对 Rapfi `Mix9svq` 的 `scoreBias` 及同局面 value/policy 与我们的输出一致性。

---

### Step 1：4T Lazy SMP 净化（在 Fix 1~3 基础上单步验证）

- [ ] **1.1 清理 Lazy SMP 内部残留 `abdada_table` 争用**（移除 512 KiB 跨深度原子写与两轮 deferral，保留古典 ABDADA 路径不动）。
- [ ] **1.2 逐项验证 Lazy SMP 增强**：
  - 禁止 Root / PV 节点 early TT cutoff；
  - 加入 Rapfi `threadBias = 0.1 * ln(numThreads)` 多线程 LMR 收紧；
  - 超时打断时保留主线程/helper 当前深度已提升的最新 `best_move`；
  - 评估 4-way TT `probe` 命中刷新 `generation`。
- [ ] **1.3 防撞顶**：待实测深度接近 22+ 时再把 `DEFAULT_TIMED_SEARCH_MAX_DEPTH` 从 `25` 提升至 `64`（同步核对 `TTEntry4` 12-bit 深度编码上限与 `README.md`）。

---

### Step 2：攻克 5~14 层深度差（1T & 4T 核心棋力跃迁）

> 按 Step 0 实测证据微调子项优先级；每包独立提交、独立对局门禁。

- [ ] **2A 战术防守精确化 + `OPPO5_EXT` 重启**：
  - 修复 `movegen.rs` 中 `hostile_threat` 的多活三覆盖与漏进闲着问题（仅限 NNUE 路径或保证古典 `base` `12/12` 完全等价）；
  - 补齐 `DEFEND_B4F3`（对手冲四活三杀点的精确防点 + 己方反冲四）；
  - 在此基础上重测 `OPPO5_EXT`（挡冲四延伸 `+1.0 ~ +1.1`），先用 `loss4t_c9_ply22` / `vct_dense_ply78` 验证杀棋视野与节点数，再跑 1T & 4T 对局门禁。
- [ ] **2B 搜索树 EBF 裁剪/延伸包（高协同打包验收）**：
  - Alpha-Improvement Reduction（`2 < depth < 12` 且 `|value| < 2000` 时 `depth -= 1.05`）；
  - 基于 Step 0 核对后的真实 `scoreBias` 校准 Policy LMR 与浅层 Policy Pruning；
  - LMR Extra Extension & Shallower Re-search；
  - `TTM_EXT`（非 SE 的 TT 步微延伸）+ Double Singular Extension（`+2.0`）+ PV 节点安全 LMP + `cutNode` / `uselessDefend` LMR 细化。
- [ ] **2C MovePicker 排序融合**：
  - 在对齐 `ScoreScale=32` 尺度后，将 `MainHistory`（Attack/Quiet）+ `Killer` + `CounterMove` 融合进候选排序，验证对 Mix9svq Policy 排序的净增益。
- [ ] **2D 验收门禁**：
  - 1T vs 上一阶段 100 局（`>= 50%`）+ 1T vs Rapfi 1T 100 局；
  - 4T vs 上一阶段 4T 100 局（`>= 50%`）+ 4T vs Rapfi 4T 100 局；
  - 目标：Step 2 结束时 **1T & 4T 对 Rapfi 双双达到 `37% ~ 41%`**。

---

### Step 3：按 Step 0 热点实测做轻量减负与收尾

> 仅针对 Step 0 profiler 证明占比 `>= 15%` 的热点动手，不做无证据的大重写。

- [ ] **3.1 低风险 NNUE 热点微手术**（按 Step 0 数据勾选）：
  - NNUE 模式下 `movegen` 跳过将被 Policy 覆写的古典 `move_value` 浮点计算；
  - NNUE 模式下精简 `EvalSnapshot`（跳过古典专用 `empty_bucket_counts` 768B 拷贝）；
  - 按 Step 0.4 数据处理 root VCT（限预算或 4T 并发 overlap，不盲删）。
- [ ] **3.2 极简时间控制与 Root Aspiration Window（可选收尾）**：
  - 仅在超时保 PV 稳固后测试 `0.9s` 软停、`singularRoot` 早停与 NNUE Root Aspiration Window（严格 1T + 4T 双门禁）。
- [ ] **3.3 最终里程碑大样本验收**：
  - `1T vs Rapfi 1T` 100~200 局、`4T vs Rapfi 4T` 100~200 局；
  - `1T vs P2 (b69f98b)`、`4T vs P2 (b69f98b)` 锚点对照；
  - Freestyle smoke 验证 + `docs/perf-log.md` 完整记录。

---

## 5. 里程碑进度追踪

| 阶段 | 状态 | 1T vs Rapfi 1T | 4T vs Rapfi 4T | 关键产出 / 提交 |
|---|---|---:|---:|---|
| 基线 P2 (`b69f98b`) | 已归档 | 23.5% (-205.0) | 28.0% (-164.1) | `master` 锚点 |
| 基线 P3 (`33d1515`) | 已完成 | 30.0% (-147.2) | 26.0% (-181.7, 64 MiB) | Batch 1~3 + Lazy SMP 初版 |
| Phase 0 · Fix 1（4T 停搜与选步） | **已完成** | — | **31.0% (-139.0, 32 MiB)** | `4T vs 33d1515` `54.0% (+27.9)` |
| Phase 0 · Fix 2（QVCF 缓存） | **已完成** | — | **29.0% (-155.5, 32 MiB)** | `4T vs Fix 1` `53.5% (+24.4)` |
| Phase 0 · Fix 3（奇异延伸排除） | **已完成** | — | **32.0% (-130.9, 32 MiB)** | `4T vs Fix 2` `49.0%`；对 Rapfi 4T 新高（`+50.8 Elo` vs P3） |
| Step 0 前置体检 | 待开始 | — | — | 实测报告（1s 深度/节点/热点/VCT/Bias） |
| Step 1 4T 止血 + 最大深度放宽 | 待开始 | — | 目标 `31%~34%+` | `DEFAULT_TIMED_SEARCH_MAX_DEPTH` + Lazy SMP 修复 |
| Step 2 EBF 与战术深度主攻 | 待开始 | 目标 `37%~41%` | 目标 `37%~41%` | `DEFEND_FOUR/B4F3` + `OPPO5_EXT` + EBF 包 |
| Step 3 轻量减负与收尾 | 待开始 | 目标 **`~40%+`** | 目标 **`~40%+`** | 按热点数据精修 + 最终大样本验收 |
