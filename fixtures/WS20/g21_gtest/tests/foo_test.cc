#include <gtest/gtest.h>

static int helper() { return 1; }

TEST(FooSuite, Adds) {
  EXPECT_EQ(helper(), 1);
}

class FooFixture : public ::testing::Test {
 protected:
  void SetUp() override {}
  void TearDown() override {}
  int unused_fixture_member() { return 2; }
};

TEST_F(FooFixture, Works) {
  EXPECT_TRUE(true);
}

class ParamFix : public testing::TestWithParam<int> {};

TEST_P(ParamFix, Param) {
  EXPECT_TRUE(GetParam() > 0);
}

// Near miss: a lowercase `test` (SFINAE probe) in a gtest-including file.
template <typename T>
struct is_probe {
  static int test(...) { return 0; }
};
