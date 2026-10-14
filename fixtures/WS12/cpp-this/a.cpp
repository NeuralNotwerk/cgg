#include "a.h"
namespace KW {
void Args::Add(const char* a, int t) { (void)a; (void)t; }
#define ADDM(ctype) void Args::Add(const char* a, ctype* v) { this->Add(a, 1); }
ADDM(bool)
void Other(Args* x) { x->Add("a", 1); }
}
