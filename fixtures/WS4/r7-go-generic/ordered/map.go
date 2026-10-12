package ordered

type Map[K comparable, V any] struct{}

func NewMap[K comparable, V any]() *Map[K, V] { return &Map[K, V]{} }

func (m *Map[K, V]) ReplaceKey(a, b K) bool { return true }
