#include <vector>
class Q : public std::vector<int> { public: Q() : std::vector<int>(3), n_(0) { } int n_; };
class R { public: R() {} void n_() {} };
