"""Pygame rendering for Gomoku."""

from typing import Optional

import pygame

from gomoku.board import Board
from gomoku.config import (
    BG_COLOR,
    BLACK_COLOR,
    BOARD_SIZE,
    GRID_SIZE,
    LINE_COLOR,
    MARGIN,
    RED,
    WHITE_COLOR,
    WINDOW_SIZE,
    Player,
)

# 字体大小
_FONT_LARGE = 48
_FONT_MEDIUM = 36
_FONT_SMALL = 28

# 棋子半径（留 2px 间距）
_PIECE_RADIUS = GRID_SIZE // 2 - 2

# 最后一手高亮：红色实心小方块边长
_HIGHLIGHT_SIZE = 6


class Renderer:
    """封装所有 Pygame 绘制操作。

    调用方负责 pygame.init() 和 pygame.display.flip()；
    Renderer 只负责向 screen 写入像素，不调用 display.flip()，
    方便调用方控制何时刷新（避免多次 flip）。

    Attributes:
        screen: Pygame 渲染目标 Surface。
    """

    def __init__(self, screen: pygame.Surface) -> None:
        self.screen = screen

    # ------------------------------------------------------------------
    # Public draw methods
    # ------------------------------------------------------------------

    def draw_board(self, board: Board) -> None:
        """绘制棋盘背景、网格线、所有棋子，以及最后一手高亮。

        Args:
            board: 当前棋盘状态。
        """
        self.screen.fill(BG_COLOR)
        self._draw_grid()
        self._draw_pieces(board)
        if board.last_move is not None:
            self.draw_last_move_highlight(*board.last_move)

    def draw_last_move_highlight(self, row: int, col: int) -> None:
        """在 (row, col) 处绘制红色小方块标记最后一手落子位置。

        Args:
            row: 行坐标。
            col: 列坐标。
        """
        cx, cy = self._board_to_pixel(row, col)
        half = _HIGHLIGHT_SIZE // 2
        rect = pygame.Rect(cx - half, cy - half, _HIGHLIGHT_SIZE, _HIGHLIGHT_SIZE)
        pygame.draw.rect(self.screen, RED, rect)

    def draw_menu(self) -> None:
        """绘制开局颜色选择界面。"""
        self.screen.fill(BG_COLOR)
        font = pygame.font.SysFont(None, _FONT_MEDIUM)
        lines = [
            "Gomoku",
            "",
            "Press B  →  Play as Black (first move)",
            "Press W  →  Play as White",
        ]
        line_height = _FONT_MEDIUM + 8
        total_height = line_height * len(lines)
        start_y = (WINDOW_SIZE - total_height) // 2

        title_font = pygame.font.SysFont(None, _FONT_LARGE)
        for idx, text in enumerate(lines):
            f = title_font if idx == 0 else font
            color = BLACK_COLOR if idx == 0 else (60, 60, 60)
            surface = f.render(text, True, color)
            x = (WINDOW_SIZE - surface.get_width()) // 2
            y = start_y + idx * line_height
            self.screen.blit(surface, (x, y))

    def draw_game_over(self, winner_text: str) -> None:
        """在当前棋盘画面上叠加游戏结束信息。

        Args:
            winner_text: 结果文字，例如 "You win!" / "Computer wins!" / "Draw!"
        """
        font_result = pygame.font.SysFont(None, _FONT_LARGE)
        font_prompt = pygame.font.SysFont(None, _FONT_SMALL)

        result_surf = font_result.render(winner_text, True, RED)
        prompt_surf = font_prompt.render("Press R to restart  |  Q to quit", True, RED)

        result_x = (WINDOW_SIZE - result_surf.get_width()) // 2
        result_y = (WINDOW_SIZE - result_surf.get_height()) // 2 - 20
        prompt_x = (WINDOW_SIZE - prompt_surf.get_width()) // 2
        prompt_y = result_y + result_surf.get_height() + 12

        # 半透明背景板，提升可读性
        overlay_w = max(result_surf.get_width(), prompt_surf.get_width()) + 40
        overlay_h = result_surf.get_height() + prompt_surf.get_height() + 40
        overlay = pygame.Surface((overlay_w, overlay_h), pygame.SRCALPHA)
        overlay.fill((222, 184, 135, 200))
        overlay_x = (WINDOW_SIZE - overlay_w) // 2
        overlay_y = result_y - 14
        self.screen.blit(overlay, (overlay_x, overlay_y))

        self.screen.blit(result_surf, (result_x, result_y))
        self.screen.blit(prompt_surf, (prompt_x, prompt_y))

    def pixel_to_board(self, pos: tuple[int, int]) -> Optional[tuple[int, int]]:
        """将鼠标像素坐标转换为棋盘格坐标。

        采用四舍五入到最近交叉点的策略；若落点在棋盘范围外则返回 None。

        Args:
            pos: 鼠标像素坐标 (x, y)。

        Returns:
            棋盘坐标 (row, col)；越界时返回 None。
        """
        col = round((pos[0] - MARGIN) / GRID_SIZE)
        row = round((pos[1] - MARGIN) / GRID_SIZE)
        if 0 <= row < BOARD_SIZE and 0 <= col < BOARD_SIZE:
            return row, col
        return None

    # ------------------------------------------------------------------
    # Private helpers
    # ------------------------------------------------------------------

    def _draw_grid(self) -> None:
        """绘制 BOARD_SIZE × BOARD_SIZE 的网格线。"""
        for i in range(BOARD_SIZE):
            # 横线
            pygame.draw.line(
                self.screen,
                LINE_COLOR,
                (MARGIN, MARGIN + i * GRID_SIZE),
                (MARGIN + (BOARD_SIZE - 1) * GRID_SIZE, MARGIN + i * GRID_SIZE),
                1,
            )
            # 竖线
            pygame.draw.line(
                self.screen,
                LINE_COLOR,
                (MARGIN + i * GRID_SIZE, MARGIN),
                (MARGIN + i * GRID_SIZE, MARGIN + (BOARD_SIZE - 1) * GRID_SIZE),
                1,
            )

    def _draw_pieces(self, board: Board) -> None:
        """绘制棋盘上的所有棋子。

        Args:
            board: 当前棋盘状态。
        """
        for row in range(BOARD_SIZE):
            for col in range(BOARD_SIZE):
                player = board.grid[row][col]
                if player == Player.NONE:
                    continue
                center = self._board_to_pixel(row, col)
                if player == Player.BLACK:
                    pygame.draw.circle(self.screen, BLACK_COLOR, center, _PIECE_RADIUS)
                else:
                    pygame.draw.circle(self.screen, WHITE_COLOR, center, _PIECE_RADIUS)
                    pygame.draw.circle(self.screen, BLACK_COLOR, center, _PIECE_RADIUS, 1)

    def _board_to_pixel(self, row: int, col: int) -> tuple[int, int]:
        """将棋盘坐标转换为像素中心坐标。

        Args:
            row: 行坐标。
            col: 列坐标。

        Returns:
            像素坐标 (x, y)。
        """
        return MARGIN + col * GRID_SIZE, MARGIN + row * GRID_SIZE
