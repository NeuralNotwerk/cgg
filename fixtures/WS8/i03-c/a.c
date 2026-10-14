char *make_name(void) { return 0; }
static const char **list_names(int n) { return 0; }
int use(void) {
  char *p = make_name();
  list_names(2);
  return p != 0;
}
