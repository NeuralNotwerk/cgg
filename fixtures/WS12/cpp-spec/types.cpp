#include "typeof.h"
namespace dap {
const TypeInfo* TypeOf<A>::type() { static TypeInfo ti; return &ti; }
const TypeInfo* TypeOf<B>::type() { static TypeInfo ti; return &ti; }
const TypeInfo* TypeOf<C>::type() { static TypeInfo ti; return &ti; }
const TypeInfo* TypeOf<D>::type() { static TypeInfo ti; return &ti; }
const TypeInfo* TypeOf<E>::type() { static TypeInfo ti; return &ti; }
const TypeInfo* TypeOf<F>::type() { static TypeInfo ti; return &ti; }
const TypeInfo* TypeOf<G>::type() { static TypeInfo ti; return &ti; }
}
