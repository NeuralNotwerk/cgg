#include "typeof.h"
namespace dap {
template <typename T> const TypeInfo* get() { return TypeOf<T>::type(); }
const TypeInfo* one() { return TypeOf<A>::type(); }
}
