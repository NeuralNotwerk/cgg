#include <map>
#include <string>
struct Handler { int at(const char* k) { return 0; } };
void lookup(std::map<std::string, Handler> handlers) { handlers.at("a"); }
