@interface Repo
- (int)zlist:(int)d;
@end
@implementation Repo
- (int)zlist:(int)d { return d; }
- (int)caller:(int)data { return zlist(data); }
@end
int top(int data) { return zlist(data); }
