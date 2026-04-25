"""Entry point: python -m gomoku"""

from gomoku.game import GameController


def main() -> None:
    controller = GameController()
    controller.run()


if __name__ == "__main__":
    main()
