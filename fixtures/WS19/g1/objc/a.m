@interface Service
- (void)run;
@end
@implementation Service
- (void)run {}
@end
void main2(id service) {
  [service run];
}
