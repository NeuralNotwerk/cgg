#include "strings.h"
std::string feature_name(std::string const& f) {
  return strcat_all("PREFIX_", f);
}
