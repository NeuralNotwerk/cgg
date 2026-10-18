#import <Foundation/Foundation.h>
@interface Box : NSObject
- (instancetype)initWithX:(int)x;
@end
@implementation Box
- (instancetype)initWithX:(int)x { self = [super init]; return self; }
@end
void make(void) { Box *b = [[Box alloc] initWithX:1]; }
