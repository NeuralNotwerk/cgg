#include "gen.h"
int get_x(void);
void handle_alpha(int v);
int helper_real(void);
int main(void) {
  handle_alpha(3);
  return get_x() + helper_real();
}
