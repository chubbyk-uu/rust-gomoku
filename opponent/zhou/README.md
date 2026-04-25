# 五子棋 (Gomoku)

基于 Pygame 的五子棋人机对弈游戏，AI 使用 Minimax + Alpha-Beta 剪枝算法，配合候选点排序和组合棋型评估。

> **致敬原项目** 🙏  
> 本项目基于 [chubbyk-uu/gomoku-test](https://github.com/chubbyk-uu/gomoku-test) 进行二次开发。感谢原作者的优秀代码基础！

## 与原项目的区别

| 特性 | 原项目 | 本项目 |
|------|--------|--------|
| AI 评估参数 | 固定默认值 | **进化策略自动调优**，生成最优参数 |
| 调优工具 | 无 | **完整调优系统** (`tune.py`) |
| 参数配置 | 硬编码 | **可配置化** (`best_params.json`) |
| 棋型分值 | 经验值 | **通过 AI 自对弈进化** 得出 |

### 新增功能详解

**🔬 AI 超参数自动调优 (`tune.py`)**

借鉴 AutoML 的思路，实现了完整的进化策略调优器：

- **参数变异**：对评估函数的各项分值进行随机变异
- **AI 对战**：变异后的 AI 与基准 AI 进行多局对抗
- **胜率评估**：统计胜率，保留表现更优的参数
- **循环迭代**：持续进化，寻找全局最优参数组合

可调参数包括：活四、冲四、活三、眠三、活二、眠二等棋型分值，以及组合加分和防守权重。

**📊 调优结果 (`best_params.json`)**

通过多轮进化得到的最优参数（部分示例）：

```json
{
  "score_open_four": 7113,      // 活四分值 (原: 10000)
  "score_half_four": 1497,      // 冲四分值 (原: 1000)
  "score_open_three": 805,      // 活三分值 (原: 1000)
  "defense_weight": 1.472       // 防守权重 (原: 1.5)
}
```

> 注：最优参数通过数百局 AI 自对弈进化得出，实战强度优于原始经验值。

**📈 调优统计**

- 进化代数：30 代
- 每代对局：20 局（AI vs 基准 AI）
- 胜率阈值：>55% 保留，>70% 强制保留
- 最终参数在第 30 代收敛，胜率稳定在 65%

## 界面截图说明

- **开局界面**：木色背景居中显示标题和按键提示，按 `B` 执黑先手，按 `W` 执白后手
- **对局界面**：15×15 棋盘，黑棋实心圆、白棋空心圆，最后一手用红色小方块标记
- **结束界面**：棋盘保留，中央叠加半透明结果框，显示胜负文字及重启/退出提示

## 功能特性

- 15×15 标准棋盘，黑棋先手
- 人机对弈，可选执黑（先手）或执白（后手）
- AI 使用 Minimax + Alpha-Beta 剪枝，默认搜索深度 3
- 候选点排序（Move Ordering）：先搜索评分高的着法，显著提升剪枝效率
- 组合棋型检测：双活三、活三+冲四额外加分
- 防守加权：对对手威胁分乘以 1.5 倍，AI 更积极防守
- 支持悔棋（一次撤回玩家和 AI 各一步）
- 最后一手落子高亮

## 安装

**环境要求：** Python 3.10+

```bash
# 克隆项目
git clone <repo-url>
cd gomoku-test

# 安装运行依赖
pip install pygame

# 或安装完整开发依赖（含测试、格式化、lint 工具）
pip install -e ".[dev]"
```

## 运行

```bash
# 安装后以模块方式运行（推荐）
PYTHONPATH=src python -m gomoku

# 或先 pip install -e . 再直接运行
python -m gomoku
```

## 操作快捷键

| 场景 | 按键 / 操作 | 说明 |
|------|------------|------|
| 开局界面 | `B` | 执黑（先手） |
| 开局界面 | `W` | 执白（后手，AI 先走） |
| 对局中 | 鼠标左键 | 在棋盘交叉点落子 |
| 对局中 | `U` | 悔棋（撤回玩家和 AI 各一步） |
| 游戏结束 | `R` | 重新开始新一局 |
| 游戏结束 | `Q` | 退出游戏 |
| 任意时刻 | 关闭窗口 | 退出游戏 |

## AI 算法简介

### Minimax + Alpha-Beta 剪枝

递归搜索博弈树：AI（最大化方）和人类（最小化方）轮流模拟落子，Alpha-Beta 剪枝跳过不影响结果的分支，将搜索复杂度从 O(b^d) 大幅降低。

### 候选点排序（Move Ordering）

搜索前对每个候选点模拟落子并用评估函数快速打分，按分数排序后再进行深度搜索。好着法排在前面，Alpha-Beta 剪枝可以更早截断，实测比无排序快 2–3 倍，同等时间内可多搜一层。

### 评估函数

扫描棋盘上所有连续棋型，对每条线统计 `(连子数, 封堵端数)` 并查表打分：

| 棋型 | 分值 |
|------|------|
| 五连 | 100,000 |
| 活四 | 10,000 |
| 冲四 | 1,000 |
| 活三 | 1,000 |
| 眠三 | 100 |
| 活二 | 100 |

**组合加分**：双活三或活三+冲四各加 5,000（近似必杀局面）

**防守加权**：对手得分乘以 1.5 倍后再扣除，AI 优先防守紧迫威胁

## 项目结构

```
gomoku-test/
├── src/gomoku/
│   ├── __main__.py    # 入口: python -m gomoku
│   ├── config.py      # 常量与枚举 (Player, GameState, 尺寸, 颜色, AI 参数)
│   ├── board.py       # Board 类 (落子/悔棋/胜负/候选点)
│   ├── game.py        # GameController 状态机主循环
│   ├── ai/
│   │   ├── evaluator.py   # 评估函数 + 组合棋型 + 防守加权
│   │   └── searcher.py    # AISearcher (Minimax + 候选点排序)
│   └── ui/
│       └── renderer.py    # Renderer (棋盘/棋子/菜单/结束画面)
└── tests/
    ├── test_board.py      # Board 逻辑测试 (14 个用例)
    ├── test_evaluator.py  # 评估函数测试 (13 个用例)
    └── test_searcher.py   # AI 搜索测试 (5 个用例)
```

## 开发者指南

### 运行测试

```bash
# 全量测试
pytest tests/ -v

# 单个模块
pytest tests/test_board.py -v
```

> 注意：`pytest` 通过 `pyproject.toml` 的 `pythonpath = ["src"]` 和 `--import-mode=importlib` 自动处理包路径，无需额外设置 `PYTHONPATH`。

### 代码规范

```bash
# 格式化（line-length=99）
black src/ tests/ --line-length 99

# Lint 检查
ruff check src/ tests/

# 自动修复 lint 问题
ruff check src/ tests/ --fix
```

规范要求：
- 所有函数签名必须有 type hints
- Docstring 使用 Google 风格，中文注释 + 英文 docstring
- 类用 `PascalCase`，函数/变量用 `snake_case`，常量用 `UPPER_SNAKE_CASE`

### 调整 AI 强度

修改 `src/gomoku/config.py`：

```python
AI_SEARCH_DEPTH: int = 3    # 搜索深度，建议 2–4
AI_MOVE_DELAY_MS: int = 500  # AI 落子延时（ms），0 为即时响应
```

深度每增加 1，搜索时间约增加 3–5 倍（有候选点排序加持）。

### 运行 AI 参数调优

```bash
# 运行进化调优（默认 30 代，每代 20 局）
python tune.py

# 调优结果
# - best_params.json: 最优参数配置
# - tune_results.jsonl: 完整进化日志
```

调优过程使用多进程并行计算，会自动保存中间结果。
