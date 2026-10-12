template <class T> struct sk_sp { T* operator->() const; void reset(); };
struct DisplayList { void Dispatch(int x) {} void reset() {} };
struct Panel { sk_sp<DisplayList> list_; void draw() { list_->Dispatch(1); list_.reset(); } };
void run(sk_sp<DisplayList> dl) { dl->Dispatch(2); dl.reset(); }
void local() { sk_sp<DisplayList> d; d->Dispatch(3); }
