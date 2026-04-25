"""Game controller: state machine managing the full game loop."""

import sys
import threading

import pygame

from gomoku.ai.searcher import AISearcher
from gomoku.board import Board
from gomoku.config import (
    AI_MOVE_DELAY_MS,
    AI_SEARCH_DEPTH,
    FPS,
    WINDOW_SIZE,
    GameState,
    Player,
)
from gomoku.ui.renderer import Renderer


class GameController:
    """管理游戏状态机与主循环。

    状态转移：
        MENU  →  PLAYING  →  GAME_OVER  →  MENU (循环)

    Attributes:
        screen: Pygame 渲染目标。
        clock: 帧率控制器。
        renderer: 绘制器。
    """

    def __init__(self) -> None:
        pygame.init()
        self.screen = pygame.display.set_mode((WINDOW_SIZE, WINDOW_SIZE))
        pygame.display.set_caption("Gomoku")
        self.clock = pygame.time.Clock()
        self.renderer = Renderer(self.screen)

        # 每局重置的状态
        self._board: Board
        self._ai: AISearcher
        self._human_player: Player
        self._ai_player: Player
        self._turn: Player
        self._state: GameState
        self._winner_text: str = ""

        # AI 异步计算相关
        self._ai_thinking: bool = False
        self._ai_result: tuple[int, int] | None = None
        self._ai_think_start: int = 0  # pygame.time.get_ticks()

    # ------------------------------------------------------------------
    # Public
    # ------------------------------------------------------------------

    def run(self) -> None:
        """启动并运行游戏主循环，直到玩家退出。"""
        self._start_new_game()
        while True:
            self._tick()
            # GAME_OVER 中按 R 会把 state 设回 MENU，此时开始新一局
            if self._state == GameState.MENU:
                self._start_new_game()

    # ------------------------------------------------------------------
    # Setup
    # ------------------------------------------------------------------

    def _start_new_game(self) -> None:
        """初始化新一局的全部状态，并进入 MENU 状态。"""
        self._board = Board()
        self._human_player = Player.BLACK  # 默认值，在 MENU 中确定
        self._ai_player = Player.WHITE
        self._turn = Player.BLACK
        self._winner_text = ""
        self._state = GameState.MENU
        self._ai_thinking = False
        self._ai_result = None

        self.renderer.draw_menu()
        pygame.display.flip()

    # ------------------------------------------------------------------
    # Main tick
    # ------------------------------------------------------------------

    def _tick(self) -> None:
        """处理一帧：事件 → AI → 渲染 → 限帧。"""
        if self._state == GameState.MENU:
            self._handle_menu_events()
        elif self._state == GameState.PLAYING:
            self._handle_playing_events()
            if self._state == GameState.PLAYING and self._turn == self._ai_player:
                self._ai_turn()
        elif self._state == GameState.GAME_OVER:
            self._handle_game_over_events()
        self.clock.tick(FPS)

    def _draw_thinking_indicator(self) -> None:
        """在 AI 思考时显示动态提示。"""
        elapsed = pygame.time.get_ticks() - self._ai_think_start
        dots = "." * (1 + (elapsed // 500) % 3)
        font = pygame.font.SysFont(None, 28)
        text = font.render(f"AI thinking{dots}", True, (180, 0, 0))
        rect = text.get_rect(center=(WINDOW_SIZE // 2, 15))
        # 只重绘顶部提示区域
        bg_rect = pygame.Rect(0, 0, WINDOW_SIZE, 30)
        self.renderer.draw_board(self._board)
        self.screen.blit(text, rect)
        pygame.display.flip()

    # ------------------------------------------------------------------
    # Event handlers
    # ------------------------------------------------------------------

    def _handle_menu_events(self) -> None:
        """处理 MENU 状态下的键盘事件（B / W 选色）。"""
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                self._quit()
            if event.type == pygame.KEYDOWN:
                if event.key == pygame.K_b:
                    self._human_player = Player.BLACK
                    self._ai_player = Player.WHITE
                    self._turn = Player.BLACK
                    self._enter_playing()
                elif event.key == pygame.K_w:
                    self._human_player = Player.WHITE
                    self._ai_player = Player.BLACK
                    self._turn = Player.BLACK  # 黑棋先手
                    self._enter_playing()

    def _handle_playing_events(self) -> None:
        """处理 PLAYING 状态下的鼠标落子和键盘悔棋事件。"""
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                self._quit()
            if event.type == pygame.KEYDOWN:
                if event.key == pygame.K_u:
                    self._undo()
            if event.type == pygame.MOUSEBUTTONDOWN and event.button == 1:
                if self._turn == self._human_player:
                    coords = self.renderer.pixel_to_board(event.pos)
                    if coords is not None:
                        self._place_and_check(*coords, self._human_player)

    def _handle_game_over_events(self) -> None:
        """处理 GAME_OVER 状态下的重启（R）和退出（Q）事件。"""
        for event in pygame.event.get():
            if event.type == pygame.QUIT:
                self._quit()
            if event.type == pygame.KEYDOWN:
                if event.key == pygame.K_r:
                    # 回到外层 run() 循环开始新一局
                    self._state = GameState.MENU
                elif event.key == pygame.K_q:
                    self._quit()

    # ------------------------------------------------------------------
    # Game logic helpers
    # ------------------------------------------------------------------

    def _enter_playing(self) -> None:
        """切换到 PLAYING 状态并初始化 AI。"""
        self._ai = AISearcher(depth=AI_SEARCH_DEPTH, ai_player=self._ai_player)
        self._state = GameState.PLAYING
        self.renderer.draw_board(self._board)
        pygame.display.flip()

    def _place_and_check(self, row: int, col: int, player: Player) -> None:
        """落子，更新显示，检查胜负/平局。

        Args:
            row: 行坐标。
            col: 列坐标。
            player: 落子方。
        """
        if not self._board.place(row, col, player):
            return

        self.renderer.draw_board(self._board)
        pygame.display.flip()

        if self._board.check_win(row, col):
            self._winner_text = "You win!" if player == self._human_player else "Computer wins!"
            self._enter_game_over()
        elif self._board.is_full():
            self._winner_text = "Draw!"
            self._enter_game_over()
        else:
            # 切换回合
            self._turn = self._ai_player if player == self._human_player else self._human_player

    def _ai_turn(self) -> None:
        """执行 AI 落子（非阻塞）：后台线程搜索，主线程保持响应。"""
        if not self._ai_thinking and self._ai_result is None:
            # 启动后台搜索
            self._ai_thinking = True
            self._ai_think_start = pygame.time.get_ticks()
            board_copy = self._board.copy()

            def _search() -> None:
                result = self._ai.find_best_move(board_copy)
                self._ai_result = result

            threading.Thread(target=_search, daemon=True).start()

        if self._ai_thinking and self._ai_result is None:
            # 搜索进行中，显示提示并保持事件响应
            self._draw_thinking_indicator()
            for event in pygame.event.get():
                if event.type == pygame.QUIT:
                    self._quit()
            return

        if self._ai_result is not None:
            # 搜索完成，确保至少等待 AI_MOVE_DELAY_MS 再落子
            elapsed = pygame.time.get_ticks() - self._ai_think_start
            if elapsed < AI_MOVE_DELAY_MS:
                self._draw_thinking_indicator()
                return

            move = self._ai_result
            self._ai_thinking = False
            self._ai_result = None

            if move is None:
                self._winner_text = "Draw!"
                self._enter_game_over()
                return
            self._place_and_check(*move, self._ai_player)

    def _undo(self) -> None:
        """悔棋：撤销最近两步（AI + 人类各一步），回到玩家回合。"""
        if not self._board.move_history:
            return
        if self._ai_thinking:
            return  # AI 思考中不允许悔棋
        # 撤销最多两步
        steps = min(2, len(self._board.move_history))
        for _ in range(steps):
            self._board.undo()
        self._turn = self._human_player
        self._ai_result = None
        self.renderer.draw_board(self._board)
        pygame.display.flip()

    def _enter_game_over(self) -> None:
        """切换到 GAME_OVER 状态并显示结果画面。"""
        self._state = GameState.GAME_OVER
        self.renderer.draw_game_over(self._winner_text)
        pygame.display.flip()

    # ------------------------------------------------------------------
    # Utility
    # ------------------------------------------------------------------

    @staticmethod
    def _quit() -> None:
        """退出 Pygame 并终止进程。"""
        pygame.quit()
        sys.exit()
