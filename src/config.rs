//! Engine configuration and bundled parameter tables aligned with the reference.

use std::sync::LazyLock;

use crate::constants::DSHAPE_SIZE;

#[derive(Clone, Debug, PartialEq)]
pub struct EvalBucketTables {
    pub last_eval: Vec<f64>,
    pub next_eval: Vec<f64>,
    pub attack_value: Vec<f64>,
    pub defend_value: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SearchParameters {
    pub drift: f64,
    pub dgn: f64,
    pub atdown3: f64,
    pub atdown4: f64,
    pub last_weight: f64,
    pub extend_ratio: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeOptions {
    pub read_config_each_move: bool,
    pub compute_vcf: bool,
    pub nonroot_vcf: bool,
    pub static_board: bool,
    pub dynamic_board_margin: i32,
    pub compute_vct: bool,
    pub root_vct_depth: i32,
    /// Experimental Lazy SMP helper search. Defaults to off because it is not
    /// reference-equivalent and has not shown stable speedups yet.
    pub lazy_smp: bool,
    pub lazy_smp_workers: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootSearchDefaults {
    pub depth: i32,
    pub wide: i32,
    pub ratio_num: i32,
    pub ratio_den: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EngineConfig {
    pub eval_tables: EvalBucketTables,
    pub search: SearchParameters,
    pub runtime: RuntimeOptions,
    pub root_search: RootSearchDefaults,
}

// Generated plain-text static data extracted from the vendored reference text.
const DEFAULT_EVAL_PARA_SOURCE: &str = include_str!("../data/static/default_eval_para.txt");

pub static DEFAULT_EVAL_PARA: LazyLock<Vec<f64>> = LazyLock::new(parse_default_eval_para_lines);

pub fn default_eval_para() -> &'static [f64] {
    DEFAULT_EVAL_PARA.as_slice()
}

pub fn adjust_loaded_parameters(para: &[f64]) -> Vec<f64> {
    let mut adjusted = para.to_vec();
    adjusted[156] += 65_536.0;
    adjusted[157] += 65_536.0;
    adjusted
}

pub fn load_default_config() -> EngineConfig {
    let para = default_eval_para();
    EngineConfig {
        eval_tables: slice_eval_tables(para),
        search: slice_search_parameters(para),
        runtime: default_runtime_options(para),
        root_search: default_root_search(),
    }
}

fn slice_eval_tables(para: &[f64]) -> EvalBucketTables {
    EvalBucketTables {
        last_eval: para[0..DSHAPE_SIZE].to_vec(),
        next_eval: para[DSHAPE_SIZE..DSHAPE_SIZE * 2].to_vec(),
        attack_value: para[DSHAPE_SIZE * 2..DSHAPE_SIZE * 3].to_vec(),
        defend_value: para[DSHAPE_SIZE * 3..DSHAPE_SIZE * 4].to_vec(),
    }
}

fn slice_search_parameters(para: &[f64]) -> SearchParameters {
    let offset = DSHAPE_SIZE * 4;
    SearchParameters {
        drift: para[offset],
        dgn: para[offset + 1],
        atdown3: para[offset + 2],
        atdown4: para[offset + 3],
        last_weight: para[offset + 4],
        extend_ratio: para[offset + 6],
    }
}

fn default_runtime_options(para: &[f64]) -> RuntimeOptions {
    let offset = DSHAPE_SIZE * 4;
    RuntimeOptions {
        read_config_each_move: para[offset + 5] != 0.0,
        compute_vcf: true,
        nonroot_vcf: false,
        static_board: true,
        dynamic_board_margin: 4,
        compute_vct: true,
        root_vct_depth: 8,
        lazy_smp: false,
        lazy_smp_workers: 0,
    }
}

fn default_root_search() -> RootSearchDefaults {
    RootSearchDefaults {
        depth: 25,
        wide: 60,
        ratio_num: 1,
        ratio_den: 1,
    }
}

fn parse_default_eval_para_lines() -> Vec<f64> {
    DEFAULT_EVAL_PARA_SOURCE
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            line.trim()
                .parse::<f64>()
                .expect("default eval parameter line parses as f64")
        })
        .collect()
}
