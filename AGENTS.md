# AGENTS.md

## 项目定位

`rust_gomoku` 是 Python reference 项目 `pygomoku` classic 主线的 Rust 重构。

当前阶段已经不是从零迁移主链，而是：

1. 守住已经完成的 Rust classic 语义。
2. 扩大 reference / Rust 差分覆盖。
3. 做性能优化和未来并行设计。
4. 所有提速都不能改变棋力、默认走法或可回归语义。

如果“更快”与“行为一致”冲突，默认选择行为一致。

## 当前仓库状态

- Rust 主线已经覆盖 board / zobrist / config / patterns / eval / movegen / ordering / TT / alphabeta / root / VCF / VCT / Gomocup / GUI。
- 当前主线是确定性串行搜索。
- Lazy SMP、root YBWC 等并行实验已经从主线移除，原因是实测收益不稳定且会改变搜索路径或棋力。
- 完整 Python reference 不随本仓库提交；本机约定路径为 `~/python_ws/pygomoku`。
- 需要运行 Python reference 差分时，优先使用 `PYGOMOKU_REF_ROOT=/path/to/pygomoku`。
- 仓库内只保留 `opponent/zhou` 作为轻量基线对手，不把 zhou 当作主语义来源。

Agent 进入本仓库后，优先阅读：

1. `AGENTS.md`
2. `README.md`
3. 如果需要核对 reference：`$PYGOMOKU_REF_ROOT/AGENTS.md` 或 `~/python_ws/pygomoku/AGENTS.md`
4. 本次改动相关的 Rust 模块、测试、差分 case
5. 必要时再读 reference 对应模块和测试

## 最高优先级原则

### 1. 不能改变棋力

这里的“不能改变棋力”在默认串行主线中具体表示：

- 固定局面、固定深度、固定宽度下，默认不改变 best move。
- 默认不改变 score、depth、tactical trace。
- 对于当前 root 差分兼容模式，默认也不改变 nodes。
- 不改变候选排序、TT 读写语义、VCF/VCT 触发顺序、fallback 随机流，除非用户明确批准并补验证。
- 不接受“平均更快但某些局面走法变了”的优化。

如果某个优化理论上“不影响棋力”但导致固定 case 的 move/score/nodes 变化，必须停下来说明原因，不要继续扩大改动。

### 2. Reference 优先

- Python reference `pygomoku` 是唯一主语义基准。
- Rust 默认行为应对齐 reference classic 主线。
- reference 内部 Python fallback / Cython 路径如有差异，先记录差异，再决定 Rust 对齐哪条实际主线。
- 不要用 zhou 的实现习惯替代 reference 语义。

### 3. 串行基线优先

- 默认主线保持单线程、确定、可回归。
- 性能优化先在串行路径做。
- 并行只能在独立分支中重新设计，主线只保留确定串行路径。
- 并行方案不能默认影响串行路径结果。

### 4. 小步、可验证

- 每次改动只覆盖一个明确问题或一个热点。
- 修改 eval / movegen / TT / alphabeta / root / VCF / VCT / protocol 前，先确定要跑哪些验证。
- 性能改动必须同时给出正确性验证和耗时对比。

## 必须保留的核心语义

### 坐标和棋子

- 坐标统一为 `(x, y)`，即 `(列, 行)`，不是 `(row, col)`。
- `Move` 是扁平化索引。
- 坐标转换应使用统一辅助函数，不要散落手写 `%` / `/`。
- 常量语义必须保持：
  - `BOARD_SIZE = 15`
  - `BLACK = 1`
  - `WHITE = -1`
  - `EMPTY = 0`

### Board 状态机

- 搜索主流程通过 `play / undo` 修改局面。
- `winner / side_to_move / move_history / move_count / zobrist_key` 必须同步。
- `replay`、复制、快照恢复必须支撑搜索和测试。
- 嵌套 `play / undo` 路径必须严格配对，尤其是 threat board / VCF / VCT。

### Search 主链

- 不擅自替换 classic alpha-beta / PVS / TT / candidate ordering 策略。
- root tactical fast path 保持 VCF 优先于 VCT。
- VCT 只在 trigger 命中后运行。
- 默认参数集中在 `src/config.rs`，不要新增散落 magic numbers。
- 默认固定搜索为 `depth=8,width=40,root_vct_depth=8`。
- 与 reference 严格差分时通常显式设为 `depth=6,width=20,root_vct_depth=4`。

### Gomocup 协议

协议层不是简单字符串解析，必须保持状态语义：

- `START`
- `RECTSTART`
- `RESTART`
- `BEGIN`
- `TURN`
- `BOARD` / `DONE`
- `TAKEBACK`
- `INFO`
- `ABOUT`
- `END`

特别注意：

- 越界和非法输入处理。
- `BOARD` 模式重建局面。
- `INFO` 对 runtime config 和 search limits 的影响。
- 任何协议改动都应补 transcript 或单元测试。

## 性能优化规则

性能优化是当前主要方向，但必须按以下规则执行。

### 可以优先做的优化

- 热点 profiling 后的低风险数据结构优化。
- 消除热路径分配。
- 固定数组替代临时 Vec。
- 缓存局部重复计算。
- 保持相同扫描顺序和 tie-break 的计算加速。
- 增量维护，但必须有 full recompute 对照或 shadow 校验。

### 高风险优化

以下改动默认高风险，必须先设计验证方案：

- 候选点生成或排序改变。
- TT replacement / probe / store 语义改变。
- alphabeta 窗口、PVS、depthdown、rootbonus 改变。
- VCF/VCT 搜索顺序、memo key、验证深度改变。
- `evaluate_board_main` 全局评估增量化替换主线。
- 使用 `unsafe` 优化热点。
- 并行搜索。

### 性能工作必须输出

每次性能工作至少说明：

- 优化目标和热点证据。
- 修改范围。
- 正确性验证结果。
- 耗时对比结果。
- 是否改变 nodes、move、score、trace。

如果没有稳定收益，应回退或标记为失败实验，不要保留复杂代码。

## 并行策略边界

当前主线不保留 Lazy SMP / YBWC 开关。

已验证失败的方向：

- root-split full-window：丢失串行 PV 的 alpha/null-window 剪枝，易增加搜索量并降棋力。
- Lazy SMP helper 填表：收益不稳定，共享 TT 可能改变主线程路径。
- 当前 root YBWC 实验：会改变 best move，对 reference 对战棋力下降。

如果未来重新做并行：

- 新建分支，不直接在主线硬塞。
- 先设计验证标准，再写代码。
- 默认串行路径必须完全保留。
- 固定局面重复运行必须稳定。
- 与串行相比，固定深度下默认不允许改变 best move / score。
- 必须跑 root diff、慢手复现、9 开局 18 线对战对比。
- 不要让共享 TT、任务完成顺序或取消时机影响默认决策。

可接受的并行方向应优先考虑：

- 不影响决策的 profiling / shadow 计算。
- 可完全 join 且顺序确定的独立任务。
- 战术搜索的并行预分析，但最终决策必须可证明等价串行语义。

## 验证策略

具体运行命令以 `README.md` 为准，避免两份文档重复维护命令细节。

Agent 选择验证范围时遵守：

- 文档或注释改动：至少跑 `git diff --check`。
- 普通 Rust 改动：跑格式、单元测试和相关二进制构建。
- eval / movegen / ordering / TT / alphabeta / root 改动：增加 root diff。
- VCF / VCT 改动：增加对应差分 case 和慢手复现 case。
- protocol / GUI 改动：增加 smoke 或 transcript 验证。
- 默认参数、候选顺序、搜索窗口、TT 语义改动：增加 Rust/reference 9 开局对战或等价固定局面回归。

如果任务涉及搜索、评估、候选、TT、VCF/VCT，不能只跑 `cargo test`。

## 文件和目录约定

- `src/config.rs`：默认参数唯一来源。
- `src/board.rs`：核心状态机。
- `src/eval/`：评估和缓存。
- `src/search/`：movegen、ordering、TT、alphabeta、root。
- `src/threats/`：threat board、VCF、VCT。
- `src/protocol/`：Gomocup 协议。
- `src/bin/gomocup_engine.rs`：Gomocup stdin/stdout 入口。
- `src/bin/gomoku_gui.rs`：本地 Web GUI。
- `src/bin/diff_probe.rs`：Rust 差分探针。
- `cases/diff/`：固定差分局面。
- `scripts/run_diff.py`：批量差分。
- `scripts/run_engine_match.py`：Rust/reference 对战。
- `opponent/zhou/`：zhou 基线对手。
- `README.bak.md` 是本地备份文件，不要提交，除非用户明确要求。

## 代码规范

- 模块、函数、变量使用 `snake_case`。
- 类型、trait、enum 使用 `PascalCase`。
- 常量使用 `UPPER_SNAKE_CASE`。
- 热路径避免无意义分配和复制。
- 优先安全 Rust；`unsafe` 需要明确收益、边界和测试。
- 注释应解释“为什么这样能保持 reference 语义”，不要写无信息量注释。
- 新增配置必须集中到 config，不要散落在 protocol、GUI 或搜索内部。

## Agent 工作流程

1. 先读 `AGENTS.md` 和 `README.md`。
2. 判断任务属于 bugfix、差分扩展、性能优化、协议/GUI、还是并行实验。
3. 若涉及 reference 语义，读取对应 reference 模块和测试。
4. 先明确验证命令，再改代码。
5. 修改后跑最小必要验证。
6. 若验证失败且原因不明确，不要继续扩大改动。
7. 提交前确认没有误加入 reference、大型对战 JSON、临时 profiling 输出或本地备份。

## 明确禁止

- 为了速度改变默认 best move、score 或候选排序。
- 未验证就重写搜索主链。
- 在主线重新接入 Lazy SMP / YBWC 旧方案。
- 把 zhou 行为当作主语义标准。
- 修改 TT、movegen、ordering、VCF/VCT 后只跑编译不跑差分。
- 把完整 Python reference 提交进本仓库。

## 一句话准则

**当前项目的核心不是“写一个更像 Rust 的新引擎”，而是在 Rust 中保留 `pygomoku` classic 棋力语义，并在不改变棋力的前提下把它变快。**
