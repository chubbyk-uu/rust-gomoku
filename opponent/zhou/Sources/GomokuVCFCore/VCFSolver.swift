import Foundation

public struct VCFQuery: Codable {
    public let apiVersion: Int
    public let mode: String
    public let attacker: Int
    public let defender: Int
    public let maxDepth: Int
    public let maxCandidates: Int
    public let boardSize: Int
    public let flatGrid: [Int]
}

public struct VCFResult: Codable {
    public let found: Bool
    public let move: [Int]?
    public let backend: String
    public let depthReached: Int
    public let nodes: Int
    public let error: String?

    public init(
        found: Bool,
        move: [Int]?,
        backend: String,
        depthReached: Int,
        nodes: Int,
        error: String?
    ) {
        self.found = found
        self.move = move
        self.backend = backend
        self.depthReached = depthReached
        self.nodes = nodes
        self.error = error
    }
}

public struct Move: Hashable {
    public let row: Int
    public let col: Int

    public init(row: Int, col: Int) {
        self.row = row
        self.col = col
    }
}

private struct CacheKey: Hashable {
    let hash: UInt64
    let attacker: Int
    let depth: Int
}

private let defaultZobristTable = buildZobristTable(size: 15)

private struct SplitMix64 {
    private var state: UInt64

    init(seed: UInt64) {
        self.state = seed
    }

    mutating func next() -> UInt64 {
        state &+= 0x9E37_79B9_7F4A_7C15
        var value = state
        value = (value ^ (value >> 30)) &* 0xBF58_476D_1CE4_E5B9
        value = (value ^ (value >> 27)) &* 0x94D0_49BB_1331_11EB
        return value ^ (value >> 31)
    }
}

private func buildZobristTable(size: Int) -> [UInt64] {
    var rng = SplitMix64(seed: 0xC0DE_CAFE_F00D_F00D ^ UInt64(size))
    var table: [UInt64] = []
    table.reserveCapacity(size * size * 2)
    for _ in 0..<(size * size * 2) {
        table.append(rng.next())
    }
    return table
}

private func zobristSlot(cellIndex: Int, player: Int) -> Int {
    (cellIndex * 2) + (player - 1)
}

public struct BoardState {
    public let size: Int
    public private(set) var cells: [Int]
    private let zobrist: [UInt64]
    private var stoneCount: Int
    private var hash: UInt64

    public init(size: Int, cells: [Int]) throws {
        guard cells.count == size * size else {
            throw SolverError.invalidBoard("flatGrid size does not match boardSize")
        }
        self.zobrist = size == 15 ? defaultZobristTable : buildZobristTable(size: size)
        self.size = size
        self.cells = cells
        self.stoneCount = 0
        self.hash = 0
        for (index, value) in cells.enumerated() {
            guard (0...2).contains(value) else {
                throw SolverError.invalidBoard("flatGrid contains unsupported cell value")
            }
            if value == 0 {
                continue
            }
            stoneCount += 1
            hash ^= zobrist[zobristSlot(cellIndex: index, player: value)]
        }
    }

    public subscript(row: Int, col: Int) -> Int {
        get { cells[(row * size) + col] }
        set {
            let index = (row * size) + col
            let oldValue = cells[index]
            if oldValue == newValue {
                return
            }
            if oldValue != 0 {
                hash ^= zobrist[zobristSlot(cellIndex: index, player: oldValue)]
                stoneCount -= 1
            }
            cells[index] = newValue
            if newValue != 0 {
                hash ^= zobrist[zobristSlot(cellIndex: index, player: newValue)]
                stoneCount += 1
            }
        }
    }

    public func checkWin(row: Int, col: Int, player: Int) -> Bool {
        let directions = [(1, 0), (0, 1), (1, 1), (1, -1)]
        for (dr, dc) in directions {
            var count = 1
            for sign in [1, -1] {
                var nr = row + (dr * sign)
                var nc = col + (dc * sign)
                while isInside(row: nr, col: nc) && self[nr, nc] == player {
                    count += 1
                    nr += dr * sign
                    nc += dc * sign
                }
            }
            if count >= 5 {
                return true
            }
        }
        return false
    }

    public func candidateMoves() -> [Move] {
        if stoneCount == 0 {
            let center = size / 2
            return [Move(row: center, col: center)]
        }

        var moves: [Move] = []
        moves.reserveCapacity(64)
        for row in 0..<size {
            for col in 0..<size {
                if self[row, col] != 0 {
                    continue
                }
                var found = false
                for dr in -1...1 {
                    for dc in -1...1 {
                        let nr = row + dr
                        let nc = col + dc
                        if isInside(row: nr, col: nc) && self[nr, nc] != 0 {
                            found = true
                            break
                        }
                    }
                    if found {
                        break
                    }
                }
                if found {
                    moves.append(Move(row: row, col: col))
                }
            }
        }
        return moves
    }

    public func boardHash() -> UInt64 {
        return hash
    }

    private func isInside(row: Int, col: Int) -> Bool {
        row >= 0 && row < size && col >= 0 && col < size
    }
}

public final class VCFSolver {
    private let maxCandidates: Int
    private var cache: [CacheKey: Bool] = [:]
    private var maxDepthBudget = 0

    public private(set) var nodes = 0
    public private(set) var depthReached = 0

    public init(maxCandidates: Int) {
        self.maxCandidates = maxCandidates
    }

    public func findWinningMove(on board: BoardState, attacker: Int, maxDepth: Int) -> Move? {
        reset(maxDepth: maxDepth)
        var working = board
        return findVCFMove(on: &working, attacker: attacker, depth: maxDepth)
    }

    public func findBlockingMove(on board: BoardState, defender: Int, maxDepth: Int) -> Move? {
        reset(maxDepth: maxDepth)
        var working = board
        return findBlockingMoveAgainstVCF(on: &working, defender: defender, depth: maxDepth)
    }

    private func reset(maxDepth: Int) {
        cache.removeAll(keepingCapacity: true)
        nodes = 0
        depthReached = 0
        maxDepthBudget = maxDepth
    }

    private func orderMoves(
        on board: BoardState,
        moves: [Move],
        currentPlayer: Int,
        maxCandidates: Int?
    ) -> [Move] {
        let opponent = currentPlayer == 1 ? 2 : 1
        var scored: [(Move, Int)] = []
        scored.reserveCapacity(moves.count)
        for move in moves {
            let attack = evaluateLocal(on: board, row: move.row, col: move.col, player: currentPlayer)
            let defend = evaluateLocal(on: board, row: move.row, col: move.col, player: opponent)
            scored.append((move, (attack * 2) + (defend * 3)))
        }
        scored.sort { lhs, rhs in lhs.1 > rhs.1 }

        let limit = min(maxCandidates ?? scored.count, scored.count)
        var ordered: [Move] = []
        ordered.reserveCapacity(limit)
        for index in 0..<limit {
            ordered.append(scored[index].0)
        }
        return ordered
    }

    private func findVCFMove(on board: inout BoardState, attacker: Int, depth: Int) -> Move? {
        guard depth > 0 else {
            return nil
        }
        for move in generateVCFAttacks(on: &board, attacker: attacker) {
            if vcfMoveWins(on: &board, attacker: attacker, move: move, depth: depth) {
                return move
            }
        }
        return nil
    }

    private func findBlockingMoveAgainstVCF(
        on board: inout BoardState,
        defender: Int,
        depth: Int
    ) -> Move? {
        guard depth > 0 else {
            return nil
        }
        let attacker = defender == 1 ? 2 : 1
        guard hasVCF(on: &board, attacker: attacker, depth: depth) else {
            return nil
        }

        let moves = orderMoves(
            on: board,
            moves: board.candidateMoves(),
            currentPlayer: defender,
            maxCandidates: nil
        )
        for move in moves {
            board[move.row, move.col] = defender

            if board.checkWin(row: move.row, col: move.col, player: defender) {
                board[move.row, move.col] = 0
                return move
            }

            if !hasVCF(on: &board, attacker: attacker, depth: max(depth - 1, 0)) {
                board[move.row, move.col] = 0
                return move
            }

            board[move.row, move.col] = 0
        }

        return nil
    }

    private func hasVCF(on board: inout BoardState, attacker: Int, depth: Int) -> Bool {
        guard depth > 0 else {
            return false
        }

        nodes += 1
        depthReached = max(depthReached, maxDepthBudget - depth + 1)

        let key = CacheKey(hash: board.boardHash(), attacker: attacker, depth: depth)
        if let cached = cache[key] {
            return cached
        }

        var result = false
        for move in generateVCFAttacks(on: &board, attacker: attacker) {
            if vcfMoveWins(on: &board, attacker: attacker, move: move, depth: depth) {
                result = true
                break
            }
        }

        cache[key] = result
        return result
    }

    private func vcfMoveWins(
        on board: inout BoardState,
        attacker: Int,
        move: Move,
        depth: Int
    ) -> Bool {
        guard depth > 0 else {
            return false
        }

        let defender = attacker == 1 ? 2 : 1
        board[move.row, move.col] = attacker

        if board.checkWin(row: move.row, col: move.col, player: attacker) {
            board[move.row, move.col] = 0
            return true
        }

        if !findImmediateWins(on: &board, player: defender, limit: 1).isEmpty {
            board[move.row, move.col] = 0
            return false
        }

        let defenses = findImmediateWins(on: &board, player: attacker, limit: nil)
        if defenses.isEmpty {
            board[move.row, move.col] = 0
            return false
        }

        for defense in defenses {
            board[defense.row, defense.col] = defender

            if board.checkWin(row: defense.row, col: defense.col, player: defender) {
                board[defense.row, defense.col] = 0
                board[move.row, move.col] = 0
                return false
            }

            if !hasVCF(on: &board, attacker: attacker, depth: depth - 1) {
                board[defense.row, defense.col] = 0
                board[move.row, move.col] = 0
                return false
            }

            board[defense.row, defense.col] = 0
        }

        board[move.row, move.col] = 0
        return true
    }

    private func generateVCFAttacks(on board: inout BoardState, attacker: Int) -> [Move] {
        let moves = orderMoves(
            on: board,
            moves: board.candidateMoves(),
            currentPlayer: attacker,
            maxCandidates: nil
        )

        var forcing: [Move] = []
        for move in moves {
            board[move.row, move.col] = attacker

            if board.checkWin(row: move.row, col: move.col, player: attacker)
                || !findImmediateWins(on: &board, player: attacker, limit: 1).isEmpty
            {
                forcing.append(move)
            }

            board[move.row, move.col] = 0

            if forcing.count >= maxCandidates {
                break
            }
        }

        return forcing
    }

    private func findImmediateWins(
        on board: inout BoardState,
        player: Int,
        limit: Int?
    ) -> [Move] {
        let moves = orderMoves(
            on: board,
            moves: board.candidateMoves(),
            currentPlayer: player,
            maxCandidates: nil
        )

        var wins: [Move] = []
        for move in moves {
            board[move.row, move.col] = player
            if board.checkWin(row: move.row, col: move.col, player: player) {
                wins.append(move)
                board[move.row, move.col] = 0
                if let limit, wins.count >= limit {
                    break
                }
                continue
            }
            board[move.row, move.col] = 0
        }
        return wins
    }
}

public enum SolverError: Error {
    case invalidBoard(String)
    case invalidMode(String)
}

private let directions: [(Int, Int)] = [(1, 0), (0, 1), (1, 1), (1, -1)]
private let localPatternScores: [([UInt8], Int)] = [
    (encodedPattern("11111"), 100_000),
    (encodedPattern("011110"), 10_000),
    (encodedPattern("211110"), 1_000),
    (encodedPattern("011112"), 1_000),
    (encodedPattern("10111"), 1_000),
    (encodedPattern("11011"), 1_000),
    (encodedPattern("11101"), 1_000),
    (encodedPattern("01110"), 1_000),
    (encodedPattern("011010"), 1_000),
    (encodedPattern("010110"), 1_000),
]

private func encodedPattern(_ pattern: String) -> [UInt8] {
    Array(pattern.utf8).map { $0 &- 48 }
}

private func evaluateLocal(on board: BoardState, row: Int, col: Int, player: Int) -> Int {
    guard board[row, col] == 0 else {
        return 0
    }

    var score = 0
    for (dr, dc) in directions {
        score += scoreLocalDirection(on: board, row: row, col: col, player: player, dr: dr, dc: dc)
    }
    return score
}

private func scoreLocalDirection(
    on board: BoardState,
    row: Int,
    col: Int,
    player: Int,
    dr: Int,
    dc: Int
) -> Int {
    var best = contiguousPointScore(on: board, row: row, col: col, player: player, dr: dr, dc: dc)
    let line = buildLocalLine(on: board, row: row, col: col, player: player, dr: dr, dc: dc)
    let center = 4

    for (pattern, score) in localPatternScores {
        let maxStart = line.count - pattern.count
        if maxStart < 0 {
            continue
        }
        for start in 0...maxStart {
            let end = start + pattern.count
            if start > center || center >= end {
                continue
            }
            if matchesPattern(line, pattern: pattern, start: start) {
                best = max(best, score)
                break
            }
        }
    }

    return best
}

private func contiguousPointScore(
    on board: BoardState,
    row: Int,
    col: Int,
    player: Int,
    dr: Int,
    dc: Int
) -> Int {
    var count = 1
    var blocks = 0

    var nr = row + dr
    var nc = col + dc
    while isInside(board: board, row: nr, col: nc) && board[nr, nc] == player {
        count += 1
        nr += dr
        nc += dc
    }
    if !isInside(board: board, row: nr, col: nc) || board[nr, nc] != 0 {
        blocks += 1
    }

    nr = row - dr
    nc = col - dc
    while isInside(board: board, row: nr, col: nc) && board[nr, nc] == player {
        count += 1
        nr -= dr
        nc -= dc
    }
    if !isInside(board: board, row: nr, col: nc) || board[nr, nc] != 0 {
        blocks += 1
    }

    return getScore(count: count, blocks: blocks)
}

private func buildLocalLine(
    on board: BoardState,
    row: Int,
    col: Int,
    player: Int,
    dr: Int,
    dc: Int,
    radius: Int = 4
) -> [UInt8] {
    var line = [UInt8](repeating: 0, count: (radius * 2) + 1)
    var index = 0
    for step in (-radius)...radius {
        let nr = row + (dr * step)
        let nc = col + (dc * step)
        if step == 0 {
            line[index] = 1
        } else if !isInside(board: board, row: nr, col: nc) {
            line[index] = 2
        } else {
            line[index] = encodeCell(board[nr, nc], player: player)
        }
        index += 1
    }
    return line
}

@inline(__always)
private func encodeCell(_ cell: Int, player: Int) -> UInt8 {
    if cell == player {
        return 1
    }
    if cell == 0 {
        return 0
    }
    return 2
}

@inline(__always)
private func matchesPattern(_ line: [UInt8], pattern: [UInt8], start: Int) -> Bool {
    for offset in 0..<pattern.count {
        if line[start + offset] != pattern[offset] {
            return false
        }
    }
    return true
}

private func getScore(count: Int, blocks: Int) -> Int {
    if count >= 5 {
        return 100_000
    }
    if blocks >= 2 {
        return 0
    }

    switch (count, blocks) {
    case (4, 0): return 10_000
    case (4, 1): return 1_000
    case (3, 0): return 1_000
    case (3, 1): return 100
    case (2, 0): return 100
    case (2, 1): return 10
    case (1, 0): return 10
    case (1, 1): return 1
    default: return 0
    }
}

@inline(__always)
private func isInside(board: BoardState, row: Int, col: Int) -> Bool {
    row >= 0 && row < board.size && col >= 0 && col < board.size
}
