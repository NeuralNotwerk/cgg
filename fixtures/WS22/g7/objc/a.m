@protocol Shape
- (double)area;
@end
@interface Sq : NSObject <Shape>
@end
@implementation Sq
- (double)area { return 1; }
@end
@interface Ci : NSObject <Shape>
@end
@implementation Ci
- (double)area { return 3; }
@end
void run(id x) { [x area]; }
