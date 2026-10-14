#ifndef DEFS_H
#define DEFS_H
#define DEFINE_PAIR(n, T) \
  static T n##_get(void) { return n##_value; } \
  static void n##_set(T v) { n##_value = v; notify(#n); }
#define DEFINE_LOCK(n) static struct lock n = { 0, { 1, 2 } }
#define CALLBACK(name, fn) int name(int a) { return fn(a) + helper(a); }
#endif
