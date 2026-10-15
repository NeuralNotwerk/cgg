#include <stdio.h>
#define ASSERT(e) do { if (!(e)) printf("fail\n"); } while (0)
int check(int y) { ASSERT(y); return y; }
