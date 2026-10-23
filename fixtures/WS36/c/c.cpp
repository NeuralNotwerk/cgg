#ifdef _WIN32
static int plat(void) { return 1; }
#elif defined(__APPLE__)
static int plat(void) { return 2; }
#else
static int plat(void) { return 3; }
#endif

#if A
static void nest(void) {}
#else
# if B
static void nest(void) {}
# else
static void nest(void) {}
# endif
#endif

#ifdef X
static void dup(void) {}
#endif
static void dup(void) {}

static void twice(void) {}
static void twice(void) {}

#if Y
static void same_branch(void) {}
static void same_branch(void) {}
#endif

int main(void) { plat(); nest(); dup(); twice(); same_branch(); return 0; }
