"""Constants and configuration for Gomoku game."""

from enum import IntEnum


# ============ Player Enum ============
class Player(IntEnum):
    NONE = 0  # 空位
    BLACK = 1  # 黑棋 (先手)
    WHITE = 2  # 白棋


# ============ Game State Enum ============
class GameState(IntEnum):
    MENU = 0  # 颜色选择菜单
    PLAYING = 1  # 对局进行中
    GAME_OVER = 2  # 对局结束


# ============ Board Parameters ============
BOARD_SIZE: int = 15  # 棋盘行列数
GRID_SIZE: int = 40  # 格子间距 (pixels)
MARGIN: int = 20  # 棋盘边距 (pixels)
WINDOW_SIZE: int = MARGIN * 2 + GRID_SIZE * (BOARD_SIZE - 1)  # 窗口边长
FPS: int = 30  # 帧率

# ============ Color Definitions ============
BLACK_COLOR: tuple[int, int, int] = (0, 0, 0)
WHITE_COLOR: tuple[int, int, int] = (255, 255, 255)
BG_COLOR: tuple[int, int, int] = (222, 184, 135)  # 棋盘背景色 (木色)
LINE_COLOR: tuple[int, int, int] = (0, 0, 0)  # 棋盘线条颜色
RED: tuple[int, int, int] = (255, 0, 0)  # 提示文字颜色

# ============ AI Configuration ============
AI_SEARCH_DEPTH: int = 5  # 默认搜索深度；局部热度排序后 5 层仍可交互
AI_MAX_CANDIDATES: int = 20  # 每层仅保留最热的前 N 个候选点，控制分支爆炸
AI_CANDIDATE_RANGE: int = 2  # 搜索候选点邻域半径；radius=2 可补上隔一格攻防点
AI_VCF_MAX_DEPTH: int = 10  # VCF 以“攻击方步数”计的最大递归深度
AI_VCF_CANDIDATES: int = 16  # VCF 仅扩展最热的前 N 个强制进攻候选
AI_VCF_BACKEND: str = "auto"  # python | swift | auto；auto 会优先使用已构建的 Swift worker
AI_VCF_SWIFT_COMMAND: str = ""  # Swift VCF 可执行文件命令；空字符串表示自动发现
AI_VCF_SWIFT_TIMEOUT_MS: int = 500  # Swift VCF IPC 超时；首次启动额外放宽到 1s
AI_MOVE_DELAY_MS: int = 10  # AI 落子前的延时 (ms), 便于观察
