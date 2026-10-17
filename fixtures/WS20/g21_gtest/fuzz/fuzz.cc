#include <stdint.h>
#include <stddef.h>

static void parse(const uint8_t *d, size_t n) {}

extern "C" int LLVMFuzzerTestOneInput(const uint8_t *data, size_t size) {
  parse(data, size);
  return 0;
}
