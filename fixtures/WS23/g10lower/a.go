package main
type store struct{}
func (s *store) load() int { return 0 }
type cache struct{}
func (c *cache) load() int { return 1 }
