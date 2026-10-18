#import <Foundation/Foundation.h>
static int square(int x) { return x * x; }
static char *label(void) { return "x"; }
static int unused_helper(int x) { return x; }
@implementation Calc
- (int)run:(int)v {
  NSLog(@"%d", square(v));
  dispatch_async(dispatch_get_main_queue(), ^{ [self done]; });
  label();
  self.handler(v);
  return square(2);
}
- (void)done { }
@end
int main(int argc, char **argv) { return square(argc); }
