import sys
import unittest
from pathlib import Path
from unittest.mock import patch

SCRIPTS_DIR = Path(__file__).resolve().parents[1] / "scripts"
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import run_gomocup_match as match_runner


class RunGomocupMatchTests(unittest.TestCase):
    def test_summarize_computes_paired_pentanomial_elo_and_terminations(self):
        games = [
            # Pair 0: A wins both -> WW
            {
                "case_index": 0,
                "engine_a_side": "black",
                "winner_engine": "a",
                "termination": "win",
                "error": None,
                "moves": [{"opening": False, "engine": "a", "elapsed_ms": 10.0}],
            },
            {
                "case_index": 0,
                "engine_a_side": "white",
                "winner_engine": "a",
                "termination": "win",
                "error": None,
                "moves": [{"opening": False, "engine": "b", "elapsed_ms": 12.0}],
            },
            # Pair 1: A wins one, board_full_draw one -> WD
            {
                "case_index": 1,
                "engine_a_side": "black",
                "winner_engine": "a",
                "termination": "win",
                "error": None,
                "moves": [],
            },
            {
                "case_index": 1,
                "engine_a_side": "white",
                "winner_engine": "draw",
                "termination": "board_full_draw",
                "error": None,
                "moves": [],
            },
            # Pair 2: A loses both -> LL
            {
                "case_index": 2,
                "engine_a_side": "black",
                "winner_engine": "b",
                "termination": "win",
                "error": None,
                "moves": [],
            },
            {
                "case_index": 2,
                "engine_a_side": "white",
                "winner_engine": "b",
                "termination": "error",
                "error": "a: engine crashed",
                "moves": [],
            },
        ]

        summary = match_runner.summarize(games, ("a", "b"))
        self.assertEqual(summary["wins"], {"a": 3, "b": 2, "draw": 1, "timeout": 0})
        self.assertEqual(summary["errors"], 1)
        self.assertEqual(
            summary["terminations"], {"win": 4, "board_full_draw": 1, "error": 1}
        )
        paired = summary["paired"]
        self.assertEqual(paired["pairs"], 3)
        self.assertEqual(paired["incomplete_pairs"], 0)
        self.assertEqual(paired["capped_pairs"], 0)
        self.assertEqual(
            paired["pentanomial"],
            {"WW": 1, "WD": 1, "WL_or_DD": 0, "LD": 0, "LL": 1},
        )
        self.assertAlmostEqual(paired["score_pct"], 58.33, places=2)
        self.assertIsNotNone(paired["ci95_pct"])
        self.assertGreater(paired["ci95_pct"], 0.0)
        self.assertGreater(paired["elo"], 0.0)

    def test_summarize_excludes_max_moves_cap_from_paired_and_avoids_zero_ci(self):
        # 1) A win + max_moves_cap pair must be excluded as incomplete, not scored 75% +/- 0.
        capped_games = [
            {
                "case_index": 0,
                "engine_a_side": "black",
                "winner_engine": "a",
                "termination": "win",
                "error": None,
                "moves": [],
            },
            {
                "case_index": 0,
                "engine_a_side": "white",
                "winner_engine": "draw",
                "termination": "max_moves_cap",
                "error": None,
                "moves": [],
            },
        ]
        capped_summary = match_runner.summarize(capped_games, ("a", "b"))
        self.assertEqual(capped_summary["paired"]["pairs"], 0)
        self.assertEqual(capped_summary["paired"]["incomplete_pairs"], 1)
        self.assertEqual(capped_summary["paired"]["capped_pairs"], 1)
        self.assertIsNone(capped_summary["paired"]["score_pct"])
        self.assertIsNone(capped_summary["paired"]["ci95_pct"])

        # 2) Six 1-1 pairs have zero sample variance; CI must still be strictly positive.
        all_tied_games = []
        for idx in range(6):
            all_tied_games.append(
                {
                    "case_index": idx,
                    "engine_a_side": "black",
                    "winner_engine": "a",
                    "termination": "win",
                    "error": None,
                    "moves": [],
                }
            )
            all_tied_games.append(
                {
                    "case_index": idx,
                    "engine_a_side": "white",
                    "winner_engine": "b",
                    "termination": "win",
                    "error": None,
                    "moves": [],
                }
            )
        tied_summary = match_runner.summarize(all_tied_games, ("a", "b"))
        paired = tied_summary["paired"]
        self.assertEqual(paired["pairs"], 6)
        self.assertEqual(paired["score_pct"], 50.0)
        self.assertEqual(paired["elo"], 0.0)
        self.assertIsNotNone(paired["ci95_pct"])
        self.assertGreater(paired["ci95_pct"], 10.0)
        self.assertLess(paired["elo_ci95_low"], -50.0)
        self.assertGreater(paired["elo_ci95_high"], 50.0)

    def test_run_match_task_records_engine_runtime_error_without_raising(self):
        class FailingEngine:
            def __init__(self, *, name, **_kwargs):
                self.name = name

            def move(self, _moves, _side, _timeout):
                raise RuntimeError(f"{self.name}: unexpected move response: ERROR No legal move.")

            def close(self):
                pass

        case = match_runner.MatchCase(
            name="err_case",
            moves=((7, 7),),
            side_to_move=match_runner.WHITE,
            tags=(),
        )
        task = match_runner.MatchTask(0, case, match_runner.BLACK)
        spec_a = match_runner.EngineSpec("a", "true", Path("."), [], True)
        spec_b = match_runner.EngineSpec("b", "true", Path("."), [], True)

        with patch.object(match_runner, "GomocupEngine", FailingEngine):
            game = match_runner.run_match_task(
                task,
                engine_a=spec_a,
                engine_b=spec_b,
                max_moves=10,
                move_timeout_sec=5.0,
                game_timeout_sec=30.0,
                rule="freestyle",
                referee_command="",
                referee_cwd=Path("."),
            )

        self.assertEqual(game["termination"], "error")
        self.assertEqual(game["winner_engine"], "a")
        self.assertEqual(game["winner_side"], "black")
        self.assertIn("ERROR No legal move.", game["error"])

    def test_run_match_task_marks_max_moves_cap_distinctly_from_full_board_draw(self):
        class DummyEngine:
            def __init__(self, *, name, **_kwargs):
                self.name = name

            def move(self, _moves, _side, _timeout):
                raise AssertionError("should not be called when max_moves already reached")

            def close(self):
                pass

        case = match_runner.MatchCase(
            name="cap_case",
            moves=((7, 7), (8, 8)),
            side_to_move=match_runner.BLACK,
            tags=(),
        )
        task = match_runner.MatchTask(0, case, match_runner.BLACK)
        spec_a = match_runner.EngineSpec("a", "true", Path("."), [], True)
        spec_b = match_runner.EngineSpec("b", "true", Path("."), [], True)

        with patch.object(match_runner, "GomocupEngine", DummyEngine):
            game = match_runner.run_match_task(
                task,
                engine_a=spec_a,
                engine_b=spec_b,
                max_moves=2,
                move_timeout_sec=5.0,
                game_timeout_sec=30.0,
                rule="freestyle",
                referee_command="",
                referee_cwd=Path("."),
            )

        self.assertEqual(game["winner_engine"], "draw")
        self.assertEqual(game["termination"], "max_moves_cap")
