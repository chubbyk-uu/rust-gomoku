# Performance & Playing-Strength Optimization Roadmap

> 本文档制定 `rust_gomoku` 面向高性能与高棋力的中长期演进路线图。设计结合了当前代码剖析、[docs/perf-log.md](perf-log.md) 的历史实证结论，以及现代顶尖 CPU 五子棋/连珠引擎 **Rapfi** 的核心架构思想。
>
> 适用基准：`master` @ `v0.1.16`（`7fafe74`），后续每个里程碑落地后均按 [AGENTS.md](../AGENTS.md) 在 [docs/perf-log.md](perf-log.md) 记录实测数据并更新本清单状态。

---

## 1. 总体原则与可行性边界

### 1.1 为什么不能只照搬 Rapfi 的上层剪枝

[docs/perf-log.md](perf-log.md) 已多次验证：在现有 92 桶线性评估 + 无动态排序 + 无树内战术安全网（`qsearch` / 树内 VCF）的前提下，直接叠加激进前向裁剪（浅层 RFP、Futility、Quiet LMR、Aspiration Window）几乎必然漏算反杀或拖慢长尾。

Rapfi 的高 NPS 与高棋力来自**四层强耦合设计**：
1. **统一增量位板底座**：全局仅维护一套 2-bit 线位板与增量 `Pattern4`，评估、着法生成、禁手判定、战术搜索零重建复用。
2. **高效率走法排序与置换表**：4-way 64B 对齐 TT + 预取 + 分阶段 `MovePicker`（TT move 零 movegen 直接搜）+ Killer/Countermove/History。
3. **深层战术与现代搜索**：深层 Zobrist-TT VCF、`df-pn` VCT、叶节点 `qsearch` 安全网、整数/定点数深度、对数 LMR 与自适应 `timectl`。
4. **高精度增量评估**：从多方向/攻防交互/分规则经典评估，演进至基于 `Pattern` 码本的增量 NNUE（Mixnet）+ Policy 排序。

因此本路线图坚持 **“先底座、再战术与排序、再搜索与时间管理、最后评估跃升”** 的递进顺序。

### 1.2 双轨治理（遵循 `AGENTS.md`）

- **轨道 A（`base` 行为等价纯提速）**：固定局面下 move / score / nodes / tactical trace 与当前 `master` 保持一致（或仅为已证明的正确性修复）；通过 `13/13` reference diff 后可直接合入 `master`。
- **轨道 B（实验分支棋力/性能突破）**：允许改变搜索行为与评估，必须在独立实验分支开发，保留可回退路径；合入或设为推荐配置前，必须以当前 `master` 为基线完成实盘对弈验证（得分率 $\ge 50\%$，0 错误、0 超时，且具备明确的 Elo 或 avg/p95/max 耗时收益）。

### 1.3 可行性实话实说

- **P0、P1、P2 及 P3 路线 1（增强经典评估 + Texel/SPSA 自动调参）**：全部为纯 Rust 工程 + 本机 CPU 可闭环验证，**100% 可在本仓库内独立完成**。
- **P3 路线 2（Rapfi 风格增量 NNUE / Mix6–Mix7）**：
  - 推理侧（Rust 增量累加器 + AVX2 / ARM64 NEON SIMD + Policy 排序 + 权重加载 + 经典评估回退）**100% 可独立实现**。
  - 训练侧需要模型权重与训练算力支持（GPU 自对弈/强引擎对局数据蒸馏，或合规引入轻量训练管线）。若暂无 GPU 集群，可先落 P3 路线 1，再逐步建设轻量蒸馏训练链路。

---

## 2. 阶段依赖关系总览

```mermaid
flowchart TD
    P0["P0: 零风险速赢 (轨道 A)"]
    M11["M1.1 统一 2-bit 线位板底座 (轨道 A)"]
    M12["M1.2 解锁 VCF 深度 + Zobrist TT + 证明集过滤 (轨道 B)"]
    M13["M1.3 自适应时间管理 timectl (轨道 B)"]
    M21["M2.1 4-way 64B TT + 1T subply/整数深度 (轨道 B)"]
    M22["M2.2 分阶段 MovePicker + Killer/Countermove/History (轨道 B)"]
    M23["M2.3 df-pn VCT + B4 免费延伸 + 触发器修复 (轨道 B)"]
    M24["M2.4 轻量 qsearch + 现代 LMR/LMP/Singular (轨道 B)"]
    M25["M2.5 多线程低屏障/零克隆并行升级 (轨道 B)"]
    M31["M3.1 增强经典评估 + Texel/SPSA 调参 (轨道 B)"]
    M32["M3.2 增量 NNUE (Mix6/Mix7 + AVX2/NEON) + Policy 排序 (轨道 B)"]

    P0 --> M11
    P0 --> M12
    P0 --> M13
    M11 --> M21
    M11 --> M22
    M12 --> M23
    M11 --> M24
    M21 --> M22
    M22 --> M24
    M21 --> M25
    M24 --> M25
    M11 --> M31
    M24 --> M31
    M11 --> M32
    M22 --> M32
    M31 -.可选渐进.-> M32
```

---

## 3. 统一验证与止损矩阵

除明确豁免的纯文档提交外，每个里程碑必须跑完对应门禁：

| 检查项 | 轨道 A (`base`) | 轨道 B (实验分支) | 命令 / 基准集 |
|---|---|---|---|
| 格式与编译 | 必须 | 必须 | `cargo fmt --check && git diff --check && cargo build --release --bins` |
| 单元与桥接测试 | 必须 | 必须 | `cargo test` + `android/rust_bridge` 测试 + Renju oracle 回归 |
| Reference Diff | **13/13 必过且节点一致** | 核对无意外规则退化 | `python3 scripts/run_diff.py --profile all --check-baselines --jobs 8` |
| 固定慢手 & 战术基准 | 必测 A/B | 必测 A/B | 5 慢手 (`c43_p32`, `c25_p68`, `c21_p145`, `c34_p154`, `c34_p83`) + `vct_dense_ply78` / `vct_counter_win_scan_ply75` / `vct_slow_probe_case` + `renju_search_bench` |
| 实盘对局验证 | 可选抽测 | **必测（vs 当前 `master`）** | `cases/renju/center_50_prefixes.jsonl` 100 局 Renju（d8/w40 与 1s/3s 限时）；推荐配置前扩至 200 局 + Freestyle 100 局 + SlowRenju/Rapfi 外部参照 |
| 合并门槛 | 净加速 > 噪声且行为 100% 等价 | 得分率 $\ge 50\%$、0 错误、0 超时、avg/p95/max 或 Elo 显著正向 | 所有数据回填 `docs/perf-log.md` |

---

## 4. 分阶段详细计划

### 阶段 P0：零风险速赢（轨道 A · 行为等价）

目标：在不改动任何搜索语义的前提下，立刻清理已定位的内存/原子/克隆浪费，把 `master` 基线再提速 `10%–20%`。

#### M0.1 瘦身 `EvalSnapshot` 与移除 release 死字段
- **现状问题**：`src/eval/caches.rs` 的 `EvalSnapshot` 每次 snapshot/restore 拷贝约 `1.16 KiB × 2`，其中 `occupied_moves: [Move; 225]`（`450 B`，占 39%）在 release 构建下从未被读取（仅用于 `#[cfg(debug_assertions)]` 的 `evaluate_dgn_from_occupied`）。
- **改造内容**：
  1. 将 `occupied_moves` / `occupied_len` 仅保留在 `#[cfg(debug_assertions)]` 下，或直接删除（`dgn_sum` 已增量维护）。
  2. 评估将 `board_shadow` (225B) 与 `empty_bucket_counts` (368B) 改为对称 `make/unmake`（或紧凑定长增量 delta），消灭大块 `memcpy`。
- **净收益假设**：每节点减少 `900B–2.1KiB` 内存拷贝与维护分支，新增成本近乎为零。
- **止损条件**：5 慢手总耗时净加速 `< 3%` 时只保留去死字段部分，不引入复杂 delta 日志。

#### M0.2 修复 TT `probe()` 瑕疵、消除每节点 `Acquire` load 与增加 Prefetch
- **现状问题**：
  - `src/search/tt.rs` 每次 `probe`/`store` 都在 `buckets_for_mode()` 执行一次 `primary_mode.load(Ordering::Acquire)`。
  - `probe()` 命中 slot 0 且仅收窄窗口（`has_window: true`）时直接 return，跳过检查 slot 1；slot 1 若同 key 且 `best_move == None` 会覆盖 slot 0 的有效 `Some(mv)`。
  - 全仓无 TT 桶软件预取（prefetch）。
- **改造内容**：
  1. 在 `AlphaBetaSearcher` 单次搜索入口一次性获取当前模式的 `&[TTBucket]` 切片引用，消除热路径 `Acquire` 原子读。
  2. 在 `make` 落子计算完子局面 `zobrist_key` 后，对目标 TT 桶发起 `prefetch`（x86 `_mm_prefetch` / ARM `prfm` 兼容封装）。
  3. 单独验证修复 `probe()` slot 1 跳过与 `fallback_best_move` 覆写（注意先检查是否改变 13/13 diff；若改变节点数，移入 M2.1 实验分支）。
- **净收益假设**：减少原子屏障与 L3/DRAM 缓存缺失停顿；无额外内存占用。
- **止损条件**：若 prefetch 在 32 MiB/64 MiB 实测未带来正向增益（`< 1%`），则不保留无用汇编封装。

#### M0.3 共享只读 `Arc<EngineConfig>` 消除多线程每层克隆
- **现状问题**：`src/search/root.rs` 与 `src/search/abdada.rs` 在每个 `depth >= 5` 迭代层中，主线程 + 每个 worker 反复 clone 整个 `EngineConfig`（含 4 个 `Vec<f64>` 堆分配）与完整 `EvalCaches`。
- **改造内容**：将 `EngineConfig` 内的只读表改为 `Arc`，持久化 worker 本地 `Board`/`EvalCaches` 复用缓冲区，仅在开局或局面变更时同步差量。
- **止损条件**：`4T`/`8T` 中盘及浅层 d5–d8 启动开销应可测下降，且 `1T` 零回归。

---

### 阶段 P1：底层统一、深层战术与时间管理（轨道 A + B）

目标：打通 `Pattern4` 统一底座，补齐当前最明显的战术短板（VCF 暗截断、暴力防 VCF 过滤）和实战短板（原始时间管理）。

#### M1.1 统一 88 线 2-bit 位板底座（`Eval` / `Threat` / `Forbidden`）【轨道 A 优先】
- **现状问题**：
  - `src/eval/local.rs` 仍是 Alpha-Beta 第一热点（`~33.5%` CPU），每步落子现场扫 60 格拼 `PackedLine`。
  - `src/threats/threat_board.rs` 仍并行维护旧 `x1..x4: Vec<Vec<i32>>` 和部分旧线扫描。
  - `src/rules/forbidden.rs` 每次判定拷贝 225B `grid` + 重建 4 条 `DirectionalLine`。
- **Rapfi 对标**：全引擎常驻一套 2-bit 线位板，落子仅翻 4 个 bit-pair，所有模块共享增量状态。
- **改造内容**：
  1. 在 `EvalCaches`（或统一 `BoardState`）常驻 88 条 2-bit `u64` 线位板，替代 `PackedLine::from_board` 现场扫盘，保持 `SHAPE_TABLE` 输出 100% 位等价。
  2. 彻底清理 `ThreatBoardView` 中遗留的 `x1..x4` 堆数组与旧线扫描，全部收口到 `Pattern4State`。
  3. 改造 `rules/forbidden.rs` 直接在 2-bit 线位板上做四/活三判定（先保留原实现作随机 oracle 对拍，2000+ 局 0 差异后切换）。
- **预期收益**：单线程 NPS 再提 **20%–35%**，Renju 单节点开销显著逼近 Freestyle，同时消除进 VCF/VCT 时的重建开销。
- **止损条件**：等价模式下 13/13 diff 必须全过，5 慢手总耗时净加速需 $\ge 10\%$。

#### M1.2 解锁 VCF 深度上限 + 定长 Zobrist TT + 证明集过滤【轨道 B】
- **现状问题**：
  - `src/threats/vcf.rs` 的 `VCFM = 5` 把配置的 `root_vcf_depth = 8` 暗中截成 **5 步**、`opponent_vcf_depth = 7` 截成 **4 步**。
  - VCF memo 使用 `HashMap<(Side, Vec<Move>, Vec<Move>), ...>` 每次分配+排序。
  - `root.rs::apply_opponent_vcf_filter` 对全盘最多 225 格逐一克隆 `Board` 并重建 `Pattern4State` 跑完整 VCF。
- **改造内容**：
  1. 用基于 Zobrist `u64` 的定长轻量 VCF TT（如 64K–256K 槽 + generation 清空）替代 `HashMap<Vec>`。
  2. 移除 `VCFM = 5` 硬截断，支持根节点 VCF 搜到 **14–20 ply**（配安全节点上限，例如 50K–200K 节点，防止极端无解局耗时失控），并确保 `opponent_vcf_depth` 真实生效。
  3. `apply_opponent_vcf_filter` 改为收集对手 VCF 证明树的相关格集合（攻击点 + 冲四防点 + 关联阻断点 + 己方反冲四/成五点），仅对该子集复用单个 `ThreatBoardView` 做 `play/undo` 试探。
- **预期收益**：6–20 步实战长冲四杀从完全盲区变为毫秒级必杀；对手 VCF 过滤提速 **10x+**。
- **止损条件**：100 局 Renju/Freestyle 对局零误判、得分率 $\ge 50\%$，且 VCF 耗时不拖累 avg/p95。

#### M1.3 现代自适应时间管理 `timectl`【轨道 B】
- **现状问题**：`src/protocol/gomocup.rs` 直接取 `min(timeout_turn, time_left)`，在仅下发局时（`timeout_match` / `time_left`）时会把整盘剩余时间全砸在一步；迭代加深无软/硬时限分离、无稳定性早停，经常开启注定超时的新一层并整层丢弃结果。
- **Rapfi 对标**：`timectl` 根据剩余局时与当前步数动态计算 `optimum_time`（软）和 `maximum_time`（硬），结合最佳走法稳定性、分数骤降（fail-low 延长）、唯一迫着秒下、EBF 预测避免启动无效末层。
- **改造内容**：
  1. 实现 `TimeManager`：支持 `turn_time` + `match_time` 综合分配（如按局势阶段估算剩余步数 `expected_moves_left`，留足安全余量 `move_overhead`）。
  2. 引入软/硬双限：每层结束检查 `elapsed >= optimum_time` 或 EBF 预测下一层超时即提前退出；出现 `best_move` 切换或大幅掉分时动态放宽至 `maximum_time`；唯一着/必胜着直接早停。
  3. 允许在严格安全条件下保留已验证更优的末层结果（仅当至少完成首选着且发现经完整窗口验证的严格改善，避免半成品污染）。
  4. 限时模式下根据局面战术密度动态分配 VCT 子预算，替代写死 30%。
- **预期收益**：Gomocup 局时赛与 1s–5s 快棋 **+50~120 Elo**，且不发生超时判负。
- **止损条件**：限时 100 局对弈超时率必须为 `0%`，相比旧限时策略需显著净胜。

---

### 阶段 P2：搜索算法、VCT 与并行架构现代化（轨道 B）

目标：彻底解决固定深度长尾爆炸、排序质量差、单活三 VCT 盲区和水平线漏杀，同等时间多看 **2–4 ply**。

#### M2.1 4-way 64B 对齐 TT + 1T `subply` / 整数深度 + 停写叶节点
- **现状问题**：
  - 1T 模式下 `floor` 写 / `ceil` 查导致小数深度同层 TT 截断大面积失效。
  - 2-way 32B 桶未对齐 64B cache line（多线程相邻桶伪共享），无同 key 去重，`generation` 未参与替换，纯叶节点全部写入 TT 冲刷 slot 1。
  - `src/zobrist.rs` 写死 `turn: 0` 且依赖平台 libc `rand()`。
- **改造内容**：
  1. 将 `Zobrist` 切换为跨平台确定性 PRNG（如 SplitMix64）并支持 `side_to_move` XOR（保留 `base` 兼容模式开关或随实验分支一并切换）。
  2. 升级 TT 为 **64B 对齐 4-way 桶**（4 × 16B 或紧凑 12B/16B 槽），实现 **同 key 覆盖 + 世代老化 + 深度优先**替换策略。
  3. 停止将纯静态叶节点（`depth <= 0`）写入 TT。
  4. 评估并统一 1T 与 MT 的深度表示（短期 1T 启用 1/64-ply `subply_tt`；随 M2.4 彻底转向整数/定点深度）。
- **预期收益**：1T 长尾节点数预计下降 **20%–50%**，多线程 TT 缓存行争用显著缓解。

#### M2.2 分阶段 `MovePicker` + Killer / Countermove / History
- **现状问题**：
  - 当前每次进节点哪怕 `tt_best_move` 首位就能截断，也要全量扫 225 格覆盖区、算完所有空位分值并堆分配 `Vec<Candidate>`（占中盘 CPU ~21%）。
  - `hsflag` 仅保留单个对手活三，多活三漏防；`single_forcing` 存在多个迫着时粗暴坍缩为 1 点。
  - 无动态排序启发，`fast_trunc` 造成大量平局团。
- **Rapfi 对标**：Stockfish 式栈上分阶段 `MovePicker`（`TT_MOVE -> FORCING_THREATS -> QUIET_ORDERED`）。
- **改造内容**：
  1. **Stage 0 (`TT_MOVE`)**：只做 `O(1)` 合法性校验，直接搜 TT 走法；截断则彻底跳过 movegen 与排序。
  2. **Stage 1 (`FORCING` / `TACTICAL`)**：直接从 `Pattern4` 位板提取成五、活四、冲四、对手活三全部防点（修复多活三漏防和 `single_forcing` 误坍缩）。
  3. **Stage 2 (`QUIET`)**：在栈数组（`[ScoredMove; 225]`，零堆分配）生成其余覆盖候选，融合静态攻防分 + **2-slot Killer + Countermove + Butterfly/Continuation History**，使用懒局部选择排序（每次只挑下一个最高分，不排尾部），移除 `fast_trunc` 精度截断。
- **注意历史教训**：严禁重蹈旧 `fast_history_ordering` 覆辙（它在全节点 eager 计算 `getmi` + 全量排序导致慢 29.5%）；新排序必须严格遵循“TT 先截断 + 懒局部选排 + 零 `getmi` 依赖”。
- **预期收益**：减少 40%–60% 内部节点 movegen，大幅提高首着截断率与降低 PVS 重搜率。

#### M2.3 VCT 升级为 `df-pn` + B4 免费延伸 + 修复单活三触发盲区
- **现状问题**：
  - 朴素 IDDFS VCT（`root_vct_depth = 6`）对宽分支爆炸、对窄长分支看不深；B4 单防点与 A3 同等扣深，叶节点不接 VCF。
  - `has_vct_trigger_for_rule` 在无冲四且不能一步双活三时（含 Renju 黑方禁 3-3）直接跳过 VCT，漏算单活三起手杀。
- **改造内容**（对应 `perf-log.md` Route C）：
  1. 先做增量增强：B4 单防点不耗（或低耗）A3 深度、VCT 叶节点接快速 VCF、扩展触发器覆盖单活三可续攻局面（配节点上限）。
  2. 核心升级：实现基于 `Pattern4State` + 固定大小 Zobrist TT 的 **`df-pn`（Proof/Disproof Number Search）** VCT 求解器，支持节点/时间预算无缝退出，并完整保留 Renju A3 静点反驳与黑方禁手验证（0 误判回归）。
  3. 解决不限时模式 VCT 无上限风险：评估合理的高节点上限（如 500K–2M 节点）确保既能算透 12–20 步杀棋又不出现极端卡死。
- **预期收益**：中后盘算杀深度从 6 步跃升至 12–20 步（预计 **+50~100+ Elo**），同时大幅压缩高密度残局 VCT 长尾。

#### M2.4 轻量 `qsearch` + 现代搜索框架（对数 LMR / LMP / IIR / Singular）
- **现状问题**：
  - `depth <= 0` 无 `qsearch`，靠 `global_eval.rs` 里低效的 `LAST5`/`NEXT43` 补丁，导致水平线漏杀，也使 RFP/Futility/LMR 无法安全开启。
  - `running_downf` 进位 LMR 存在 $O(k^2)$ 粗暴减层、余数传子节点破坏 PV、漏杀不重搜等结构缺陷。
  - `extend_ratio = 0.9` 无 `ply` 上限 + `wide = 40` 全树不衰减制造长尾。
- **改造内容**（**严格逐项消融，每开一项必须跑 100 局实盘对弈验证**）：
  1. **轻量 `qsearch`**：基于 `Pattern4` 位板在 `depth <= 0` 仅搜索迫着（成五/活四/冲四及必防，限深 6–10 ply），替换 `LAST5`/`NEXT43` 全盘扫描。
  2. **转向整数/定点深度 + 对数 LMR 表**：替代 `running_downf`；`R = lmr_table[depth][move_idx]`，按 `is_pv`、`improving`、`is_forcing`、`history` 调整，绝不跨层传余数，fail-high 始终全深重搜。
  3. **有上限的迫着延伸 + LMP**：用「单防/威胁延伸 + `ply` 上限」替代无上限 `extend_ratio = 0.9`；浅层安静着引入 LMP 替代全树固定 `wide = 40`。
  4. **按需验证 IIR / RFP / Futility / Singular Extension**：只在有 `qsearch` 和校准评估后逐项测试，不达标即回退。
- **预期收益**：消除系统性漏杀与 15s–70s 极端长尾，同等时间内有效搜索深度提升 **+2 ~ +4 ply**。

#### M2.5 多线程并行升级（去强屏障 + 多样化探索 + VCT 并行）
- **现状问题**：每层主线程阻塞等全部 worker、PV 首着前全员重复搜 #0、8T 扩展率停留在 `~2.2x–2.4x`、`overlap_vct_alphabeta` 同步拷贝整张 TT 且新建线程池。
- **改造内容**：
  1. 在 M2.1 + M2.2 + M2.4 基础上，试验 **异步迭代 / 错峰深度 + 轻量 ABDADA 避碰**，去除每层强制等待屏障，并允许 worker 在 PV 尚未稳定时探索不同候选/深度。
  2. 若 `df-pn` VCT 落地，支持共享 VCT TT 的多线程并行算杀，并让 `overlap_vct_alphabeta` 复用现有线程池、彻底移除整张 64 MiB TT 拷贝。
- **预期收益**：4T 进一步提速，8T 扩展率向 **4x–5x+** 逼近。

---

### 阶段 P3：评估函数代际跃升（打破棋力天花板 · 轨道 B）

> 搜索和战术优化到位后，棋力的最终决定因素是评估精度。提供两条递进路线，可先落 M3.1，再冲刺 M3.2。

#### M3.1 路线 1：增强经典评估（4 方向 + 攻防交叉 + 分规则）+ Texel/SPSA 自动调参
- **现状问题**：`default_eval_para.txt` 仅有 92 个双方向线性桶（丢弃第 3/4 方向）、零攻防交叉项、无阶段/位置/连接特征、Freestyle 与 Renju 共用一套 `f64` 权重。
- **改造内容**：
  1. 评估全面改为 `i32` 定点整数运算，消除热路径 `f64` 转换。
  2. 复用统一 `Pattern4` 扩展特征：完整 4 方向组合、同点攻防交互表（`self_pat × opp_pat`）、开中残局阶段插值（Tapered Eval）、中心/连通/先手 Tempo 项。
  3. 为 **Freestyle** 与 **Renju** 分别建立独立参数表。
  4. 在 `tools/` 下编写自对弈数据采集 + **Texel Tuning / SPSA** 参数优化器（纯 CPU 即可运行）。
- **预期收益**：评估更快且质量显著跃升，保守 **+100~200 Elo**。

#### M3.2 路线 2（终极目标）：Rapfi 风格增量 NNUE（Mix6 / Mix7 + AVX2 & ARM64 NEON）+ Policy 排序
- **Rapfi 对标（arXiv:2503.13178）**：
  - 输入为 4 方向长度 11 的 1D 棋型码本（直接复用本项目 `Pattern4` 的 88 条 2-bit 线位板）。
  - 轻量 **Mixnet**（`Mix6` / `Mix7` 级，INT8/INT16 量化权重仅 **0.5 MB – 3 MB**，适合内嵌桌面端与 Android APK/AAB）：码本映射 → 局部 3×3 深度可分离卷积 / 通道混合 → 全局累加器 Value Head + 局部 Policy Head。
  - `make/unmake` 时仅增量更新落子 4 轴 ±5 格影响区域的特征累加器。
- **改造内容**：
  1. 设计抽象 `Evaluator` trait（`ClassicalEval` 与 `NnueEval` 可热切换、支持无缝回退）。
  2. 实现增量 NNUE 累加器与 SIMD 推理后端：
     - x86_64：`AVX2`（可选 `AVX-VNNI`），带运行时特性检测与标量回退。
     - Android / ARM64：`NEON`（可选 `dotprod`），兼顾移动端低功耗与高 NPS。
  3. 将 NNUE 输出的轻量 **Policy 分数**接入 M2.2 的 `MovePicker`，取代静态启发式排序。
  4. 配套外部 PyTorch 训练/量化导出脚本（支持自对弈或强引擎对局蒸馏，Freestyle / Renju 各自导出紧凑二进制权重文件）。
- **预期收益**：跨代棋力飞跃（保守 **+400~800+ Elo**），同时 Policy 排序大幅压缩 Alpha-Beta 分支因子，使现代前向裁剪收益最大化。

---

## 5. 已知禁区与防回退清单（来自 `docs/perf-log.md`）

以下方向已被历史实验证伪或证明存在严格前提，**除非前提发生根本改变，否则不得重复投入**：

1. **盲目扩大默认 TT 到 128 MiB**：L3/TLB 抖动导致正常中盘与部分长尾变慢（`+6% ~ +28%`）；默认保持 1–3T `32 MiB` / 4+T `64 MiB`，扩大前必须证明 4-way/对齐后缓存局部性已改善。
2. **在无 `qsearch` 与旧线性评估下直接加 RFP / Futility / Quiet LMR**：已实测掉棋力（`47.5%`, `-17.4 Elo`）甚至反拖慢 max 耗时（`36s -> 82s`）。必须放在 **M2.4（`qsearch` + 新排序就绪后）** 再逐项消融验证。
3. **直接截断 `extend_ratio` 而不补强战术安全网**：曾使 100 局胜率暴跌至 `38.5%`（`-81.4 Elo`）。
4. **破坏懒排序的 Eager History 排序**：旧 `fast_history_ordering` 因每节点全量 `getmi` + 全排序变慢 `+29.5%`；新 History 必须依附分阶段 `MovePicker`。
5. **非重点微优化**：`recompute_all` 复用、VCF memo key 微调（非热点）、`ThreatBoardView` 在 `play()` 上做全量额外 bitset 维护、旧式 root YBWC / root full-window split / 无保护 Aspiration Window。
6. **在 `1T` 与 `MT` 共享 TT 或 `subply` 量化上破坏 soundness**：严守 `store = floor`、`probe = ceil`、超上限深搜退化为仅 move-hint、1T/MT 模式隔离等全部回归测试。

---

## 6. 执行追踪清单

- [x] **P0 — 零风险速赢 (`base`)**
  - [x] `M0.1` 瘦身 `EvalSnapshot`（去除 release 死字段 `occupied_moves`，`board_shadow` 改增量日志，紧凑化 undo log）
  - [x] `M0.2` 消除 TT 热路径 `Acquire` load（`TTTableView`）+ 增加 x86_64 TT `prefetch`（`probe()` 语义修正留入 `M2.1` 以守住 13/13 严格等价）
  - [x] `M0.3` `Arc<EngineConfig>` 与持久化 `WorkerLocal` 复用 `EvalCaches` / `AlphaBetaSearcher`，消除多线程每层克隆
- [ ] **P1 — 底层统一、战术与时间管理**
  - [ ] `M1.1` 88 线 2-bit 位板统一 `Eval` / `Threat` / `Forbidden`（`base` 等价）
  - [ ] `M1.2` 解锁 VCF `VCFM=5` 截断 + 定长 Zobrist TT + 证明集对手 VCF 过滤（实验分支）
  - [ ] `M1.3` 自适应时间管理 `timectl`（局时切分、软/硬时限、稳定性早停、动态 VCT 配额）（实验分支）
- [ ] **P2 — 搜索与并行现代化（实验分支）**
  - [ ] `M2.1` 64B 对齐 4-way TT + `generation` 替换 + 停写叶节点 + 1T `subply` / 整数深度 + 确定性 Zobrist
  - [ ] `M2.2` 栈上分阶段 `MovePicker`（TT 零 movegen）+ Killer / Countermove / History + 修复多活三/`single_forcing`
  - [ ] `M2.3` `df-pn` VCT + B4 免费延伸 + 叶接 VCF + 单活三触发修复
  - [ ] `M2.4` 轻量 `qsearch` + 对数 LMR / LMP / IIR / 有上限延伸（逐项消融验证）
  - [ ] `M2.5` 多线程异步错峰并行 + 无整表拷贝 `overlap_vct_alphabeta` / 并行 `df-pn`
- [ ] **P3 — 评估代际升级（实验分支）**
  - [ ] `M3.1` 4 方向 + 攻防交叉 + 分规则 `i32` 经典评估 + Texel/SPSA 调参
  - [ ] `M3.2` Rapfi 风格增量 NNUE（Mix6/Mix7 + AVX2/NEON SIMD）+ Policy 排序
