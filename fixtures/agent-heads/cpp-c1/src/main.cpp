#include <boost/algorithm/string.hpp>
#include <spdlog/spdlog.h>
#include "absl/strings/str_cat.h"
#include <vector>
#include <string>
int main() {
  std::vector<std::string> v;
  std::string s = "a,b";
  boost::split(v, s, boost::is_any_of(","));
  spdlog::info("hi");
  auto t = absl::StrCat("a", "b");
  boost::algorithm::to_lower(s);
  return 0;
}
