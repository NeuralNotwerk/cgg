@interface Tool
- (void)work;
@end
@implementation Tool
- (void)work {}
@end
@interface Other
- (void)work;
@end
@implementation Other
- (void)work {}
@end
@interface F
+ (Tool *)make;
@end
@implementation F
+ (Tool *)make { return nil; }
- (void)use {
    Tool *t = [F make];
    [t work];
    [[F make] work];
}
@end
