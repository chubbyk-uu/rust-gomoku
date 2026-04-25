from gomoku.config import BOARD_SIZE, Player


cdef tuple _DIRECTIONS = ((1, 0), (0, 1), (1, 1), (1, -1))
cdef dict _SCORE_TABLE = {
    (5, 0): 100_000,
    (5, 1): 100_000,
    (5, 2): 100_000,
    (4, 0): 10_000,
    (4, 1): 1_000,
    (3, 0): 1_000,
    (3, 1): 100,
    (2, 0): 100,
    (2, 1): 10,
    (1, 0): 10,
    (1, 1): 1,
}
cdef dict _LOCAL_PATTERN_SCORES = {
    "11111": _SCORE_TABLE[(5, 0)],
    "011110": _SCORE_TABLE[(4, 0)],
    "211110": _SCORE_TABLE[(4, 1)],
    "011112": _SCORE_TABLE[(4, 1)],
    "10111": _SCORE_TABLE[(4, 1)],
    "11011": _SCORE_TABLE[(4, 1)],
    "11101": _SCORE_TABLE[(4, 1)],
    "01110": _SCORE_TABLE[(3, 0)],
    "011010": _SCORE_TABLE[(3, 0)],
    "010110": _SCORE_TABLE[(3, 0)],
}
cdef tuple _BROKEN_FOUR_PATTERNS = ("10111", "11011", "11101")
cdef tuple _BROKEN_THREE_PATTERNS = ("011010", "010110")
cdef int _PLAYER_NONE = int(Player.NONE)


cdef inline int _get_score(int count, int blocks):
    if count >= 5:
        return 100_000
    if blocks >= 2:
        return 0
    return <int>_SCORE_TABLE.get((count, blocks), 0)


cdef inline str _encode_cell(object cell, object player):
    if cell == player:
        return "1"
    if int(cell) == _PLAYER_NONE:
        return "0"
    return "2"


cdef int _contiguous_point_score(
    list grid,
    int r,
    int c,
    object player,
    int dr,
    int dc,
):
    cdef int count = 1
    cdef int blocks = 0
    cdef int nr = r + dr
    cdef int nc = c + dc

    while 0 <= nr < BOARD_SIZE and 0 <= nc < BOARD_SIZE and grid[nr][nc] == player:
        count += 1
        nr += dr
        nc += dc
    if nr < 0 or nr >= BOARD_SIZE or nc < 0 or nc >= BOARD_SIZE or int(grid[nr][nc]) != _PLAYER_NONE:
        blocks += 1

    nr = r - dr
    nc = c - dc
    while 0 <= nr < BOARD_SIZE and 0 <= nc < BOARD_SIZE and grid[nr][nc] == player:
        count += 1
        nr -= dr
        nc -= dc
    if nr < 0 or nr >= BOARD_SIZE or nc < 0 or nc >= BOARD_SIZE or int(grid[nr][nc]) != _PLAYER_NONE:
        blocks += 1

    return _get_score(count, blocks)


cdef str _build_local_line(
    list grid,
    int r,
    int c,
    object player,
    int dr,
    int dc,
    int radius,
):
    cdef list chars = []
    cdef int step
    cdef int nr
    cdef int nc

    for step in range(-radius, radius + 1):
        nr = r + dr * step
        nc = c + dc * step
        if step == 0:
            chars.append("1")
        elif nr < 0 or nr >= BOARD_SIZE or nc < 0 or nc >= BOARD_SIZE:
            chars.append("2")
        else:
            chars.append(_encode_cell(grid[nr][nc], player))
    return "".join(chars)


cdef int _score_local_direction(
    list grid,
    int r,
    int c,
    object player,
    int dr,
    int dc,
):
    cdef int score = _contiguous_point_score(grid, r, c, player, dr, dc)
    cdef str line = _build_local_line(grid, r, c, player, dr, dc, 4)
    cdef int center = 4
    cdef str pattern
    cdef int pattern_score
    cdef int start
    cdef int idx

    for pattern, pattern_score in _LOCAL_PATTERN_SCORES.items():
        start = 0
        while True:
            idx = line.find(pattern, start)
            if idx == -1:
                break
            if idx <= center < idx + len(pattern):
                if pattern_score > score:
                    score = pattern_score
            start = idx + 1

    return score


def evaluate_local_native(list grid, int r, int c, object player):
    cdef int score = 0
    cdef int dr
    cdef int dc

    if int(grid[r][c]) != _PLAYER_NONE:
        return 0

    for dr, dc in _DIRECTIONS:
        score += _score_local_direction(grid, r, c, player, dr, dc)
    return score


cdef bint _is_immediate_win_at(list grid, int row, int col, object player):
    cdef int dr
    cdef int dc
    cdef int count
    cdef int sign
    cdef int step
    cdef int r
    cdef int c

    if int(grid[row][col]) != _PLAYER_NONE:
        return False

    for dr, dc in _DIRECTIONS:
        count = 1
        for sign in (1, -1):
            step = 1
            while True:
                r = row + sign * dr * step
                c = col + sign * dc * step
                if 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE and grid[r][c] == player:
                    count += 1
                    step += 1
                else:
                    break
        if count >= 5:
            return True

    return False


def immediate_win_moves_native(list grid, list moves, object player, object limit=None):
    cdef list wins = []
    cdef object move
    cdef int row
    cdef int col
    cdef int max_wins = -1

    if limit is not None:
        max_wins = int(limit)

    for move in moves:
        row, col = move
        if _is_immediate_win_at(grid, row, col, player):
            wins.append((row, col))
            if max_wins != -1 and len(wins) >= max_wins:
                break

    return wins


cdef bint _has_neighboring_stone(list grid, int row, int col):
    cdef int di
    cdef int dj
    cdef int nr
    cdef int nc

    for di in range(-1, 2):
        for dj in range(-1, 2):
            nr = row + di
            nc = col + dj
            if 0 <= nr < BOARD_SIZE and 0 <= nc < BOARD_SIZE and int(grid[nr][nc]) != _PLAYER_NONE:
                return True
    return False


cdef bint _has_immediate_followup_after_place(list grid, int row, int col, object player):
    cdef int r
    cdef int c

    grid[row][col] = player
    try:
        for r in range(BOARD_SIZE):
            for c in range(BOARD_SIZE):
                if int(grid[r][c]) != _PLAYER_NONE:
                    continue
                if not _has_neighboring_stone(grid, r, c):
                    continue
                if _is_immediate_win_at(grid, r, c, player):
                    return True
        return False
    finally:
        grid[row][col] = Player.NONE


def vcf_attack_moves_native(list grid, list moves, object player, int max_candidates):
    cdef list forcing = []
    cdef object move
    cdef int row
    cdef int col

    for move in moves:
        row, col = move
        if _is_immediate_win_at(grid, row, col, player) or _has_immediate_followup_after_place(
            grid, row, col, player
        ):
            forcing.append((row, col))
            if len(forcing) >= max_candidates:
                break

    return forcing


def candidate_moves_native(list grid, int radius=1):
    cdef int row
    cdef int col
    cdef int di
    cdef int dj
    cdef int nr
    cdef int nc
    cdef bint has_stone = False
    cdef bint found = False
    cdef int center = BOARD_SIZE // 2
    cdef list moves = []

    for row in range(BOARD_SIZE):
        for col in range(BOARD_SIZE):
            if int(grid[row][col]) != _PLAYER_NONE:
                has_stone = True
                break
        if has_stone:
            break

    if not has_stone:
        return [(center, center)]

    for row in range(BOARD_SIZE):
        for col in range(BOARD_SIZE):
            if int(grid[row][col]) != _PLAYER_NONE:
                continue
            found = False
            for di in range(-radius, radius + 1):
                for dj in range(-radius, radius + 1):
                    nr = row + di
                    nc = col + dj
                    if 0 <= nr < BOARD_SIZE and 0 <= nc < BOARD_SIZE:
                        if int(grid[nr][nc]) != _PLAYER_NONE:
                            found = True
                            break
                if found:
                    break
            if found:
                moves.append((row, col))

    return moves


cdef str _encode_line(list chars):
    return "2" + "".join(chars) + "2"


cdef int _count_pattern(str line, str pattern):
    cdef int count = 0
    cdef int start = 0
    cdef int idx

    while True:
        idx = line.find(pattern, start)
        if idx == -1:
            return count
        count += 1
        start = idx + 1


cdef list _iter_line_strings(list grid, object player):
    cdef list lines = []
    cdef list chars
    cdef int r
    cdef int c
    cdef int start_c
    cdef int start_r

    for r in range(BOARD_SIZE):
        chars = []
        for c in range(BOARD_SIZE):
            chars.append(_encode_cell(grid[r][c], player))
        lines.append(_encode_line(chars))

    for c in range(BOARD_SIZE):
        chars = []
        for r in range(BOARD_SIZE):
            chars.append(_encode_cell(grid[r][c], player))
        lines.append(_encode_line(chars))

    for start_c in range(BOARD_SIZE):
        chars = []
        r = 0
        c = start_c
        while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE:
            chars.append(_encode_cell(grid[r][c], player))
            r += 1
            c += 1
        lines.append(_encode_line(chars))

    for start_r in range(1, BOARD_SIZE):
        chars = []
        r = start_r
        c = 0
        while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE:
            chars.append(_encode_cell(grid[r][c], player))
            r += 1
            c += 1
        lines.append(_encode_line(chars))

    for start_c in range(BOARD_SIZE):
        chars = []
        r = 0
        c = start_c
        while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE:
            chars.append(_encode_cell(grid[r][c], player))
            r += 1
            c -= 1
        lines.append(_encode_line(chars))

    for start_r in range(1, BOARD_SIZE):
        chars = []
        r = start_r
        c = BOARD_SIZE - 1
        while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE:
            chars.append(_encode_cell(grid[r][c], player))
            r += 1
            c -= 1
        lines.append(_encode_line(chars))

    return lines


cdef int _score_for_native(list grid, object player):
    cdef int total = 0
    cdef int half_fours = 0
    cdef int open_threes = 0
    cdef int i
    cdef int j
    cdef int dr
    cdef int dc
    cdef int prev_r
    cdef int prev_c
    cdef int count
    cdef int r
    cdef int c
    cdef int blocks
    cdef int pr
    cdef int pc
    cdef str line
    cdef str pattern
    cdef int broken_fours
    cdef int broken_threes

    for i in range(BOARD_SIZE):
        for j in range(BOARD_SIZE):
            if grid[i][j] != player:
                continue
            for dr, dc in _DIRECTIONS:
                prev_r = i - dr
                prev_c = j - dc
                if 0 <= prev_r < BOARD_SIZE and 0 <= prev_c < BOARD_SIZE and grid[prev_r][prev_c] == player:
                    continue

                count = 0
                r = i
                c = j
                while 0 <= r < BOARD_SIZE and 0 <= c < BOARD_SIZE and grid[r][c] == player:
                    count += 1
                    r += dr
                    c += dc

                blocks = 0
                if r < 0 or r >= BOARD_SIZE or c < 0 or c >= BOARD_SIZE or int(grid[r][c]) != _PLAYER_NONE:
                    blocks += 1
                pr = i - dr
                pc = j - dc
                if pr < 0 or pr >= BOARD_SIZE or pc < 0 or pc >= BOARD_SIZE or int(grid[pr][pc]) != _PLAYER_NONE:
                    blocks += 1

                total += _get_score(count, blocks)

                if blocks < 2:
                    if count >= 4 and blocks == 1:
                        half_fours += 1
                    elif count == 3 and blocks == 0:
                        open_threes += 1

    for line in _iter_line_strings(grid, player):
        broken_fours = 0
        for pattern in _BROKEN_FOUR_PATTERNS:
            broken_fours += _count_pattern(line, pattern)
        broken_threes = 0
        for pattern in _BROKEN_THREE_PATTERNS:
            broken_threes += _count_pattern(line, pattern)
        total += broken_fours * 1000
        total += broken_threes * 1000
        half_fours += broken_fours
        open_threes += broken_threes

    if open_threes >= 2:
        total += 5000
    if open_threes >= 1 and half_fours >= 1:
        total += 5000

    return total


def evaluate_native(list grid, object ai_player, double defense_weight):
    cdef object opponent = Player.WHITE if ai_player == Player.BLACK else Player.BLACK
    cdef int ai_score = _score_for_native(grid, ai_player)
    cdef int opp_score = _score_for_native(grid, opponent)
    return ai_score - int(opp_score * defense_weight)


def order_moves_by_hotness_native(
    list grid,
    list moves,
    object current_player,
    object opponent,
    double defense_weight,
    object max_candidates=None,
):
    cdef list scored = []
    cdef object move
    cdef int row
    cdef int col
    cdef int attack_score
    cdef int defend_score
    cdef double hotness

    for move in moves:
        row, col = move
        attack_score = evaluate_local_native(grid, row, col, current_player)
        defend_score = evaluate_local_native(grid, row, col, opponent)
        hotness = attack_score + defend_score * defense_weight
        scored.append((row, col, hotness))

    scored.sort(key=lambda item: item[2], reverse=True)
    if max_candidates is None:
        return [(row, col) for row, col, _ in scored]
    return [(row, col) for row, col, _ in scored[:int(max_candidates)]]


def order_search_moves_native(
    list grid,
    list moves,
    object current_player,
    object opponent,
    object tt_move,
    list killers,
    double defense_weight,
    object max_candidates=None,
):
    cdef list priority = []
    cdef list normal = []
    cdef object move
    cdef int row
    cdef int col
    cdef object scored_moves
    cdef int remaining_slots

    for move in moves:
        row, col = move
        if tt_move is not None and move == tt_move:
            priority.insert(0, (row, col))
        elif move in killers:
            priority.append((row, col))
        else:
            normal.append((row, col))

    scored_moves = order_moves_by_hotness_native(
        grid,
        normal,
        current_player,
        opponent,
        defense_weight,
        None,
    )

    if max_candidates is None:
        return priority + scored_moves

    remaining_slots = max(int(max_candidates) - len(priority), 0)
    return (priority + scored_moves[:remaining_slots])[: int(max_candidates)]
