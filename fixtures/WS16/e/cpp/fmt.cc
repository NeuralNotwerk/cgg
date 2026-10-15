#include "fmt.h"
namespace fmt {
char* to_buffer(char* first, char* last, int value) { return first; }
char* to_buffer(char* first, char* last, int value, int fmt) { return first; }
char* to_buffer(char* first, char* last, int value, int fmt, int precision) { return last; }
char* use(char* b, char* e) { return to_buffer(b, e, 1, 2, 3); }
}
