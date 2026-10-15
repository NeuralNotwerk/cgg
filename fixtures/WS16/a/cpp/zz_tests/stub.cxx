#include <string>
template <typename... Args>
std::string strcat_all(Args&&...) { return ""; }
void use_stub() { strcat_all("a", "b"); }
