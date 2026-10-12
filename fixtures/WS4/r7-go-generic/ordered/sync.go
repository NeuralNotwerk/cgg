package ordered

type SyncMap[K comparable, V any] struct{}

func (m *SyncMap[K, V]) ReplaceKey(a, b K) bool { return false }
