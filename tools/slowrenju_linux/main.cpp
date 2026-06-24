#include <algorithm>
#include <cctype>
#include <ctime>
#include <cstdlib>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

#include "game.h"

void InitHash();

namespace {

struct ProtocolStone {
    int x;
    int y;
    int relative_side;
};

int max_search_depth = 8;
int search_width = 40;
int ratio_num = 1;
int ratio_den = 1;
std::vector<int> moves;
int engine_side = 0;

std::string upper(std::string text) {
    std::transform(text.begin(), text.end(), text.begin(), [](unsigned char c) {
        return static_cast<char>(std::toupper(c));
    });
    return text;
}

bool parse_xy(const std::string &text, int &x, int &y) {
    char comma = '\0';
    std::istringstream input(text);
    return static_cast<bool>(input >> x >> comma >> y) && comma == ',';
}

void reset_position(int size) {
    boardSize = size;
    S = size;
    moves.clear();
    engine_side = 0;
    init();
    InitHash();
}

int absolute_side_for_relative(int relative_side, int inferred_engine_side) {
    return relative_side == 1 ? inferred_engine_side : -inferred_engine_side;
}

bool load_board(const std::vector<ProtocolStone> &stones) {
    int own_count = 0;
    int opponent_count = 0;
    for (const ProtocolStone &stone : stones) {
        if (stone.relative_side == 1) {
            own_count++;
        } else if (stone.relative_side == 2) {
            opponent_count++;
        } else {
            return false;
        }
    }

    if (own_count == opponent_count) {
        engine_side = 1;
    } else if (own_count + 1 == opponent_count) {
        engine_side = -1;
    } else {
        return false;
    }

    init();
    for (const ProtocolStone &stone : stones) {
        if (stone.x < 0 || stone.y < 0 || stone.x >= boardSize || stone.y >= boardSize) {
            return false;
        }
        if (board[stone.x][stone.y] != 0) {
            return false;
        }
        board[stone.x][stone.y] =
            absolute_side_for_relative(stone.relative_side, engine_side);
    }
    bmove = static_cast<int>(stones.size());
    return true;
}

int choose_move() {
    compend = 0;
    comphalfend = 0;
    gvstop = false;
    ts = std::clock();
    constexpr long long fixed_search_budget =
        24LL * 60LL * 60LL * static_cast<long long>(CLOCKS_PER_SEC);
    timee = fixed_search_budget;
    timel = fixed_search_budget * 15;
    return rootsearch(max_search_depth - 1, search_width, ratio_num, ratio_den);
}

void emit_move(int move) {
    if (move < 0 || move >= S * S) {
        std::cout << "ERROR No legal move." << std::endl;
        return;
    }
    const int x = move % S;
    const int y = move / S;
    std::cout << x << ',' << y << std::endl;
}

void handle_info(std::istringstream &input) {
    std::string key;
    long long value = 0;
    if (!(input >> key >> value)) {
        return;
    }
    key = upper(key);
    if (key == "RULE") {
        value &= 5;
        if (value == 0) {
            nosix = 0;
            fflag = 0;
        } else if (value == 1) {
            nosix = 1;
            fflag = 0;
        } else if (value == 4) {
            nosix = 0;
            fflag = 1;
        }
    } else if (key == "MAX_NODE") {
        nodelimit = value;
    } else if (key == "COMPUTE_VCF") {
        computevcf = value != 0;
    } else if (key == "STATIC") {
        staticboard = static_cast<int>(value % 2);
    } else if (key == "SR_DEPTH") {
        max_search_depth = (std::max)(1, static_cast<int>(value));
    } else if (key == "SR_WIDTH") {
        search_width = (std::max)(1, static_cast<int>(value));
    }
}

}  // namespace

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        const std::string arg = argv[i];
        if (arg == "--depth" && i + 1 < argc) {
            max_search_depth = (std::max)(1, std::atoi(argv[++i]));
        } else if (arg == "--width" && i + 1 < argc) {
            search_width = (std::max)(1, std::atoi(argv[++i]));
        } else if (arg == "--ratio-num" && i + 1 < argc) {
            ratio_num = (std::max)(1, std::atoi(argv[++i]));
        } else if (arg == "--ratio-den" && i + 1 < argc) {
            ratio_den = (std::max)(1, std::atoi(argv[++i]));
        }
    }

    std::srand(1232356);
    reset_position(15);
    std::cout << "MESSAGE SlowRenju Linux compatibility entry is ready." << std::endl;

    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream input(line);
        std::string command;
        input >> command;
        command = upper(command);

        if (command == "START") {
            int size = 0;
            input >> size;
            if (size < 5 || size > 20) {
                std::cout << "ERROR Size error." << std::endl;
            } else {
                reset_position(size);
                std::cout << "OK" << std::endl;
            }
        } else if (command == "RESTART") {
            reset_position(boardSize);
            std::cout << "OK" << std::endl;
        } else if (command == "INFO") {
            handle_info(input);
        } else if (command == "ABOUT") {
            std::cout
                << "name=\"SlowRenju-linux\", version=\"" << version
                << "\", author=\"Tianyi Hao\", country=\"China\""
                << std::endl;
        } else if (command == "BEGIN") {
            engine_side = 1;
            emit_move(choose_move());
        } else if (command == "TURN") {
            std::string coordinate;
            input >> coordinate;
            int x = 0;
            int y = 0;
            if (!parse_xy(coordinate, x, y)) {
                std::cout << "ERROR Coordinate error." << std::endl;
                continue;
            }
            if (engine_side == 0) {
                engine_side = -1;
            }
            moves.push_back(x + y * boardSize);
            init();
            for (std::size_t i = 0; i < moves.size(); i++) {
                const int move = moves[i];
                board[move % boardSize][move / boardSize] = i % 2 == 0 ? 1 : -1;
            }
            bmove = static_cast<int>(moves.size());
            const int response = choose_move();
            moves.push_back((response % S) + (response / S) * boardSize);
            emit_move(response);
        } else if (command == "BOARD") {
            std::vector<ProtocolStone> stones;
            while (std::getline(std::cin, line)) {
                if (upper(line) == "DONE") {
                    break;
                }
                std::replace(line.begin(), line.end(), ',', ' ');
                std::istringstream stone_input(line);
                ProtocolStone stone{};
                if (!(stone_input >> stone.x >> stone.y >> stone.relative_side)) {
                    stones.clear();
                    break;
                }
                stones.push_back(stone);
            }
            if (!load_board(stones)) {
                std::cout << "ERROR Board error." << std::endl;
            } else {
                emit_move(choose_move());
            }
        } else if (command == "END") {
            break;
        }
    }
    return 0;
}
