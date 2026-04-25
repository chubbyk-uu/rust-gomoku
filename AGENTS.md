# AGENTS.md

## 项目概述

`rust_gomoku` 的首要目标不是“做一个新的五子棋引擎”，而是：

**在尽量不破坏语义的前提下，用 Rust 完整重构参考项目 `pygomoku`。**

这里的“语义”包括但不限于：

- 棋盘状态变化语义
- 坐标与落子编码语义
- 搜索、评估、候选点、TT、VCF / VCT 的行为语义
- Gomocup 协议行为
- 默认参数与默认开局响应
- 固定局面上的回归结果

如果 Rust 实现更“优雅”、更“地道”，但行为与 `pygomoku` 主线不一致，默认视为偏离目标，而不是优化。

## 当前仓库状态

- classic 主链已经推进到 board / eval / search / VCF / VCT / Gomocup / GUI
- 完整 Python reference 不随本仓库提交；本机约定路径为 `~/python_ws/pygomoku`
- 需要运行 Python reference 差分时，优先通过 `PYGOMOKU_REF_ROOT=/path/to/pygomoku` 指定外部 reference
- 仓库内保留 `opponent/zhou` 作为 zhou 基线对手

因此，现阶段 Agent 在本仓库工作时，应把以下文件视为最高优先级上下文：

1. 本仓库 `AGENTS.md`
2. 本仓库 `README.md`
3. `$PYGOMOKU_REF_ROOT/AGENTS.md` 或 `~/python_ws/pygomoku/AGENTS.md`
4. `$PYGOMOKU_REF_ROOT/README.md`、`tests/` 和 `pygomoku/` 对应模块源码

## 核心工程原则

### 1. 参考实现优先

- 外部 Python reference `pygomoku` 是当前唯一语义基准
- 发生行为歧义时，优先对齐参考实现和参考测试，而不是靠主观判断重写
- 不要把仓库里的 `zhou` 实现习惯当作主语义来源

### 2. 先语义一致，再谈性能

- 先建立可验证的 Rust 等价实现
- 多线程能力必须在架构设计阶段就纳入考虑，不能等主实现完成后再硬塞
- 但多线程落地仍要以不破坏 reference 语义为前提
- 再逐步做性能优化、内存布局优化、并行化或 `unsafe` 热点处理
- 不接受“更快但行为变了”的无证据改动

### 3. 小步迁移

- 单次改动尽量只覆盖一个明确模块或一条明确行为链路
- 每迁移一层，就补该层对应测试或对齐校验
- 不要在尚未验证底层状态机前直接实现完整搜索器

### 4. 确定性优先

- 参考项目强调 `classic` 主线的稳定、确定、可回归
- Rust 重构也必须优先保持确定性
- 涉及候选排序、哈希、回退随机流、协议应答时，要特别警惕“看似合法但结果不同”的回归

### 5. 先核对 reference 内部语义一致性

- 参考项目同时存在 Python 实现和 Cython 加速路径
- 理论上 Cython 与 Python fallback 应保持语义一致，但迁移时不能只假设它们一致
- 凡是参考项目含 `.py` + `.pyx` 或 native fallback 双路径的模块，迁移前都应先检查两条路径的行为是否一致
- 如果发现 reference 的 Python fallback 与 Cython 存在语义漂移，应先记录差异，再决定 Rust 以哪条路径为基准；默认优先以测试覆盖到的实际主线行为为准
- 不要把 reference 内部潜在 bug 原样扩散到更多 Rust 模块而不留痕迹

## 参考项目关键语义

以下约束在 Rust 中必须显式保留。

### 坐标与落子编码

**关键**：参考项目统一使用 `(x, y)`，即 `(列, 行)`，不是 `(row, col)`。

- `x` 表示列
- `y` 表示行
- `Move` 是扁平化整数索引
- 任何 Rust 端坐标转换都应集中到统一函数，避免散落手写 `%` / `/`

如果未来同时保留：

- 协议坐标
- 内部索引
- UI 坐标
- 调试输出

则必须在类型或辅助函数层面显式区分，避免 `(x, y)` / `(row, col)` 混淆。

### 棋盘与颜色常量

参考项目关键约定：

- `BOARD_SIZE = 15`
- `BLACK = 1`
- `WHITE = -1`
- `EMPTY = 0`

Rust 端不要轻易改成另一套对外可见语义，例如：

- `enum` 序号与参考值不一致但又直接参与协议/序列化
- `bool` 表示颜色
- 隐式使用 `usize` 代替带符号 side 值

可以内部包装类型，但外部行为和语义必须与参考实现一致。

### 棋盘状态机

`Board` 在参考项目中是最核心的可信状态机。Rust 迁移时应保持以下原则：

- 搜索主流程应通过统一的 `play` / `undo` 路径修改正式局面
- `winner`、`side_to_move`、`move_history`、`move_count`、`zobrist_key` 要保持同步更新
- 对非法着法、重复落子、错误 side 的处理要与参考语义一致
- `replay`、复制、快照恢复等能力要能支撑搜索和测试

### `classic` 主线语义

参考项目不是“随便找个更强的引擎替代掉”，而是维护一条 `classic` 主线语义。

Rust 重构时：

- 默认以 `pygomoku` 当前 `classic` 行为为准
- 不因为 Rust 里更容易重写，就主动替换搜索策略
- 不擅自改默认参数、默认搜索宽度、默认战术开关

### 协议语义

Gomocup 协议层不是简单字符串解析，它有明确状态行为。

至少要保留对这些状态转换的精确处理：

- `START`
- `RESTART`
- `BEGIN`
- `TURN`
- `BOARD`
- `DONE`
- `TAKEBACK`
- `INFO`
- `ABOUT`

尤其要保留：

- 非法输入处理
- 越界坐标处理
- `BOARD` 模式重建局面逻辑
- `INFO` 对运行时配置的影响

### Python fallback 与 Cython 加速语义

参考项目里有不少热点模块同时维护了 Python 与 Cython 两条实现路径。

Rust 迁移这些模块时，要额外确认：

- Python fallback 与 Cython 输出是否一致
- 边界局面、非法输入、缓存恢复、胜负判断是否一致
- 默认运行路径到底更接近 Python 还是已编译 Cython

尤其要重点核对这些区域：

- `patterns/*`
- `eval/*`
- `search/_movegen_cy.pyx` 对应 `search/movegen.py`
- `search/_ordering_cy.pyx` 对应 `search/ordering.py`
- `threats/_threat_board_cy.pyx` 对应 `threats/threat_board.py`

如果 reference 两条路径不一致：

- 先补一个能复现差异的测试或最小局面
- 在 `AGENTS.md`、测试名或代码注释里明确“Rust 当前对齐的是哪条语义”
- 除非用户明确要求修 reference bug，否则不要一边迁移一边偷偷改语义

### 多线程设计边界

Rust 实现**必须在设计阶段就考虑多线程架构**，否则很难充分体现 Rust 在现代多核 CPU 上的性能潜力。

但这个要求的前提仍然是：**不改变 reference 对外语义**。

可以接受的方向：

- 从一开始就为并行搜索预留清晰模块边界
- 从一开始设计任务拆分、线程池、取消机制、统计聚合、共享状态访问方式
- 让搜索框架、TT、时间控制、日志采样在架构上能够承载并发
- 在此基础上保留一个可验证的 reference 兼容执行路径

必须明确控制的风险：

- 多线程不能默认改变 best move、score、候选顺序、TT 命中行为
- 不能因为竞态或合并顺序不同，导致相同局面在相同配置下返回不同结果
- 不能让时间控制、协议响应、日志顺序变得不可预测
- 共享 TT、共享缓存、根节点并行搜索都可能引入非确定性，必须单独验证

工程建议：

- 架构设计阶段就要考虑并发，而不是事后补丁式加入
- 即使内部按并发架构设计，也应保留单线程或确定性兼容模式，用于 reference 对齐、调试和回归
- 多线程可以是主性能路线，但必须有办法退回到稳定的兼容执行模式
- TT、候选集、任务切分、根节点并行、停止条件都要从第一版设计时就考虑并发影响
- 对多线程模式至少要额外验证固定局面、固定深度、固定时间预算下的结果稳定性

## Rust 重构建议顺序

当前仓库最适合的迁移顺序如下。

### 第一阶段：基础数据与不变量

先实现并验证：

1. `constants`
2. `types`
3. `move` 编码 / 解码
4. `board`
5. `zobrist`

没有这层，不要急着上搜索。

### 第二阶段：配置与评估基础设施

优先迁移：

1. `config`
2. `patterns`
3. `eval/caches`
4. `eval/local`
5. `eval/global_eval`

原因：

- 搜索依赖这些基础语义
- 这些模块里有大量容易“结果差一点但不易察觉”的隐性回归

### 第三阶段：搜索主链路

再迁移：

1. `search/movegen`
2. `search/ordering`
3. `search/tt`
4. `search/alphabeta`
5. `search/root`

注意：

- TT、候选排序、fallback 行为都要做固定局面对齐
- 不要先做“Rust 风格大重构”再想办法对齐 reference

### 第四阶段：战术模块

迁移：

1. `threat_board`
2. `vcf`
3. `vct`

这些模块语义脆弱、耦合深，必须建立针对性回归。

### 第五阶段：外部入口

最后实现：

1. Gomocup 协议入口
2. 命令行引擎入口
3. GUI 或其他外部集成
4. 与 `zhou` 的对战验证链路

不要在核心语义尚未稳定时优先做 GUI。

## 推荐的 Rust 模块映射

若无更强约束，建议 Rust 目录结构尽量贴近参考项目模块边界，降低比对成本。

可参考：

```text
src/
├── lib.rs
├── constants.rs
├── types.rs
├── board.rs
├── zobrist.rs
├── config.rs
├── patterns/
├── eval/
├── search/
├── threats/
├── protocol/
└── bin/
    └── gomocup_engine.rs
```

原则：

- 模块边界尽量与参考项目一一对应
- 文件名尽量贴近参考模块名
- 不要过早引入为“抽象而抽象”的 trait 层

## 代码规范

### 命名与风格

- 模块、函数、变量使用 `snake_case`
- 类型、trait、enum 使用 `PascalCase`
- 常量使用 `UPPER_SNAKE_CASE`
- 对外公开类型与函数优先写清晰、简短的 rustdoc
- 优先显式类型，避免热路径里出现难以追踪的隐式转换

### Rust 编码要求

- 优先安全代码，`unsafe` 只能在确有必要且已验证收益时引入
- 不要为了“更泛型”牺牲语义可比对性
- 热路径避免无意义分配与复制
- 涉及棋盘索引、哈希、缓存时优先使用固定宽度整数类型
- 需要与参考实现对齐的默认值必须集中定义，避免散落 magic numbers
- 任何跨模块共享的不变量，都应通过类型或辅助函数固化

### 注释要求

- 注释重点说明“为何这样写能对齐 reference 语义”
- 不写无信息量注释
- 对明显偏离直觉但必须兼容参考实现的逻辑，要写明兼容原因

## 测试与验证策略

Rust 重构不能只做“能编译”或“能下一盘棋”的验证。

### 最低要求

- 新增模块必须附带对应单元测试
- 修 bug 优先补回归测试
- 修改搜索、评估、缓存、协议时，不接受只有手工验证没有自动测试

### 对齐优先级

建议按参考项目测试顺序逐步建立 Rust 侧对齐用例：

1. `test_board`
2. `test_zobrist`
3. `test_config`
4. `test_patterns`
5. `test_eval`
6. `test_movegen`
7. `test_tt`
8. `test_search`
9. `test_vcf`
10. `test_vct`
11. `test_protocol`
12. `test_gomocup_engine`

### 推荐验证方式

- 对固定局面做 reference / Rust 双端结果比对
- 对关键默认配置做快照校验
- 对搜索结果做固定局面回归
- 对协议输入输出做逐条对齐测试
- 对 reference 中 Python fallback / Cython 双路径模块做交叉对照
- 对 Rust 多线程模式做重复运行稳定性测试，确认相同输入不会漂移

如果条件允许，优先建立“同一局面同时喂给 Python reference 和 Rust 实现”的对照测试或夹具。

## Agent 工作建议

适合本仓库的工作顺序：

1. 先读本仓库 `AGENTS.md`
2. 读根目录 `README.md`
3. 如果任务需要核对 reference，先解析 `PYGOMOKU_REF_ROOT`；未设置时使用本机约定 `~/python_ws/pygomoku`
4. 读本次改动相关的 reference 模块与测试
5. 明确本次任务是“语义迁移”还是“性能优化”
6. 先写或补测试，再改实现

### 每次动手前要先确认

- 这次改动对应 reference 的哪个模块
- 需要对齐哪些测试或固定局面
- 是否涉及 `(x, y)` 与 `(row, col)` 转换
- 是否影响默认参数、默认搜索行为或协议行为
- reference 该模块是否同时存在 Python fallback 与 Cython / native 路径
- 如果准备引入并行，是否会影响确定性、TT 行为、候选排序或时间控制

### 明确禁止

- 未验证语义就大规模重写搜索
- 因为 Rust 写起来方便，就改变参考项目的数据流
- 将 `zhou` 的实现细节直接迁入主线
- 在没有测试的情况下修改 TT、候选排序、VCF / VCT、协议状态机

## 参考项目概述

`pygomoku` 是一个 **Python + Cython** 的自由规则五子棋引擎项目。

核心能力：

- 15x15 自由规则五子棋引擎
- 迭代加深 + Alpha-Beta + TT + 候选点裁剪
- VCF 战术搜索
- root-only VCT 战术搜索
- 可选 Cython 热点加速
- Pygame GUI 人机对战
- Gomocup 协议引擎入口
- 与仓库内基线对手 `opponent/zhou` 的固定开局批量对战

技术栈：

- Python 3.11+
- setuptools / editable install
- Cython 可选扩展
- pytest + pytest-xdist
- pygame（GUI 可选）

## 一句话准则

**任何 Rust 改动，默认先问自己：它是在更完整地复现 `pygomoku`，还是在不知不觉地重新发明另一个引擎。**
