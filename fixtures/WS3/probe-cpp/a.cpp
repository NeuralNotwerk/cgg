#include <map>
#include <string>
#include <vector>
#include <memory>
struct Handler { int at(const char* k) { return 0; } int size() { return 1; } void run() {} };
struct Key { int at(const char* k) { return 0; } };
void untyped_map(std::map<std::string, int> handlers) { handlers.at("a"); }
void comma_map(std::map<std::string, Handler> handlers) { handlers.at("a"); }
void qmap(QMap<Key, Handler> handlers) { handlers.at("a"); }
void vec(std::vector<Handler> hs) { hs.size(); }
void uptr(std::unique_ptr<Handler> h) { h->run(); }
void plain(Handler h) { h.run(); }
