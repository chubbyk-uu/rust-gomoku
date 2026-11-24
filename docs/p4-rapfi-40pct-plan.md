# P4 计划：1T / 4T 对同权重 Rapfi 逼近 40% 得分率

> - 创建时间：2026-11-24（基准与记录修订：2026-11-24 22:45）
> - 当前工作分支：`exp/p3-search-strength`
> - **当前唯一锁定基准（`p4_base`，commit `4065c80`）**：搜索内核严格等价 `p4_fix1 (45f46bb)` + 协议层保留 `INFO max_memory (812a993)`；基准二进制归档为 `opponent/baselines/gomocup_nnue_p4_base`（SHA-256 `ca62b0c30f7fdf626d76b30002ab8480b6ced3d1a8f890a287c248645c307ec9`）。
> - 锚点主线基线：`master`（`b69f98b`，P2 基线，作为常驻测试对手保留）

---

## 1. 目标与评测口径

### 1.1 终极目标

在**同神经网络权重、同 TT 容量、同计时、同硬件公平并发**条件下，将 `rust_gomoku` NNUE 搜索棋力推升至：

- **1T vs Rapfi 1T（Renju `1s/move`，`Mix9svq`，32 MiB TT，`database=off`，`--reuse-engine-state`，`--jobs 8~10`）**：得分率逼近 **`40%`**（约 `-70 Elo`，相对 `p4_base` 的 `27.5%` 净增约 **`+98 Elo`**）。
- **4T vs Rapfi 4T（Renju `1s/move`，`Mix9svq`，32 MiB TT，`database=off`，`--reuse-engine-state`，`--jobs 4`）**：得分率逼近 **`40%`**（约 `-70 Elo`，相对 `p4_base` 的 `31.0%` 净增约 **`+69 Elo`**）。

### 1.2 硬性约束与实验纪律

1. **古典 `base`（`use_nnue = false`）100% 行为等价**：`python3 scripts/run_diff.py --check-baselines --jobs 8` 保持全通过（当前 12/12 或完整 13/13 差分）。
2. **规则与稳定性零退步**：Renju / Freestyle 0 非法着、0 错误、0 进程外脚本超时（注：对局脚本外部超时上限为 `120s`，引擎内部单手时限为 `1000ms`，因管道收尾会有 ~1~2ms 外部壁钟波动；第 225 手全盘仅剩 1 个黑方禁手点导致无合法着属 Renju 正常判负）；`cargo test --locked` 与 `android/rust_bridge` 全部通过。
3. **默认参数集中管理**：涉及 `src/config.rs` 默认参数修改同步更新 `README.md` / `README-cn.md`。
4. **严格公平评测口径**（本机 `i7-13700F`：8 P-core / 16 线程 + 8 E-core / 8 线程）：
   - 必须开启 `--reuse-engine-state`，并统一传 `timeout_turn=1000`、`time_left=300000`、`rule=4`、`32 MiB TT`（`tt_bits=20`；注：`INFO max_memory` 当前仅约束自动 TT 容量，不等同于进程总 RSS 上限）；
   - 严禁将 `--jobs 16`（超配挤占 E-core）与 `--jobs 8/10` 混比；统一 **1T 用 `--jobs 8`（内战）或 `--jobs 10`（对 Rapfi）**、**4T 用 `--jobs 4`**，且不得并发跑两组大型对局。
5. **单点验收与双端把关**：
   - **每次只改一个原子点**，绝不再打包多个未验证的搜索改动；
   - **固定深度无时限门禁**：**严禁发送 `INFO timeout_turn 0`**（[gomocup.rs](file:///home/jerry/rust_ws/rust-gomoku/src/protocol/gomocup.rs#L341) 会将其解析为 **`200ms` 限时**）。统一使用 `case_probe --case <棋谱> --rule renju --depth 8 --width 40 --threads 1 --tt-bits 20 --nnue-dir <权重目录>`，或直接启动归档 `gomocup_engine --depth 8 --width 40` 并省略所有时钟 `INFO`；
   - **搜索/启发改动**必须同时通过 **1T vs `p4_base`（100 局）** 与 **4T vs `p4_base`（100 局）** 双端检验，报告配对得分率、95% 区间、固定局面节点变化以及 avg/p95 耗时。
6. **分支纪律**：全部实验在 `exp/p3-search-strength`（或后续实验分支）验证，未经用户明确批准不合入 `master`、不推送 `origin`。

---

## 2. Phase 0 全面审计与 `p4_base` 基准锁定（2026-11-24）

### 2.1 五个候选版本的代码实质关系

经完整核对 `git diff 122aa89..812a993`，从 `p3_batch3` 到 `p4_fix3` 的真实代码演进关系如下：

1. **`p3_batch3 (122aa89)`**：P3 Batch 3（Singular Extension + Multi-Cut + NegExt）1T 核 + 旧同步每层 ABDADA（`DepthJob`）4T。
2. **`33d1515 (p3_batch3_4tfix)`**：**1T 代码与 `p3_batch3` 100% 完全一致**；4T 切到异步 Lazy SMP（`LazySmpJob`）+ 4T SE 去重 + 4T worker 每步清空 `quiet_history` + 奇数 worker LMR `+0.25`。
3. **`p4_fix1 (45f46bb)`（已锁定为 `p4_base` 搜索内核）**：
   - **1T 代码仍与 `p3_batch3` 100% 完全一致**（`sync_worker_state` 与 `abdada` 分支在 `threads == 1` 下均不触发）；
   - **4T 修复**：修复 `pick_best_thread` 为可传递且与 helper 返回顺序无关的选择逻辑，并加入主线程赢/输杀连续确认机制（注：审计已确认当前确认仍会被 `+10` 层缓存杀分空转穿透，列入下方 P1 专项修复）。
4. **`p4_fix2 (dc23d00)`（暂撤出基准，待不加盐单点重修）**：叶层 QVCF 增加 `nonlocal_b4_dropped` 保护，并用 `qvcf_self_last_salt(last_move)` 给 fail-low 写表加盐。直接内战 `p4_fix2_4t vs p4_fix1_4t` 为 `53.5% [47.37%, 59.63%]`，对 `33d1515_4t` 为 `49.0%`、对 Rapfi 4T 为 `29.0%`（100 局区间内未单独证实 4T 退化；保守撤回后改为无盐写表修复再做 1T/4T 消融）。
5. **`p4_fix3 (f9e92b0)`（暂撤出基准，待拆成两项独立开关验证）**：Singular Extension 提前排掉 `options.skip_move` 后再排序、截断宽度并从 `index = 0` 编号（注：当前非根 LMR 深度门槛为 `depth >= 2.0`，见 [alphabeta.rs](file:///home/jerry/rust_ws/rust-gomoku/src/search/alphabeta.rs#L1495)；本地 Rapfi [search.cpp](file:///home/jerry/rust_ws/rust-gomoku/opponent/rapfi/Rapfi/search/ab/search.cpp#L1437-L1701) 同样在跳过 `skipMove` 后才递增 `moveCount`，首替代着法同样免 LMR。此前将 `p4_fix3` 4T 波动直接归因于“第 1 个替代着免 LMR”属未验证假设，后续拆成“提前排除占位”与“是否给 LMR 索引 `+1`”两个开关实测）。
6. **`812a993`（已保留进 `p4_base`）**：纯协议层支持 `INFO max_memory <bytes>` 约束自动 TT 上限（搜索逻辑零改动；注：不限制进程总 RSS，且冷启动限时回落古典路径仍未受 `max_memory` 约束，列为 P2 协议待修项）。

### 2.2 全量实测对局矩阵（`center_50_prefixes.jsonl`，Renju `1s/move`，100 局，`--reuse-engine-state`）

#### A. 对同权重 Rapfi（`Mix9svq`，32 MiB TT，`database=off`）

| 对阵 | 并发 / TT | 成绩 (W-L-D) | 得分率 (95% CI) | Elo 差 | Pentanomial (WW/WD/WL/LD/LL) | 均耗 (ms) | 备注 |
|---|---|---:|---:|---:|---|---:|---|
| `master (b69f98b, P2)` 1T vs Rapfi 1T | — / 32M | 23-76-1 | 23.5% | -205.0 | — | — | P2 历史锚点 |
| `master (b69f98b, P2)` 4T vs Rapfi 4T | — / 32M | 28-72-0 | 28.0% | -164.1 | — | — | P2 历史锚点 |
| `p3_batch3 (122aa89)` 1T vs Rapfi 1T | `j=16` / 32M | 30-70-0 | 30.0% `[22.94%, 37.06%]` | -147.2 | `0/0/30/0/20` | 524.6 / 606.8 | 1T 默认 TT 即 32M；因 `j=16` 超配不可与 `j=10` 混比 |
| **`p3_batch3 (122aa89)` 1T vs Rapfi 1T** | **`j=10` / 32M** | **27-72-1** | **27.5% `[20.38%, 34.62%]`** | **-168.4** | `0/0/27/1/22` | 542.6 / 597.4 | **1T 公平基线**（= `p4_base` 1T；1 盘 224 ply 黑无合法着） |
| `p4_fix3 (812a993)` 1T vs Rapfi 1T | `j=10` / 32M | 26-73-1 | 26.5% | -177.2 | `0/0/26/1/23` | 553.1 / 605.7 | 1T 点估计略低于 `p3_batch3` |
| `p3_batch3 (122aa89)` 4T vs Rapfi 4T | `j=4` / 64M | 23-76-1 | 23.5% | -205.0 | `0/0/23/1/26` | 537.9 / 546.8 | 旧同步 ABDADA |
| `33d1515` 4T vs Rapfi 4T | `j=4` / 64M | 25-73-2 | 26.0% | -181.7 | `0/0/25/2/23` | 521.6 / 545.0 | 初版 Lazy SMP |
| **`p4_fix1 (45f46bb)` 4T vs Rapfi 4T** | **`j=4` / 32M** | **31-69-0** | **31.0% `[24.00%, 38.00%]`** | **-139.0** | `0/0/31/0/19` | 498.7 / 509.4 | **4T 公平基线**（= `p4_base` 4T；执黑 23/50、执白 8/50；1009/2081 手外测 `>1000ms`，max `1001.666ms`） |
| `p4_fix2 (dc23d00)` 4T vs Rapfi 4T | `j=4` / 32M | 29-71-0 | 29.0% | -155.5 | `0/0/29/0/21` | 506.3 / 525.1 | 1 盘 224 ply 黑无合法着 |
| `p4_fix3 (f9e92b0)` 4T vs Rapfi 4T | `j=4` / 32M | 30-66-4 | 32.0% `[25.35%, 38.65%]` | -130.9 | `0/0/30/4/16` | 515.3 / 556.0 | 内战 4T 偏弱，不作保守基准 |

#### B. 内部自博弈对照（全部 `32 MiB TT / tt_bits=20`，`--reuse-engine-state`，100 局，0 错误，0 脚本 120s 超时）

> 注：以下 100 局内战的配对 95% 置信区间均跨过 50%，数值均为**点估计**，不宣称单次 100 局已达到统计显著。

| 线程 | 对阵 | 并发 | 成绩 (W-L-D) | 得分率 (配对 95% CI) | Elo 差 | Pentanomial (WW/WD/WL/LD/LL) | 结论 |
|---|---|---:|---:|---:|---:|---|---|
| 1T | `p4_fix1_1t` vs `p3_batch3_1t` | `j=8` | 40-48-12 | 46.0% | -27.85 | `0/3/39/5/3` | 1T 代码完全相同，计时波动 |
| 1T | `p4_fix3_1t` vs `p3_batch3_1t` | `j=8` | 51-43-6 | 54.0% | +27.85 | `6/3/36/3/2` | `fix2+fix3` 在 1T 点估计为正，待单点拆测 |
| **4T** | **`p4_fix1_4t` vs `33d1515_4t`** | **`j=4`** | **50-42-8** | **54.0% `[48.44%, 59.56%]`** | **+27.85** | `5/3/38/3/1` | 点估计领先 `33d1515` |
| **4T** | **`p4_fix1_4t` vs `p3_batch3_4t`** | **`j=4`** | **49-43-8** | **53.0% `[46.35%, 59.65%]`** | **+20.87** | `6/4/33/4/3` | 点估计领先旧同步 ABDADA |
| 4T | `p4_fix2_4t` vs `p4_fix1_4t` | `j=4` | 48-41-11 | 53.5% `[47.37%, 59.63%]` | +24.36 | `5/5/34/4/2` | 点估计高于 `p4_fix1`；单凭内战未证实 `fix2` 4T 退化 |
| 4T | `p4_fix2_4t` vs `33d1515_4t` | `j=4` | 45-47-8 | 49.0% | -6.95 | `3/3/36/5/3` | 区间内波动 |
| 4T | `p4_fix3_4t` vs `p4_fix2_4t` | `j=4` | 46-48-6 | 49.0% | -6.95 | `2/2/40/4/2` | 区间内波动 |
| 4T | `p4_fix3_4t` vs `p3_batch3_4t` | `j=4` | 42-49-9 | 46.5% | -24.36 | `1/2/39/5/3` | 点估计偏弱 |
| 4T | `p4_fix3_4t` vs `33d1515_4t` | `j=4` | 43-52-5 | 45.5% | -31.35 | `2/2/37/3/6` | 点估计偏弱 |

**基准锁定结论**：
- 保留 **`p4_base`（`4065c80` = `p4_fix1 (45f46bb)` 搜索内核 + `812a993` `INFO max_memory`）** 作为下一轮干净、保守的统一对照基准：1T 与 `p3_batch3` 完全一致（对 Rapfi `27.5%`），4T 保留已验证的顺序无关投票（对 Rapfi `31.0%`，内战点估计领先 `33d1515` 与 `p3_batch3`）。

---

## 3. 已完成的前置体检与历史实验复盘

### 3.1 Step 0 前置体检数据（有效，继续指导后续优化）

- **同 `1s` + `32 MiB TT` 深度差距**：`c4_ply20` 上 1T NPS 我们 `228K` vs Rapfi `287K`，但深度 `12` vs `17`（差 5 层）；`loss4t_c9_ply22` 上 Rapfi 1T `d25-46 (-M46)` / 4T `d27-36 (-M38)`，我方仅 `d12~14`。**主矛盾仍是搜索树 EBF 与战术延伸深度**。
- **CPU 热点 Profile（`perf record`）**：
  - `c4_ply20 d=12` (1T)：NNUE `51.7%` vs **古典 `EvalCaches + movegen` `30.2%`**（`value_wide_compute_for_rule` 11.8%、`update_bucket_attack` 7.2%、`generate_candidates` 6.3%、`update_direction_cache` 3.4%、`restore_snapshot` 1.6%）；`TTTableView4::probe` 仅 `1.79%`。
  - `vct_dense_ply78 d=8` (1T)：NNUE `36.3%` vs **古典 `EvalCaches + movegen` `42.7%`**。
  - `c4_ply20 d=12` (4T)：热点比例与 1T 一致，`abdada_table` `< 1.5%`。
- **Root VCF / VCT 实盘审计（20 个中局）**：Root VCF 命中 `2/20`（保留）；Root df-pn VCT 触发 `15/20`（75%）、命中 `0/20`、`avg = 13.7ms`、`max = 67.3ms`。
- **Mix9svq `scoreBias` 对齐**：Rapfi `PolicyBuffer::scoreBias = 0`、`ScoreScale = 32`（`score = int(raw_logit * 32)`）。

### 3.2 已暂撤或否决的 3 组实验复盘（口径已勘误）

1. **已否决 — Step 1（同时改动 4 项：关 Lazy SMP `abdada_table` + 禁 Root/PV TT 截断 + `threadBias` + `max_depth=64`）**：
   - `p4_step1_4t` vs `p4_fix3_4t`：**`42-50-8`（`46.0%`，`-27.85 Elo`）**，均耗从 `507.2ms` 涨到 `551.2ms`。
   - 注：此前记录的 `tt_generation` `2.13×` 包含全部 `best_move_hint` 命中（[alphabeta.rs](file:///home/jerry/rust_ws/rust-gomoku/src/search/alphabeta.rs#L699-L705)），无法单独区分重复计算与有效共享，且本轮同时改了 4 项，后续如需重测其中的单一项须独立验证。
2. **已否决 — Step 2A+2B（一次性打包 7 项启发改动）**：
   - 打包了 `filter_nnue_tactical_defenses`、`OPPO5_EXT`、`TTM_EXT`、PV LMP、未校准 Policy Pruning、绝对 Policy LMR、无 Aspiration Window 的 `Alpha-Improvement Reduction`。
   - 1T vs Rapfi 1T 仅 **`27.0%`**（v1）和 **`28.0%`**（v2），且被速杀比例从 `40.0%` 恶化到 **`52.1% ~ 53.4%`**。**结论：严禁打包，必须逐项孤立验证。**
3. **暂缓整包合入（保留归档 `gomocup_nnue_p4_step31` 供拆项复测） — Step 3.1（NNUE 轻量缓存与 movegen 捷径）**：
   - **实验记录勘误**：此前将 `c4_ply20` 的 `tt_generation current` `733 -> 1319` 误诊为“固定 `d=8` 破坏搜索树等价”。核查证实该次测试命令漏传 `--depth 8` 且发送了 `INFO timeout_turn 0`，触发了默认限时深度上限 **`25`**（`64` 仅出现在 `p4_step1`，`p4_fix3` 与 `p4_step31` 默认限时上限均为 `25`）和 **`200ms` 限时**，因此 `733 -> 1319` 实际是 `200ms` 限时下的 TT 提示计数。
   - **归档复测事实**：在真正固定 `d8/w40`（无时限、1T、32 MiB TT）下，`p4_fix3` 与 `p4_step31` 的 `c4_ply20` 均为 **`current=750`（`4,7`，`27463` 节点）**、`loss4t_c9_ply22` 均为 **`current=1520`（`8,11`，`13943` 节点）**，且单核中位数耗时实测提速 `118.05 -> 109.86ms`、`51.74 -> 45.29ms`；另 10 个实战前缀的 `d8/d10 w40` 非计时输出亦一致（有限 case 尚不足以证明全局等价）。
   - **暂缓合入依据**：限时 4T 对同一 Rapfi，`p4_fix3` 得 `32.0%`，`p4_step31` 得 `26.0%`（按开局与执色配对重抽差 `-6%`，95% bootstrap CI 约 `[-11%, -1.5%]`），1T 内战 `41-47-12`（`47.0%`）。保留归档二进制，后续拆成单个小项复测。

---

## 4. 从 `p4_base`（`4065c80`）出发的单点推进路线

> 原则：每次只改 **1 个原子点**，必须同时过 **`1T vs p4_base (100g)`** 与 **`4T vs p4_base (100g)`** 才合入并滚动更新基准；绝不再打包。

- [x] **Milestone 0 — 锁定 `p4_base`（`4065c80`）并完成全面审计**：归档 `opponent/baselines/gomocup_nnue_p4_base`，修订全部失实实验记录。
- [x] **Step P1-1 — QVCF 写表消融与窄窗口杀分边界修复（已收口）**：
  - **已否决（`nonlocal_b4_dropped` 禁写纯盘面 TT / 加盐写表）**：无盐禁写版虽彻底消除 `B -> A` 跨序 TT 串值，但 6 个 `d8/w40` 固定局面总节点膨胀 **`+9.46%`**（`c4_ply20 +4.18%`、`loss4t_c9_ply22 +21.42%`、`p4-14-white-23 +85.42%`；`p4_fix2` 加盐版 `+16.59%`），1T 对 `p4_base` **`48.0%`** `[42.05%, 53.95%]`（`44-48-8`）、4T 对 `p4_base` **`49.0%`** `[43.68%, 54.32%]`（`45-47-8`），均低于 50%。核对 Rapfi 源码（[movepick.cpp](file:///home/jerry/rust_ws/rust-gomoku/opponent/rapfi/Rapfi/search/movepick.cpp#L503-L514)、[search.cpp](file:///home/jerry/rust_ws/rust-gomoku/opponent/rapfi/Rapfi/search/ab/search.cpp#L2236-L2263)）确认 Rapfi 同样直接以 `RANGE_SQUARE2_LINE4` 结果写入纯 `zobristKey` 作为限时速度取舍，故撤回禁写逻辑。
  - **已保留（零成本对齐 Rapfi）**：移除原 `alphabeta.rs:2328` 窄窗口 beta 截断杀分强升 `HASHF_EXACT` 的缺陷，仅存 `HASHF_BETA`（附窄窗口双杀距回归单测）。
- [ ] **Step P1-2 — 4T 杀分确认区分实际重搜与 TT 截断**：
  - 记录确认来源（实际重搜 / 根 TT / 子节点 TT 截断），避免 `+10` 层缓存杀分空转穿透 6 层确认与选线程深度；先在 `prior-10-white-11`、`p4-14-white-23` 等定向局面量实际展开、耗时和着法变化，再做 4T 对照验证。
- [ ] **Step P2-1 — Singular Extension `skip_move` 排除与 LMR 索引双开关验证**：
  - 拆为两个独立开关分别验证：（1）提前排除 `skip_move` 不占宽度与 `policy_top_logit`；（2）是否对 SE 替代着法的 LMR 索引 `+1`。
- [ ] **Step P2-2 — 精确 15 行 Rapfi `quickWinCheck` + `Mate Distance Pruning`**：
  - 对齐 `wincheck.h:35-53` 与 `search.cpp:1243-1246, 2059-2063`，单测验证零 Renju 禁手误判，再过 1T/4T 双端门禁。
- [ ] **Step P2-3 — Step 3.1 拆项复测与安全热路径提速**：
  - 每次只拆 1 小项，注意实际内存布局与依赖：`EvalSnapshot` 实测 `608B`、`empty_bucket_counts` 为 `368B`（维护供 NNUE `has_l4/has_l5` 读取的 `nonzero_bucket_mask`），`move_value` 在策略排序前参与 `vbw <= 0.0` 过滤；每项须先过嵌套 `play/undo` 不变量测试 + `case_probe` 真正固定深度多局面等价验证，再跑 1T/4T 对局。
- [ ] **Step P2-4 — `INFO max_memory` 进程总内存预算与冷启动回落 TT 约束 + 其余 EBF/排序单点**：
  - 按 Gomocup 协议口径扣减 NNUE 权重与线程缓冲后约束 TT，并同步限制冷启动古典回落路径；
  - `MainHistory + CounterMove`（`ScoreScale=32`）、单一战术延伸/裁剪项等逐项双端验收，向 1T / 4T 对 Rapfi `~40%` 推进。

---

## 5. 里程碑进度追踪

| 阶段 | 状态 | 1T vs Rapfi 1T (32M, fair) | 4T vs Rapfi 4T (32M, `j=4`) | 备注 |
|---|---|---:|---:|---|
| 历史锚点 P2 (`b69f98b`) | 已归档 | 23.5% (-205.0) | 28.0% (-164.1) | `master` 锚点 |
| P3 (`122aa89` / `33d1515`) | 已归档 | 27.5% (-168.4) | 23.5% / 26.0% (64M) | 1T 与 `p4_base` 相同 |
| **`p4_base` (`4065c80`)** | **当前基准** | **27.5% `[20.38%, 34.62%]`** | **31.0% `[24.00%, 38.00%]`** | 执黑 23/50、执白 8/50 |
| P1-1 ~ P2-4（单点推进） | 进行中 | 目标 **`~40%`** | 目标 **`~40%`** | 1T + 4T 双门禁滚动推进 |
