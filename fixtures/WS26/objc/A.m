#import "A.h"
static int compare(const void *a, const void *b) { return 0; }
static void unusedFn(void) {}
@implementation A
- (void)tap:(id)sender { }
- (void)orphan { }
- (void)go {
  [button addTarget:self action:@selector(tap:) forControlEvents:1];
  SEL s = @selector(handle:with:);
  SEL t = NSSelectorFromString(@"viaString:");
  sort_with(x, n, 4, compare);
  report(orphan);
}
@end
