/* Synthetic Rust spellings exercise ELF binding and version boundaries. */
#define STRONG "_ZN7fixture6strong17h1111111111111111E"
#define STATE "_ZN7fixture5state17h2222222222222222E"
#define WEAK "_ZN7fixture4weak17h3333333333333333E"
#define TLS "_ZN7fixture3tls17h4444444444444444E"
#define INDIRECT "_ZN7fixture8indirect17h5555555555555555E"
#define VERSIONED "_ZN7fixture9versioned17h6666666666666666E"
#define LOOKUP "_ZN7fixture6lookup17h7777777777777777E"
#define EXTERNAL "_ZN7fixture8external17h8888888888888888E"

int strong(void) __asm__(STRONG);
extern int state __asm__(STATE);
int weak(void) __asm__(WEAK);
extern _Thread_local int tls __asm__(TLS);
int indirect(void) __asm__(INDIRECT);
int versioned(void) __asm__(VERSIONED);
int lookup(void) __asm__(LOOKUP);
int external(void) __asm__(EXTERNAL) __attribute__((weak));
int cpp(void) __asm__("_ZN3foo3barE");
int plain_api(void);
