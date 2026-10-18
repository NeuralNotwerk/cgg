struct tool { int x; };
struct tool *make_tool(void) { return 0; }
int use(void) {
    struct tool *t = make_tool();
    return t->x;
}
