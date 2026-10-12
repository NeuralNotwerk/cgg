char *ptr_ret(void) { return 0; }
static void *static_ptr_ret(void *a) { return a; }
int **dbl_ptr(void) { return 0; }
int plain(void) { return 0; }
int main(void) { ptr_ret(); static_ptr_ret(0); dbl_ptr(); return plain(); }
