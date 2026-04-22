//! Threat-search helpers aligned with the classic reference.

pub mod threat_board;
pub mod types;
pub mod vcf;
pub mod vct;

pub use threat_board::{
    broken_four_reply, forcing_threat_moves, has_open_four, has_vct_trigger, threat_moves,
    winning_threat_moves, ThreatBoardView,
};
pub use types::{AttackMove, ThreatLevel};
pub use vcf::{VCFResult, VCFSearcher, VcfMemoEntry, NO_MOVE, VCFM};
pub use vct::{VCTResult, VCTSearcher, VctMemoEntry};
