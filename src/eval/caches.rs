//! Incremental evaluation caches.

use crate::constants::BOARD_SIZE;

pub fn caches_backend_name() -> &'static str {
    "python"
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSnapshot {
    pub initialized: bool,
    pub board_shadow: Vec<Vec<i8>>,
    pub shape_log_len: usize,
    pub value_log_len: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalCaches {
    pub initialized: bool,
    pub board_shadow: Vec<Vec<i8>>,
    pub shape_cache: Vec<Vec<Vec<Vec<i32>>>>,
    pub value_cache: Vec<Vec<Vec<i32>>>,
    pub attack_cache: Vec<Vec<Vec<i32>>>,
    pub(crate) shape_log: Vec<(usize, usize, usize, usize, i32)>,
    pub(crate) value_log: Vec<(usize, usize, usize, i32, i32)>,
    pub(crate) active_snapshot_count: usize,
}

impl Default for EvalCaches {
    fn default() -> Self {
        Self::new()
    }
}

impl EvalCaches {
    pub fn new() -> Self {
        Self {
            initialized: false,
            board_shadow: new_board_matrix(),
            shape_cache: new_shape_cache(),
            value_cache: new_value_cache(),
            attack_cache: new_value_cache(),
            shape_log: Vec::new(),
            value_log: Vec::new(),
            active_snapshot_count: 0,
        }
    }

    pub fn set_shape_value(
        &mut self,
        player: usize,
        x: usize,
        y: usize,
        direction: usize,
        value: i32,
    ) {
        let old_value = self.shape_cache[player][x][y][direction];
        if old_value == value {
            return;
        }
        if self.active_snapshot_count > 0 {
            self.shape_log.push((player, x, y, direction, old_value));
        }
        self.shape_cache[player][x][y][direction] = value;
    }

    pub fn snapshot(&mut self) -> EvalSnapshot {
        self.active_snapshot_count += 1;
        EvalSnapshot {
            initialized: self.initialized,
            board_shadow: self.board_shadow.clone(),
            shape_log_len: self.shape_log.len(),
            value_log_len: self.value_log.len(),
        }
    }

    pub fn restore_snapshot(&mut self, snapshot: &EvalSnapshot) {
        self.initialized = snapshot.initialized;
        self.board_shadow = snapshot.board_shadow.clone();

        while self.shape_log.len() > snapshot.shape_log_len {
            let (player, x, y, direction, old_value) =
                self.shape_log.pop().expect("shape log length tracked");
            self.shape_cache[player][x][y][direction] = old_value;
        }
        while self.value_log.len() > snapshot.value_log_len {
            let (player, x, y, old_bucket, old_attack) =
                self.value_log.pop().expect("value log length tracked");
            self.value_cache[player][x][y] = old_bucket;
            self.attack_cache[player][x][y] = old_attack;
        }
        if self.active_snapshot_count > 0 {
            self.active_snapshot_count -= 1;
        }
    }

    pub fn copy(&self) -> Self {
        self.clone()
    }

    pub fn shape_log_len(&self) -> usize {
        self.shape_log.len()
    }

    pub fn value_log_len(&self) -> usize {
        self.value_log.len()
    }

    pub fn restore_from(&mut self, other: &Self) {
        self.initialized = other.initialized;
        self.board_shadow = other.board_shadow.clone();
        self.shape_cache = other.shape_cache.clone();
        self.value_cache = other.value_cache.clone();
        self.attack_cache = other.attack_cache.clone();
        self.shape_log.clear();
        self.value_log.clear();
        self.active_snapshot_count = 0;
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

fn new_board_matrix() -> Vec<Vec<i8>> {
    vec![vec![0; BOARD_SIZE]; BOARD_SIZE]
}

fn new_shape_cache() -> Vec<Vec<Vec<Vec<i32>>>> {
    vec![
        vec![vec![vec![0; 4]; BOARD_SIZE]; BOARD_SIZE],
        vec![vec![vec![0; 4]; BOARD_SIZE]; BOARD_SIZE],
    ]
}

fn new_value_cache() -> Vec<Vec<Vec<i32>>> {
    vec![
        vec![vec![0; BOARD_SIZE]; BOARD_SIZE],
        vec![vec![0; BOARD_SIZE]; BOARD_SIZE],
    ]
}
