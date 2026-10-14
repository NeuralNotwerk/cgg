#include <fstream>
#include <string>
std::string read_all(const std::string& path) {
  std::ifstream info(path);
  std::string line(10, 'x');
  helper(line);
  return line;
}
void helper(const std::string& s) {}
