/* Native PE loader fixture: executable -> middle DLL -> leaf DLL. */
static const volatile unsigned char padding[65536] = {1};

#if defined(LEAF)
__declspec(dllexport) int leaf_value(void) {
    return 39 + padding[0];
}
__declspec(dllexport) int (*relocated_pointer)(void) = leaf_value;
#elif defined(MIDDLE)
__declspec(dllimport) int leaf_value(void);
__declspec(dllexport) int middle_value(void) {
    return leaf_value() + padding[0];
}
__declspec(dllexport) int (*relocated_pointer)(void) = middle_value;
#else
__declspec(dllimport) int middle_value(void);
int entry(void) {
    return middle_value() != 40 + padding[0];
}
#endif
