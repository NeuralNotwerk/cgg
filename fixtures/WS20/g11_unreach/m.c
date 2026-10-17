static int target(void) { return 1; }
static int live(void) { return 2; }
static int labelled(void) { return 3; }

int main(void) {
    live();
    goto done;
    target();
done:
    labelled();
    return 0;
}
