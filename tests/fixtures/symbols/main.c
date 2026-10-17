#include <dlfcn.h>
#include <stdio.h>
#include "symbols.h"

int main(void) {
    void *library = dlopen("libfixture.so", RTLD_NOW);
    if (!library) return 1;
    int (*function)(void) = (int (*)(void))dlsym(library, LOOKUP);
    if (!function) return 2;
    printf("%d %d %d %d %d %d %d %d %d\n", strong(), state, weak(), tls,
           indirect(), versioned(), function(), cpp(), plain_api());
    return dlclose(library) != 0;
}
