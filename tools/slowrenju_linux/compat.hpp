#pragma once

#include <algorithm>
#include <cassert>
#include <cmath>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <iostream>
#include <string>
#include <unordered_map>
#include <vector>

#ifndef __int64
#define __int64 long long
#endif

inline int slowrenju_printf_s(const char *format, ...) {
    va_list args;
    va_start(args, format);
    const int result = std::vprintf(format, args);
    va_end(args);
    return result;
}

inline int slowrenju_sprintf_s(char *buffer, std::size_t size, const char *format, ...) {
    va_list args;
    va_start(args, format);
    const int result = std::vsnprintf(buffer, size, format, args);
    va_end(args);
    return result;
}

inline int slowrenju_strcat_s(char *destination, std::size_t size, const char *source) {
    const std::size_t used = std::strlen(destination);
    if (used >= size) {
        return 1;
    }
    const std::size_t remaining = size - used - 1;
    std::strncat(destination, source, remaining);
    return std::strlen(source) > remaining ? 1 : 0;
}

#define printf_s slowrenju_printf_s
#define sprintf_s slowrenju_sprintf_s
#define strcat_s slowrenju_strcat_s
