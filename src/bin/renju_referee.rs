//! Line-oriented JSON referee for external Renju engine matches.

use std::io::{self, BufRead};

use rust_gomoku::{
    classify_forbidden_move, xy_to_move, Board, ForbiddenKind, RuleSet, BLACK, EMPTY,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct RefereeRequest {
    moves: Vec<[usize; 2]>,
    candidate: [usize; 2],
    side: i8,
}

#[derive(Serialize)]
struct RefereeResponse {
    legal: bool,
    forbidden_kind: Option<&'static str>,
    winner: i8,
    error: Option<String>,
}

fn forbidden_name(kind: ForbiddenKind) -> &'static str {
    match kind {
        ForbiddenKind::None => "none",
        ForbiddenKind::DoubleThree => "double_three",
        ForbiddenKind::DoubleFour => "double_four",
        ForbiddenKind::Overline => "overline",
    }
}

fn judge(request: RefereeRequest) -> RefereeResponse {
    let mut board = Board::new();
    for (index, [x, y]) in request.moves.into_iter().enumerate() {
        let side = if index % 2 == 0 { BLACK } else { -BLACK };
        let Ok(move_) = xy_to_move(x, y) else {
            return RefereeResponse {
                legal: false,
                forbidden_kind: None,
                winner: EMPTY,
                error: Some(format!("prefix move {} is out of bounds", index + 1)),
            };
        };
        if let Err(err) = board.play_for_rule(move_, Some(side), RuleSet::Renju) {
            return RefereeResponse {
                legal: false,
                forbidden_kind: None,
                winner: EMPTY,
                error: Some(format!("prefix move {} is illegal: {err:?}", index + 1)),
            };
        }
    }
    if request.side != board.side_to_move() {
        return RefereeResponse {
            legal: false,
            forbidden_kind: None,
            winner: EMPTY,
            error: Some("candidate side does not match alternating order".to_string()),
        };
    }

    let Ok(candidate) = xy_to_move(request.candidate[0], request.candidate[1]) else {
        return RefereeResponse {
            legal: false,
            forbidden_kind: None,
            winner: EMPTY,
            error: Some("candidate is out of bounds".to_string()),
        };
    };
    let kind = classify_forbidden_move(&board, candidate, request.side, RuleSet::Renju);
    let Ok(kind) = kind else {
        return RefereeResponse {
            legal: false,
            forbidden_kind: None,
            winner: EMPTY,
            error: Some("candidate is occupied or invalid".to_string()),
        };
    };
    if kind.is_forbidden() {
        return RefereeResponse {
            legal: false,
            forbidden_kind: Some(forbidden_name(kind)),
            winner: -request.side,
            error: None,
        };
    }
    if let Err(err) = board.play_for_rule(candidate, Some(request.side), RuleSet::Renju) {
        return RefereeResponse {
            legal: false,
            forbidden_kind: None,
            winner: EMPTY,
            error: Some(format!("candidate play failed: {err:?}")),
        };
    }
    RefereeResponse {
        legal: true,
        forbidden_kind: Some("none"),
        winner: board.winner(),
        error: None,
    }
}

fn main() {
    for line in io::stdin().lock().lines() {
        let response = match line {
            Ok(line) => match serde_json::from_str::<RefereeRequest>(&line) {
                Ok(request) => judge(request),
                Err(err) => RefereeResponse {
                    legal: false,
                    forbidden_kind: None,
                    winner: EMPTY,
                    error: Some(format!("invalid request: {err}")),
                },
            },
            Err(err) => RefereeResponse {
                legal: false,
                forbidden_kind: None,
                winner: EMPTY,
                error: Some(format!("stdin error: {err}")),
            },
        };
        println!(
            "{}",
            serde_json::to_string(&response).expect("referee response serializes")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(moves: &[[usize; 2]], candidate: [usize; 2], side: i8) -> RefereeRequest {
        RefereeRequest {
            moves: moves.to_vec(),
            candidate,
            side,
        }
    }

    #[test]
    fn black_exact_five_wins_but_overline_loses() {
        let exact_five = judge(request(
            &[
                [3, 7],
                [0, 0],
                [4, 7],
                [0, 1],
                [5, 7],
                [0, 2],
                [6, 7],
                [0, 3],
            ],
            [7, 7],
            BLACK,
        ));
        assert!(exact_five.legal);
        assert_eq!(exact_five.winner, BLACK);

        let overline = judge(request(
            &[
                [2, 7],
                [0, 0],
                [3, 7],
                [0, 1],
                [4, 7],
                [0, 2],
                [6, 7],
                [0, 3],
                [7, 7],
                [0, 4],
            ],
            [5, 7],
            BLACK,
        ));
        assert!(!overline.legal);
        assert_eq!(overline.forbidden_kind, Some("overline"));
        assert_eq!(overline.winner, -BLACK);
    }

    #[test]
    fn cross_double_three_is_illegal_and_white_overline_wins() {
        let double_three = judge(request(
            &[
                [6, 7],
                [0, 0],
                [8, 7],
                [0, 1],
                [7, 6],
                [0, 2],
                [7, 8],
                [0, 3],
            ],
            [7, 7],
            BLACK,
        ));
        assert!(!double_three.legal);
        assert_eq!(double_three.forbidden_kind, Some("double_three"));

        let white_overline = judge(request(
            &[
                [0, 0],
                [2, 7],
                [0, 1],
                [3, 7],
                [0, 2],
                [4, 7],
                [0, 3],
                [6, 7],
                [1, 0],
                [7, 7],
                [1, 1],
            ],
            [5, 7],
            -BLACK,
        ));
        assert!(white_overline.legal);
        assert_eq!(white_overline.winner, -BLACK);
    }
}
