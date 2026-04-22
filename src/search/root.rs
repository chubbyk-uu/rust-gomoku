//! Root iterative-deepening search for the classic mainline.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::board::{move_to_xy, xy_to_move, Board};
use crate::config::EngineConfig;
use crate::constants::{INF, WIN};
use crate::eval::{recompute_all, EvalCaches};
use crate::search::{AlphaBetaSearcher, SearchOptions, SearchStats, TranspositionTable};
use crate::threats::VCFSearcher;
use crate::types::{Move, Side};

const CLASSIC_RAND_SEED: i32 = 1_232_356;
const CLASSIC_FALLBACK_STATE: [u32; 31] = [
    3344391599, 3760159923, 229790648, 3328593876, 529145457, 4021946065, 1735816513, 469166854,
    1730624144, 2908500504, 649120694, 3012569930, 473519764, 1775465023, 936985512, 994684877,
    4231614135, 825016603, 3651181685, 2927649197, 4259523512, 1063826198, 2094918629, 2306226027,
    4013509952, 563982589, 367722354, 742065300, 1591101748, 477268195, 3720283884,
];
const CLASSIC_FALLBACK_FPTR: usize = 25;
const CLASSIC_FALLBACK_RPTR: usize = 22;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct SearchLimits {
    pub max_depth: i32,
    pub root_width: usize,
    pub node_limit: Option<usize>,
    pub time_limit_ms: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchResult {
    pub move_: Move,
    pub score: i32,
    pub depth: i32,
    pub nodes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct VctSearchResult {
    pub move_: Option<Move>,
    pub found: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NullVctSearcher;

impl NullVctSearcher {
    pub fn search(&mut self, _board: &Board, _side: Side, _depth: i32) -> VctSearchResult {
        VctSearchResult::default()
    }
}

#[derive(Clone, Debug)]
pub struct ClassicFallbackRng {
    state: [u32; 31],
    fptr: usize,
    rptr: usize,
}

impl ClassicFallbackRng {
    pub fn new(seed: i32) -> Result<Self, &'static str> {
        if seed != CLASSIC_RAND_SEED {
            return Err("only the classic fallback seed is supported");
        }
        Ok(Self {
            state: CLASSIC_FALLBACK_STATE,
            fptr: CLASSIC_FALLBACK_FPTR,
            rptr: CLASSIC_FALLBACK_RPTR,
        })
    }

    pub fn randrange(&mut self, upper: usize) -> Result<usize, &'static str> {
        if upper == 0 {
            return Err("upper must be positive");
        }
        let value = self.state[self.fptr].wrapping_add(self.state[self.rptr]);
        self.state[self.fptr] = value;
        self.fptr = (self.fptr + 1) % self.state.len();
        self.rptr = (self.rptr + 1) % self.state.len();
        Ok((((value >> 1) & 0x7FFF_FFFF) as usize) % upper)
    }
}

pub fn new_classic_fallback_rng() -> ClassicFallbackRng {
    ClassicFallbackRng::new(CLASSIC_RAND_SEED).expect("classic fallback seed stays supported")
}

fn shape_label(shape: i32) -> i32 {
    (shape >> 16) & 0xF
}

fn shape_aux(shape: i32) -> i32 {
    shape & 0xF
}

pub fn fallback_ai_move(
    board: &Board,
    caches: &EvalCaches,
    side: Side,
    rng: &mut ClassicFallbackRng,
) -> Result<Move, &'static str> {
    let player = if side == 1 { 0 } else { 1 };
    let opponent = 1 - player;
    let mut best_value = i64::MIN;
    let mut best_moves = Vec::new();

    for move_index in 0..(board.size() * board.size()) {
        let move_ = move_index as Move;
        if !board.is_legal_move(move_) {
            continue;
        }
        let (x, y) = move_to_xy(move_).expect("iterated move index stays in range");

        let mut offensive = 0_i64;
        let (mut a1l, mut b2l, mut a2l, mut b3l, mut a4l, mut a3l, mut b4l, mut a5l, mut a6l) = (
            0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64,
        );
        for direction in 0..4 {
            let shape = caches.shape_cache[player][x][y][direction];
            let label = shape_label(shape);
            if label == 2 {
                a1l += 1;
            } else if label == 3 {
                b2l += 1;
            } else if label == 9 || label == 8 {
                a3l += 1;
            } else if label == 10 {
                b4l += i64::from(shape_aux(shape));
            } else if label == 12 {
                a5l += 1;
            } else if label == 7 {
                b3l += 1;
            } else if (4..=6).contains(&label) {
                a2l += 1;
            } else if label == 11 {
                a4l += 1;
                b4l += 1;
            } else if label == 13 {
                a6l += 1;
            }
        }
        offensive += a1l;
        offensive += b2l;
        offensive += a2l * 5;
        offensive += b3l * 10;
        offensive += a3l * 12;
        offensive += b4l * 16;
        offensive += i64::from(a3l >= 2) * 100;
        offensive += i64::from(b4l > 0 && a3l > 0) * 3000;
        offensive += i64::from(b4l >= 2) * 4000;
        offensive += a4l * 6000;
        offensive += a5l * 1_000_000;
        let _ = a6l;

        let mut defensive = 0_i64;
        let (mut a2l, mut b3l, mut a3l, mut b4l, mut a4l, mut a5l, mut a6l) =
            (0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64, 0_i64);
        for direction in 0..4 {
            let shape = caches.shape_cache[opponent][x][y][direction];
            let label = shape_label(shape);
            if label == 9 || label == 8 {
                a3l += 1;
            } else if label == 10 {
                b4l += i64::from(shape_aux(shape));
            } else if label == 12 {
                a5l += 1;
            } else if label == 7 {
                b3l += 1;
            } else if (4..=6).contains(&label) {
                a2l += 1;
            } else if label == 11 {
                a4l += 1;
                b4l += 1;
            } else if label == 13 {
                a6l += 1;
            }
        }
        defensive += a2l;
        defensive += b3l;
        defensive += a3l * 6;
        defensive += b4l * 11;
        defensive += i64::from(a3l >= 2) * 15;
        defensive += i64::from(b4l > 0 && a3l > 0) * 1500;
        defensive += i64::from(b4l >= 2) * 2000;
        defensive += a4l * 3000;
        defensive += a5l * 50_000;
        let _ = a6l;

        let total = 5 * offensive + 5 * defensive;
        if total > best_value {
            best_value = total;
            best_moves.clear();
            best_moves.push(move_);
        } else if total == best_value {
            best_moves.push(move_);
        }
    }

    if best_moves.is_empty() {
        return Err("fallback AIs found no legal move on non-terminal board");
    }
    Ok(best_moves[rng.randrange(best_moves.len())?])
}

#[derive(Clone, Debug)]
pub struct RootSearcher {
    pub config: EngineConfig,
    pub tt: TranspositionTable,
    pub alphabeta: AlphaBetaSearcher,
    pub vcf: VCFSearcher,
    pub vct: NullVctSearcher,
    pub fallback_rng: ClassicFallbackRng,
}

impl RootSearcher {
    pub fn new(config: EngineConfig) -> Self {
        let tt = TranspositionTable::default();
        let alphabeta = AlphaBetaSearcher::with_tt(config.clone(), tt.clone());
        Self {
            config,
            tt,
            alphabeta,
            vcf: VCFSearcher::default(),
            vct: NullVctSearcher,
            fallback_rng: new_classic_fallback_rng(),
        }
    }

    pub fn with_tt(config: EngineConfig, tt: TranspositionTable) -> Self {
        let alphabeta = AlphaBetaSearcher::with_tt(config.clone(), tt.clone());
        Self {
            config,
            tt,
            alphabeta,
            vcf: VCFSearcher::default(),
            vct: NullVctSearcher,
            fallback_rng: new_classic_fallback_rng(),
        }
    }

    pub fn root_allowed_moves(&self, board: &Board) -> Option<HashSet<Move>> {
        if self.config.runtime.static_board || board.move_count() == 0 {
            return None;
        }
        let moves = board.occupied_moves();
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for move_ in moves {
            let (x, y) = move_to_xy(move_).expect("occupied move stays in range");
            xs.push(x as i32);
            ys.push(y as i32);
        }
        let margin = self.config.runtime.dynamic_board_margin;
        let mut xmin = 0.max(xs.iter().copied().min()? - margin);
        let mut xmax = (board.size() as i32 - 1).min(xs.iter().copied().max()? + margin);
        let mut ymin = 0.max(ys.iter().copied().min()? - margin);
        let mut ymax = (board.size() as i32 - 1).min(ys.iter().copied().max()? + margin);

        let mut toggle = 0;
        while (xmax - xmin) != (ymax - ymin) {
            toggle += 1;
            if (xmax - xmin) > (ymax - ymin) {
                if toggle % 2 == 1 {
                    if ymin > 0 {
                        ymin -= 1;
                    } else {
                        ymax = (board.size() as i32 - 1).min(ymax + 1);
                    }
                } else if ymax < board.size() as i32 - 1 {
                    ymax += 1;
                } else {
                    ymin = 0.max(ymin - 1);
                }
            } else if toggle % 2 == 1 {
                if xmin > 0 {
                    xmin -= 1;
                } else {
                    xmax = (board.size() as i32 - 1).min(xmax + 1);
                }
            } else if xmax < board.size() as i32 - 1 {
                xmax += 1;
            } else {
                xmin = 0.max(xmin - 1);
            }
        }

        let mut allowed = HashSet::new();
        for y in ymin..=ymax {
            for x in xmin..=xmax {
                if board
                    .at(x as usize, y as usize)
                    .expect("window stays in bounds")
                    == 0
                {
                    allowed.insert(xy_to_move(x as usize, y as usize).expect("window stays valid"));
                }
            }
        }
        Some(allowed)
    }

    pub fn apply_opponent_vcf_filter(
        &mut self,
        board: &Board,
        side: Side,
        allowed_moves: Option<HashSet<Move>>,
    ) -> Option<HashSet<Move>> {
        if !self.config.runtime.compute_vcf {
            return allowed_moves;
        }
        let opponent_vcf = self.vcf.search(board, -side, 7);
        if !opponent_vcf.found {
            return allowed_moves;
        }

        let candidates: Vec<Move> = if let Some(allowed) = allowed_moves {
            let mut moves: Vec<_> = allowed
                .into_iter()
                .filter(|&move_| board.is_legal_move(move_))
                .collect();
            moves.sort_unstable();
            moves
        } else {
            (0..(board.size() * board.size()))
                .map(|index| index as Move)
                .filter(|&move_| board.is_legal_move(move_))
                .collect()
        };

        let mut filtered = HashSet::new();
        for move_ in candidates {
            let mut trial = board.clone();
            trial
                .play(move_, Some(side))
                .expect("candidate move stays legal on trial board");
            if !self.vcf.search(&trial, -side, 7).found {
                filtered.insert(move_);
            }
        }
        Some(filtered)
    }

    pub fn search(&mut self, board: &mut Board, limits: Option<SearchLimits>) -> SearchResult {
        let limits = limits.unwrap_or(SearchLimits {
            max_depth: self.config.root_search.depth,
            root_width: self.config.root_search.wide as usize,
            node_limit: None,
            time_limit_ms: None,
        });

        if board.move_count() == 0 {
            let center =
                xy_to_move(board.size() / 2, board.size() / 2).expect("center stays valid");
            return SearchResult {
                move_: center,
                score: 0,
                depth: 0,
                nodes: 0,
            };
        }

        let side = board.side_to_move();
        let mut caches = EvalCaches::new();
        recompute_all(board, &mut caches);

        if self.config.runtime.compute_vcf {
            let vcf_result = self.vcf.search(board, side, 8);
            if vcf_result.found {
                return SearchResult {
                    move_: vcf_result
                        .move_
                        .expect("successful VCF search returns a move"),
                    score: INF,
                    depth: 0,
                    nodes: 0,
                };
            }
        }

        self.alphabeta.config = self.config.clone();
        self.alphabeta.tt = self.tt.clone();
        let mut best_move = None;
        let mut best_score = -INF;
        let mut total_nodes = 0_usize;
        let deadline = limits
            .time_limit_ms
            .map(|ms| Instant::now() + Duration::from_secs_f64(ms / 1000.0));
        let root_allowed_moves =
            self.apply_opponent_vcf_filter(board, side, self.root_allowed_moves(board));
        if let Some(allowed) = &root_allowed_moves {
            let mut root_legal_moves: Vec<_> = allowed
                .iter()
                .copied()
                .filter(|&move_| board.is_legal_move(move_))
                .collect();
            root_legal_moves.sort_unstable();
            if root_legal_moves.is_empty() {
                let move_ = fallback_ai_move(board, &caches, side, &mut self.fallback_rng)
                    .expect("fallback move exists on non-terminal board");
                return SearchResult {
                    move_,
                    score: -INF,
                    depth: 0,
                    nodes: 0,
                };
            }
            if root_legal_moves.len() == 1 {
                return SearchResult {
                    move_: root_legal_moves[0],
                    score: 0,
                    depth: 0,
                    nodes: 0,
                };
            }
        }

        let mut completed_depth = 0_i32;
        for depth in 1..=limits.max_depth {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                break;
            }
            let mut stats = SearchStats {
                node_limit: limits.node_limit,
                deadline,
                ..SearchStats::default()
            };
            let (score, move_) = self.alphabeta.search(
                board,
                &mut caches,
                side,
                f64::from(depth),
                -INF,
                INF,
                limits.root_width,
                &mut stats,
                SearchOptions {
                    opo: 1,
                    root: true,
                    root_allowed_moves: root_allowed_moves.as_ref(),
                    ..SearchOptions::default()
                },
            );
            total_nodes += stats.nodes;
            self.tt = self.alphabeta.tt.clone();
            if stats.stop {
                break;
            }
            completed_depth = depth;
            if let Some(move_) = move_ {
                best_move = Some(move_);
                best_score = score;
            } else if score <= -WIN {
                best_move = Some(
                    fallback_ai_move(board, &caches, side, &mut self.fallback_rng)
                        .expect("fallback move exists on non-terminal board"),
                );
                best_score = score;
            }
            if score >= WIN || score <= -WIN {
                break;
            }
        }

        let move_ = best_move.unwrap_or_else(|| {
            fallback_ai_move(board, &caches, side, &mut self.fallback_rng)
                .expect("fallback move exists on non-terminal board")
        });
        SearchResult {
            move_,
            score: best_score,
            depth: completed_depth,
            nodes: total_nodes,
        }
    }
}
