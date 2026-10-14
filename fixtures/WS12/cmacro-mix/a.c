#include "defs.h"
static int width_value;
static int height_value;
void notify(const char *s) { (void)s; }
int helper(int a) { return a; }
int doubler(int a) { return a * 2; }
DEFINE_PAIR(width, int)
DEFINE_PAIR(height, int);
DEFINE_LOCK(biglock);
CALLBACK(on_event, doubler)
int main(void) {
  width_set(3);
  DEFINE_PAIR(inner, int)
  return width_get() + height_get() + on_event(1);
}
