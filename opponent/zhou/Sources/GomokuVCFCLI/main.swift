import Foundation
import GomokuVCFCore

let encoder = JSONEncoder()
let decoder = JSONDecoder()
encoder.outputFormatting = [.sortedKeys]

func writeResult(_ result: VCFResult) {
    let data = try! encoder.encode(result)
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([0x0A]))
}

func solve(_ input: Data) -> VCFResult {
    do {
        let query = try decoder.decode(VCFQuery.self, from: input)
        let board = try BoardState(size: query.boardSize, cells: query.flatGrid)
        let solver = VCFSolver(maxCandidates: query.maxCandidates)

        let move: Move?
        switch query.mode {
        case "find_win":
            move = solver.findWinningMove(
                on: board,
                attacker: query.attacker,
                maxDepth: query.maxDepth
            )
        case "find_block":
            move = solver.findBlockingMove(
                on: board,
                defender: query.defender,
                maxDepth: query.maxDepth
            )
        default:
            throw SolverError.invalidMode("unsupported mode: \(query.mode)")
        }

        return VCFResult(
            found: move != nil,
            move: move.map { [$0.row, $0.col] },
            backend: "swift",
            depthReached: solver.depthReached,
            nodes: solver.nodes,
            error: nil
        )
    } catch {
        return VCFResult(
            found: false,
            move: nil,
            backend: "swift",
            depthReached: 0,
            nodes: 0,
            error: String(describing: error)
        )
    }
}

let isServerMode = CommandLine.arguments.contains("--server")

if isServerMode {
    while let line = readLine() {
        let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
        if trimmed.isEmpty {
            continue
        }
        autoreleasepool {
            let result = solve(Data(trimmed.utf8))
            writeResult(result)
        }
    }
} else {
    let result = solve(FileHandle.standardInput.readDataToEndOfFile())
    writeResult(result)
    let exitCode: Int32 = result.error == nil ? 0 : 1
    Foundation.exit(exitCode)
}
