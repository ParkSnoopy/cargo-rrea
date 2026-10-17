#include <stdint.h>

static uint32_t counter = 7;

uint32_t fixture_native_next(uint32_t input) {
    counter += input;
    return counter;
}
