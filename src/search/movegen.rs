//! Normal search candidate generation.

use std::collections::HashSet;
use std::sync::LazyLock;

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::EngineConfig;
use crate::constants::{BOARD_AREA, BOARD_SIZE, EMPTY, WIN};
use crate::eval::{attack_level, move_value, EvalCaches};
use crate::patterns::{Line, ShapeLabel, DIAGONAL_DOWN, DIAGONAL_UP, HORIZONTAL, VERTICAL};
use crate::rules::{classify_forbidden_move, ForbiddenKind, RuleSet};
use crate::types::Move;

const COVER_DIRS: [(isize, isize); 32] = [
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
    (-2, -2),
    (-2, -1),
    (-2, 0),
    (-2, 1),
    (-2, 2),
    (-1, -2),
    (-1, 2),
    (0, -2),
    (0, 2),
    (1, -2),
    (1, 2),
    (2, -2),
    (2, -1),
    (2, 0),
    (2, 1),
    (2, 2),
    (-3, -3),
    (-3, 0),
    (-3, 3),
    (0, -3),
    (0, 3),
    (3, -3),
    (3, 0),
    (3, 3),
];

const COVER_NEIGHBOR_CAP: usize = 32;
const COVER_SENTINEL: Move = u16::MAX;

static COVER_NEIGHBORS: LazyLock<[[Move; COVER_NEIGHBOR_CAP]; BOARD_AREA]> = LazyLock::new(|| {
    let mut table = [[COVER_SENTINEL; COVER_NEIGHBOR_CAP]; BOARD_AREA];
    for move_index in 0..BOARD_AREA {
        let x = move_index % BOARD_SIZE;
        let y = move_index / BOARD_SIZE;
        let mut count = 0;
        for &(dx, dy) in COVER_DIRS.iter() {
            let xx = x as isize + dx;
            let yy = y as isize + dy;
            if xx >= 0 && yy >= 0 && xx < BOARD_SIZE as isize && yy < BOARD_SIZE as isize {
                table[move_index][count] = (yy as usize * BOARD_SIZE + xx as usize) as Move;
                count += 1;
            }
        }
    }
    table
});

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub move_: Move,
    pub order_score: f64,
    pub self_attack: i32,
    pub opp_attack: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateGenerationResult {
    pub candidates: Vec<Candidate>,
    pub single_forcing: bool,
    pub hostile_threat: bool,
    pub win_priority: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateDiagnostic {
    pub move_: Move,
    pub move_value: f64,
    pub adjusted_move_value: f64,
    pub self_attack: i32,
    pub opp_attack: i32,
    pub requires_full_detector: bool,
    pub forbidden_kind: Option<ForbiddenKind>,
    pub rule_legal: bool,
    pub retained: bool,
    pub order_score: Option<f64>,
    pub final_rank: Option<usize>,
    pub rejection_reason: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandidateDiagnosticsResult {
    pub covered_count: usize,
    pub single_forcing: bool,
    pub hostile_threat: bool,
    pub win_priority: bool,
    pub points: Vec<CandidateDiagnostic>,
    pub ordered_candidates: Vec<Candidate>,
}

pub fn movegen_backend_name() -> &'static str {
    "python"
}

#[derive(Clone, Debug)]
struct CoveredMovesBuf {
    moves: [Move; BOARD_AREA],
    len: usize,
}

struct MoveAnalysis {
    vbw_map: [f64; BOARD_AREA],
    self_attack_map: [i32; BOARD_AREA],
    opp_attack_map: [i32; BOARD_AREA],
    sglflag: i32,
    hsflag: Option<Move>,
    winpri: bool,
}

impl MoveAnalysis {
    fn new() -> Self {
        Self {
            vbw_map: [0.0; BOARD_AREA],
            self_attack_map: [0; BOARD_AREA],
            opp_attack_map: [0; BOARD_AREA],
            sglflag: 0,
            hsflag: None,
            winpri: false,
        }
    }
}

impl Default for CoveredMovesBuf {
    fn default() -> Self {
        Self {
            moves: [0; BOARD_AREA],
            len: 0,
        }
    }
}

impl CoveredMovesBuf {
    fn push(&mut self, move_: Move) {
        self.moves[self.len] = move_;
        self.len += 1;
    }

    fn as_slice(&self) -> &[Move] {
        &self.moves[..self.len]
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CoverageTracker {
    counts: [u16; BOARD_AREA],
}

impl CoverageTracker {
    pub(crate) fn from_board(board: &Board) -> Self {
        let mut tracker = Self {
            counts: [0; BOARD_AREA],
        };
        for played in board.move_history() {
            tracker.add_move(played.move_);
        }
        tracker
    }

    pub(crate) fn add_move(&mut self, move_: Move) {
        for &candidate in COVER_NEIGHBORS[move_ as usize].iter() {
            if candidate == COVER_SENTINEL {
                break;
            }
            self.counts[candidate as usize] += 1;
        }
    }

    pub(crate) fn remove_move(&mut self, move_: Move) {
        for &candidate in COVER_NEIGHBORS[move_ as usize].iter() {
            if candidate == COVER_SENTINEL {
                break;
            }
            let count = &mut self.counts[candidate as usize];
            debug_assert!(*count > 0, "coverage remove must match add");
            *count -= 1;
        }
    }

    fn collect_moves(&self, board: &Board, out: &mut CoveredMovesBuf) {
        out.len = 0;
        if board.move_count() == 0 {
            out.push(xy_to_move(BOARD_SIZE / 2, BOARD_SIZE / 2).expect("center move is valid"));
            return;
        }
        let grid = board.grid_rows();
        for move_index in 0..BOARD_AREA {
            if self.counts[move_index] == 0 {
                continue;
            }
            if grid[move_index / BOARD_SIZE][move_index % BOARD_SIZE] == EMPTY {
                out.push(move_index as Move);
            }
        }
    }
}

fn collect_covered_moves(board: &Board, out: &mut CoveredMovesBuf) {
    out.len = 0;
    if board.move_count() == 0 {
        out.push(xy_to_move(BOARD_SIZE / 2, BOARD_SIZE / 2).expect("center move is valid"));
        return;
    }

    let mut seen = [false; BOARD_AREA];
    let grid = board.grid_rows();
    for played in board.move_history() {
        for &candidate in COVER_NEIGHBORS[played.move_ as usize].iter() {
            if candidate == COVER_SENTINEL {
                break;
            }
            let index = candidate as usize;
            if !seen[index] && grid[index / BOARD_SIZE][index % BOARD_SIZE] == EMPTY {
                seen[index] = true;
            }
        }
    }
    for (move_index, is_seen) in seen.into_iter().enumerate() {
        if is_seen {
            out.push(move_index as Move);
        }
    }
}

pub fn covered_moves(board: &Board) -> Vec<Move> {
    let mut moves = CoveredMovesBuf::default();
    collect_covered_moves(board, &mut moves);
    moves.as_slice().to_vec()
}

pub fn apply_hostile_three_extension(
    board: &Board,
    move_: Move,
    side: i8,
    vbw_map: &mut [f64; BOARD_AREA],
) {
    let (x, y) = move_to_xy(move_).expect("move is valid");
    let hostile_side = -side;
    let line_specs = [
        (
            Line::from_board(board, x, HORIZONTAL).expect("direction valid"),
            y,
            1_usize,
        ),
        (
            Line::from_board(board, y, VERTICAL).expect("direction valid"),
            x,
            2_usize,
        ),
        (
            Line::from_board(board, x + y, DIAGONAL_DOWN).expect("direction valid"),
            y,
            3_usize,
        ),
        (
            Line::from_board(board, BOARD_SIZE - 1 - y + x, DIAGONAL_UP).expect("direction valid"),
            BOARD_SIZE - 1 - y,
            4_usize,
        ),
    ];

    let mut encoded = 0_i32;
    let mut direction = 0_usize;
    for (line, point_index, direction_id) in line_specs {
        if point_index + 1 < BOARD_SIZE && line.cells[point_index + 3] == i32::from(hostile_side) {
            encoded = line.a3pb(point_index + 1);
        } else if point_index >= 1 && line.cells[point_index + 1] == i32::from(hostile_side) {
            encoded = line.a3pb(point_index - 1);
        }
        if encoded > 0 {
            direction = direction_id;
            break;
        }
    }
    if direction == 0 {
        return;
    }

    for target in decode_bonus_targets(move_, direction, encoded) {
        vbw_map[target as usize] += 10_000.0;
    }
}

pub fn generate_candidates(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
    wide: Option<usize>,
    root_allowed_moves: Option<&HashSet<Move>>,
    preferred_move: Option<Move>,
    preserve_scan_order: bool,
) -> CandidateGenerationResult {
    let mut moves_buf = CoveredMovesBuf::default();
    collect_covered_moves(board, &mut moves_buf);
    generate_candidates_from_moves(
        board,
        caches,
        side,
        config,
        wide,
        root_allowed_moves,
        preferred_move,
        preserve_scan_order,
        true,
        moves_buf.as_slice(),
    )
}

pub fn diagnose_candidates(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
) -> CandidateDiagnosticsResult {
    let mut moves_buf = CoveredMovesBuf::default();
    collect_covered_moves(board, &mut moves_buf);
    let moves = moves_buf.as_slice();
    let mut analysis = MoveAnalysis::new();
    analyze_moves(board, caches, side, config, moves, &mut analysis);
    let generated = select_candidates(
        board, caches, side, config, None, None, None, false, true, moves, &analysis,
    );
    let ordered_candidates =
        crate::search::ordering::order_candidates(board, &generated.candidates, side, None);
    let mut ranks = [usize::MAX; BOARD_AREA];
    for (rank, candidate) in ordered_candidates.iter().enumerate() {
        ranks[candidate.move_ as usize] = rank;
    }

    let mut points = Vec::with_capacity(moves.len());
    for &move_ in moves {
        let move_index = move_ as usize;
        let (x, y) = move_to_xy(move_).expect("covered move is valid");
        let requires_full_detector = rule_detector_required(caches, x, y, side, config.rule_set);
        let forbidden_kind = if requires_full_detector
            && config.rule_set == RuleSet::Renju
            && side == crate::constants::BLACK
        {
            classify_forbidden_move(board, move_, side, config.rule_set).ok()
        } else {
            None
        };
        let rule_legal = is_rule_legal_for_movegen(board, caches, move_, side, config.rule_set);
        let adjusted_move_value = adjusted_move_value(&analysis, move_);
        let final_rank = (ranks[move_index] != usize::MAX).then_some(ranks[move_index]);
        let retained = final_rank.is_some();
        let rejection_reason = if !rule_legal {
            Some(if forbidden_kind.is_some_and(ForbiddenKind::is_forbidden) {
                "forbidden"
            } else {
                "illegal"
            })
        } else if adjusted_move_value <= 0.0 {
            Some("non_positive_value")
        } else if !retained && (generated.win_priority || generated.single_forcing) {
            Some("forcing_collapse")
        } else if !retained {
            Some("candidate_filter")
        } else {
            None
        };

        points.push(CandidateDiagnostic {
            move_,
            move_value: analysis.vbw_map[move_index],
            adjusted_move_value,
            self_attack: analysis.self_attack_map[move_index],
            opp_attack: analysis.opp_attack_map[move_index],
            requires_full_detector,
            forbidden_kind,
            rule_legal,
            retained,
            order_score: retained.then_some(adjusted_move_value - 300_000_000.0),
            final_rank,
            rejection_reason,
        });
    }

    CandidateDiagnosticsResult {
        covered_count: moves.len(),
        single_forcing: generated.single_forcing,
        hostile_threat: generated.hostile_threat,
        win_priority: generated.win_priority,
        points,
        ordered_candidates,
    }
}

pub(crate) fn generate_candidates_with_coverage(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
    wide: Option<usize>,
    root_allowed_moves: Option<&HashSet<Move>>,
    preferred_move: Option<Move>,
    preserve_scan_order: bool,
    coverage: &CoverageTracker,
) -> CandidateGenerationResult {
    let mut moves_buf = CoveredMovesBuf::default();
    coverage.collect_moves(board, &mut moves_buf);
    generate_candidates_from_moves(
        board,
        caches,
        side,
        config,
        wide,
        root_allowed_moves,
        preferred_move,
        preserve_scan_order,
        false,
        moves_buf.as_slice(),
    )
}

fn generate_candidates_from_moves(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
    wide: Option<usize>,
    root_allowed_moves: Option<&HashSet<Move>>,
    preferred_move: Option<Move>,
    preserve_scan_order: bool,
    presort_for_forcing: bool,
    moves: &[Move],
) -> CandidateGenerationResult {
    let mut analysis = MoveAnalysis::new();
    analyze_moves(board, caches, side, config, moves, &mut analysis);
    select_candidates(
        board,
        caches,
        side,
        config,
        wide,
        root_allowed_moves,
        preferred_move,
        preserve_scan_order,
        presort_for_forcing,
        moves,
        &analysis,
    )
}

fn analyze_moves(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
    moves: &[Move],
    analysis: &mut MoveAnalysis,
) {
    let mut at1pri = 0_i32;
    let mut at2pri = 0_i32;

    for &move_ in moves {
        let (x, y) = move_to_xy(move_).expect("covered move is valid");
        let vbw = move_value(caches, x, y, side, config).trunc();
        let att1 = attack_level(caches, x, y, side);
        let att2 = attack_level(caches, x, y, -side);
        let move_index = move_ as usize;
        analysis.vbw_map[move_index] = vbw;
        analysis.self_attack_map[move_index] = att1;
        analysis.opp_attack_map[move_index] = att2;

        if vbw <= 0.0 {
            at2pri = at2pri.max(att2);
            continue;
        }
        if att2 == 6 || att1 >= 5 {
            analysis.sglflag += 1;
        } else if att2 == 5 {
            analysis.hsflag = Some(move_);
        }
        at1pri = at1pri.max(att1);
        at2pri = at2pri.max(att2);
    }

    analysis.winpri = at1pri == 6 || (at1pri == 5 && at2pri <= 5);
    if analysis.sglflag == 0 {
        if let Some(hostile_move) = analysis.hsflag {
            apply_hostile_three_extension(board, hostile_move, side, &mut analysis.vbw_map);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn select_candidates(
    board: &Board,
    caches: &EvalCaches,
    side: i8,
    config: &EngineConfig,
    wide: Option<usize>,
    root_allowed_moves: Option<&HashSet<Move>>,
    preferred_move: Option<Move>,
    preserve_scan_order: bool,
    presort_for_forcing: bool,
    moves: &[Move],
    analysis: &MoveAnalysis,
) -> CandidateGenerationResult {
    let _wide = wide.unwrap_or(config.root_search.wide as usize);
    let mut candidates = Vec::with_capacity(moves.len());
    for &move_ in moves {
        if !is_rule_legal_for_movegen(board, caches, move_, side, config.rule_set) {
            continue;
        }
        if let Some(allowed) = root_allowed_moves {
            if !allowed.contains(&move_) {
                continue;
            }
        }

        let move_index = move_ as usize;
        let vbw = adjusted_move_value(analysis, move_);
        if vbw <= 0.0 {
            continue;
        }

        let mut score = vbw - 300_000_000.0;
        if preferred_move == Some(move_) {
            score = 100.0;
        }
        let candidate = Candidate {
            move_,
            order_score: score,
            self_attack: analysis.self_attack_map[move_index],
            opp_attack: analysis.opp_attack_map[move_index],
        };
        if score >= f64::from(WIN) {
            candidates = vec![candidate];
            break;
        }
        if score <= -f64::from(WIN) && score >= -200_000_000.0 {
            continue;
        }
        candidates.push(candidate);
    }

    if !preserve_scan_order && presort_for_forcing {
        candidates.sort_unstable_by(|a, b| {
            b.order_score
                .partial_cmp(&a.order_score)
                .unwrap()
                .then_with(|| a.move_.cmp(&b.move_))
        });
    }

    if analysis.winpri && !candidates.is_empty() {
        let candidate = best_forcing_candidate(&candidates, preserve_scan_order);
        return CandidateGenerationResult {
            candidates: vec![candidate],
            single_forcing: false,
            hostile_threat: analysis.hsflag.is_some(),
            win_priority: true,
        };
    }
    if analysis.sglflag > 0 && !candidates.is_empty() {
        let candidate = best_forcing_candidate(&candidates, preserve_scan_order);
        return CandidateGenerationResult {
            candidates: vec![candidate],
            single_forcing: true,
            hostile_threat: analysis.hsflag.is_some(),
            win_priority: analysis.winpri,
        };
    }

    CandidateGenerationResult {
        candidates,
        single_forcing: false,
        hostile_threat: analysis.hsflag.is_some(),
        win_priority: analysis.winpri,
    }
}

fn adjusted_move_value(analysis: &MoveAnalysis, move_: Move) -> f64 {
    let move_index = move_ as usize;
    let mut vbw = analysis.vbw_map[move_index];
    if analysis.hsflag.is_some() {
        vbw -= 5000.0;
        if analysis.self_attack_map[move_index] >= 4 {
            vbw += 8000.0;
        }
    }
    vbw
}

fn rule_detector_required(
    caches: &EvalCaches,
    x: usize,
    y: usize,
    side: i8,
    rule: RuleSet,
) -> bool {
    if rule != RuleSet::Renju || side != crate::constants::BLACK {
        return false;
    }
    if caches.rule_set != rule || !caches.initialized {
        return true;
    }
    renju_black_candidate_needs_full_detector(caches, x, y)
}

fn is_rule_legal_for_movegen(
    board: &Board,
    caches: &EvalCaches,
    move_: Move,
    side: i8,
    rule: RuleSet,
) -> bool {
    if rule == RuleSet::Freestyle {
        return board.is_legal_move(move_);
    }
    if side != crate::constants::BLACK {
        return board.is_legal_move_for_rule(move_, side, rule);
    }
    if !board.is_legal_move(move_) {
        return false;
    }
    let (x, y) = move_to_xy(move_).expect("candidate move is valid");
    if rule_detector_required(caches, x, y, side, rule) {
        return board.is_legal_move_for_rule(move_, side, rule);
    }
    true
}

fn renju_black_candidate_needs_full_detector(caches: &EvalCaches, x: usize, y: usize) -> bool {
    let mut open_threes = 0;
    let mut fours = 0;
    let mut overlines = 0;
    let mut exact_fives = 0;

    for &shape in &caches.shape_cache[0][x][y] {
        let label = (shape >> 16) & 0xF;
        let aux = shape & 0xF;
        if label == ShapeLabel::L3 as i32 || label == ShapeLabel::L3B as i32 {
            open_threes += 1;
        } else if label == ShapeLabel::L4S as i32 {
            fours += aux;
        } else if label == ShapeLabel::L5 as i32 {
            exact_fives += 1;
        } else if label == ShapeLabel::L4 as i32 {
            fours += 1;
        } else if label == ShapeLabel::L6 as i32 {
            overlines += 1;
        }
    }

    if exact_fives > 0 {
        return false;
    }
    overlines > 0 || fours >= 2 || open_threes >= 2
}

fn best_forcing_candidate(candidates: &[Candidate], preserve_scan_order: bool) -> Candidate {
    if preserve_scan_order {
        return candidates[0];
    }
    candidates
        .iter()
        .copied()
        .max_by(|a, b| {
            a.order_score
                .partial_cmp(&b.order_score)
                .expect("candidate scores are finite")
                .then_with(|| b.move_.cmp(&a.move_))
        })
        .expect("caller checked non-empty candidates")
}

#[cfg(test)]
mod tests {
    use super::*;

    // The Phase 1/2 forbidden fixtures classify a single black candidate. Here
    // we reuse them as a board+movegen integration regression: the rule-aware
    // legality gate that movegen/search rely on must agree with each fixture's
    // expected classification, and Renju movegen must never emit a forbidden
    // black candidate that freestyle would keep.
    #[test]
    fn forbidden_fixtures_drive_rule_aware_legality_and_movegen() {
        use crate::config::load_default_config;
        use crate::constants::{BLACK, WHITE};
        use crate::eval::recompute_all_for_rule;
        use crate::rules::RuleSet;
        use std::collections::HashSet;

        #[derive(serde::Deserialize)]
        struct Stone {
            x: usize,
            y: usize,
            side: i8,
        }
        #[derive(serde::Deserialize)]
        struct Point {
            x: usize,
            y: usize,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            name: String,
            moves: Vec<Stone>,
            candidate: Point,
            expected: String,
        }

        let raw = include_str!("../../cases/renju/forbidden_hand_cases.jsonl");
        let mut freestyle_config = load_default_config();
        freestyle_config.rule_set = RuleSet::Freestyle;
        let mut renju_config = load_default_config();
        renju_config.rule_set = RuleSet::Renju;

        let mut checked = 0usize;
        let mut movegen_filtered = 0usize;
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let case: Fixture = serde_json::from_str(line)
                .unwrap_or_else(|err| panic!("fixture parse failed: {err}"));

            let mut board = Board::new();
            for stone in &case.moves {
                board.grid_rows_mut()[stone.y][stone.x] = stone.side;
            }
            let candidate = xy_to_move(case.candidate.x, case.candidate.y).unwrap();
            assert_eq!(
                board.at(case.candidate.x, case.candidate.y),
                Ok(EMPTY),
                "{}: candidate must be empty",
                case.name
            );

            let forbidden = case.expected != "none";

            // The legality gate must match the fixture for black under Renju,
            // and never forbid in freestyle or for white.
            assert_eq!(
                board.is_legal_move_for_rule(candidate, BLACK, RuleSet::Renju),
                !forbidden,
                "{}: renju black legality disagrees with expected `{}`",
                case.name,
                case.expected
            );
            assert!(
                board.is_legal_move_for_rule(candidate, BLACK, RuleSet::Freestyle),
                "{}: freestyle must never forbid",
                case.name
            );
            assert!(
                board.is_legal_move_for_rule(candidate, WHITE, RuleSet::Renju),
                "{}: white has no forbidden moves",
                case.name
            );

            // Movegen: when freestyle would emit the candidate (it is covered by
            // a neighbouring stone), Renju movegen must drop it iff forbidden.
            let allowed: HashSet<Move> = [candidate].into_iter().collect();
            let emits = |config: &EngineConfig| {
                let mut board_for_cache = board.clone();
                let mut caches = EvalCaches::new();
                recompute_all_for_rule(&mut board_for_cache, &mut caches, config.rule_set);
                generate_candidates(
                    &board_for_cache,
                    &caches,
                    BLACK,
                    config,
                    None,
                    Some(&allowed),
                    None,
                    false,
                )
                .candidates
                .iter()
                .any(|c| c.move_ == candidate)
            };
            if emits(&freestyle_config) {
                let renju_emits = emits(&renju_config);
                assert_eq!(
                    renju_emits, !forbidden,
                    "{}: renju movegen filtering disagrees with expected `{}`",
                    case.name, case.expected
                );
                if forbidden && !renju_emits {
                    movegen_filtered += 1;
                }
            }

            checked += 1;
        }
        assert!(
            checked >= 60,
            "expected to check the full fixture set, got {checked}"
        );
        assert!(
            movegen_filtered >= 10,
            "movegen filtering branch barely exercised ({movegen_filtered}); test may be vacuous"
        );
    }

    #[test]
    fn coverage_tracker_matches_covered_moves_after_play_and_undo() {
        let mut board = Board::new();
        let mut tracker = CoverageTracker::from_board(&board);
        let mut buf = CoveredMovesBuf::default();
        tracker.collect_moves(&board, &mut buf);
        assert_eq!(buf.as_slice(), covered_moves(&board).as_slice());

        for (x, y) in [(7, 7), (8, 7), (7, 8), (8, 8), (6, 7)] {
            let move_ = xy_to_move(x, y).unwrap();
            board.play(move_, None).unwrap();
            tracker.add_move(move_);
            tracker.collect_moves(&board, &mut buf);
            assert_eq!(buf.as_slice(), covered_moves(&board).as_slice());
        }

        while let Some(played) = board.undo().ok() {
            tracker.remove_move(played.move_);
            tracker.collect_moves(&board, &mut buf);
            assert_eq!(buf.as_slice(), covered_moves(&board).as_slice());
        }
    }

    #[test]
    fn renju_candidate_diagnostics_match_incremental_and_full_caches() {
        use crate::config::load_default_config;
        use crate::constants::{BLACK, WHITE};
        use crate::eval::{recompute_all_for_rule, value_wide_compute_for_rule};

        let mut config = load_default_config();
        config.rule_set = RuleSet::Renju;
        let mut board = Board::new();
        let mut incremental = EvalCaches::new();
        recompute_all_for_rule(&mut board, &mut incremental, RuleSet::Renju);

        for (x, y, side) in [
            (6, 7, BLACK),
            (0, 0, WHITE),
            (8, 7, BLACK),
            (0, 1, WHITE),
            (7, 6, BLACK),
            (0, 2, WHITE),
            (7, 8, BLACK),
            (0, 3, WHITE),
        ] {
            board
                .play_for_rule(xy_to_move(x, y).unwrap(), Some(side), RuleSet::Renju)
                .unwrap();
            value_wide_compute_for_rule(&mut board, &mut incremental, (x, y), RuleSet::Renju);

            let mut full_board = board.clone();
            let mut full = EvalCaches::new();
            recompute_all_for_rule(&mut full_board, &mut full, RuleSet::Renju);
            assert_eq!(
                diagnose_candidates(&board, &incremental, board.side_to_move(), &config),
                diagnose_candidates(&full_board, &full, full_board.side_to_move(), &config)
            );
        }

        let center = xy_to_move(7, 7).unwrap();
        let diagnostics = diagnose_candidates(&board, &incremental, BLACK, &config);
        let point = diagnostics
            .points
            .iter()
            .find(|point| point.move_ == center)
            .expect("double-three point is covered");
        assert!(point.requires_full_detector);
        assert_eq!(point.forbidden_kind, Some(ForbiddenKind::DoubleThree));
        assert!(!point.rule_legal);
        assert_eq!(point.rejection_reason, Some("forbidden"));
    }

    #[test]
    fn best_forcing_candidate_matches_presort_order() {
        let candidates = [
            Candidate {
                move_: 12,
                order_score: 3.0,
                self_attack: 0,
                opp_attack: 0,
            },
            Candidate {
                move_: 7,
                order_score: 5.0,
                self_attack: 0,
                opp_attack: 0,
            },
            Candidate {
                move_: 3,
                order_score: 5.0,
                self_attack: 0,
                opp_attack: 0,
            },
        ];
        assert_eq!(best_forcing_candidate(&candidates, false).move_, 3);
        assert_eq!(best_forcing_candidate(&candidates, true).move_, 12);
    }
}

fn decode_bonus_targets(move_: Move, direction_index: usize, encoded: i32) -> Vec<Move> {
    let (x, y) = move_to_xy(move_).expect("move is valid");
    let mut raw = vec![ga(encoded), gb(encoded)];
    if encoded >= (1 << 24) {
        raw.push(gc(encoded));
    }

    let mut targets = Vec::new();
    for value in raw {
        let (tx, ty) = match direction_index {
            1 => (x as isize, value as isize),
            2 => (value as isize, y as isize),
            3 => (x as isize + y as isize - value as isize, value as isize),
            4 => (
                BOARD_SIZE as isize - 1 + x as isize - y as isize - value as isize,
                BOARD_SIZE as isize - 1 - value as isize,
            ),
            _ => continue,
        };
        if tx >= 0 && ty >= 0 && tx < BOARD_SIZE as isize && ty < BOARD_SIZE as isize {
            targets.push(
                xy_to_move(tx as usize, ty as usize)
                    .expect("decoded hostile bonus target is valid"),
            );
        }
    }
    targets
}

fn ga(value: i32) -> i32 {
    value & 0xFF
}

fn gb(value: i32) -> i32 {
    (value >> 8) & 0xFF
}

fn gc(value: i32) -> i32 {
    (value >> 16) & 0xFF
}
