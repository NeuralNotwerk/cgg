static int target() { return 1; }
static int live() { return 2; }
int main() {
    live();
    throw 1;
    target();
}
