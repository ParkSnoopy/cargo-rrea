#include "symbols.h"

int strong(void) { return 20; }
int state = 22;
__attribute__((weak)) int weak(void) { return 1; }
_Thread_local int tls = 2;
static int implementation(void) { return 3; }
static int (*resolve(void))(void) { return implementation; }
int indirect(void) __attribute__((ifunc("resolve")));
int versioned(void) { return 4; }
int lookup(void) { return 5; }
int cpp(void) { return 6; }
int plain_api(void) { return external ? external() : 0; }
