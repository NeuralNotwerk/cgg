namespace dap {
struct TypeInfo {};
template <typename T> struct TypeOf {};
struct A {};
template <> struct TypeOf<A> { static const TypeInfo* type(); };
struct B {};
template <> struct TypeOf<B> { static const TypeInfo* type(); };
struct C {};
template <> struct TypeOf<C> { static const TypeInfo* type(); };
struct D {};
template <> struct TypeOf<D> { static const TypeInfo* type(); };
struct E {};
template <> struct TypeOf<E> { static const TypeInfo* type(); };
struct F {};
template <> struct TypeOf<F> { static const TypeInfo* type(); };
struct G {};
template <> struct TypeOf<G> { static const TypeInfo* type(); };
}
