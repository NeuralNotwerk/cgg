/* A struct main.c does not include: its fields hide nothing there. */
struct thread { void *start; void (*handler)(int); };
