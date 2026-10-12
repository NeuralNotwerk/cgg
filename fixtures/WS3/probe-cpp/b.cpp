#include <vector>
struct Widget { void zork() {} };
void vec2(std::vector<Widget> ws) { ws.zork(); }
void vec3(std::vector<Widget> ws, int n) { ws.zork(); }
