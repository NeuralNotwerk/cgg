char *cpp_ptr_ret(void) { return 0; }
static void *static_cpp_ptr_ret(void *a) { return a; }
int **cpp_dbl_ptr(void) { return 0; }
int plain2(void) { return 0; }
int main2(void) { cpp_ptr_ret(); static_cpp_ptr_ret(0); cpp_dbl_ptr(); return plain2(); }
